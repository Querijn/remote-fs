use ignore::Walk;
use notify::Watcher;
use std::path::Path;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::{Receiver, channel};
use std::{collections::HashMap, fs::create_dir_all};

use crate::message_handler::{MessageType, compose_data_message};

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut state: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        state = state.wrapping_mul(0x00000100000001b3);
        state ^= b as u64;
    }
    state
}

pub(crate) struct FileWatcher {
    pub root: String,

    _watcher: notify::RecommendedWatcher,
    files: Vec<String>,
    file_hashes: HashMap<String, u64>,
    events: Receiver<Result<notify::Event, notify::Error>>,
    ignore_files_until: HashMap<String, u64>,
}

impl FileWatcher {
    pub fn new(root: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, events) = channel();
        let mut _watcher = notify::recommended_watcher(tx)?;

        let root = if Path::new(root).is_relative() {
            std::env::current_dir()?.join(root)
        } else {
            Path::new(root).to_path_buf()
        };

        _watcher.watch(&root, notify::RecursiveMode::Recursive)?;

        let mut fw = FileWatcher {
            root: root.to_str().unwrap().to_string(),
            _watcher,
            files: Vec::new(),
            file_hashes: HashMap::new(),
            events,
            ignore_files_until: HashMap::new(),
        };
        fw.index_files();
        Ok(fw)
    }

    pub fn handle_message(&mut self, msg: &MessageType, is_authorative: bool) {
        let root = self.root.clone();
        let make_absolute_path = |path: &str| -> String {
            format!("{}/{}", root, path)
        };

        match msg {
            MessageType::Sync { files } => {
                eprintln!("Received sync message");
                if is_authorative {
                    eprintln!("Unexpected sync message from non-authoritative source");
                    return;
                }

                for file in files {
                    let path = make_absolute_path(&file.0);

                    // create parent paths
                    if let Some(parent) = Path::new(&path).parent() {
                        if !parent.exists() {
                            create_dir_all(parent).unwrap_or_else(|_| {
                                eprintln!("Failed to create directory: {}", parent.display());
                            });
                        }
                    }

                    if let Err(e) = std::fs::write(&path, &file.1) {
                        eprintln!("Failed to write file {}: {:?}", path, e);
                    }
                    self.mark_as_modified(&file.0);
                }

                eprintln!("Sync message processed, files written to '{}'", self.root);
            }
            MessageType::CreateEvent { path, contents }
            | MessageType::ModifyEvent { path, contents } => {
                let abs_path = make_absolute_path(&path);
                if let Err(e) = std::fs::write(&abs_path, contents) {
                    eprintln!("Failed to write file {}: {:?}", path, e);
                }
                self.mark_as_modified(&path);
            }
            MessageType::DeleteEvent { path } => {
                if let Err(e) = std::fs::remove_file(&path) {
                    eprintln!("Failed to delete file {}: {:?}", path, e);
                }
                self.mark_as_modified(&path);
            }
            MessageType::MoveEvent { old_path, new_path } => {
                let abs_old_path = make_absolute_path(&old_path);
                let abs_new_path = make_absolute_path(&new_path);
                if let Err(e) = std::fs::rename(&abs_old_path, &abs_new_path) {
                    eprintln!(
                        "Failed to move file from {} to {}: {:?}",
                        old_path, new_path, e
                    );
                }
                self.mark_as_modified(&old_path);
                self.mark_as_modified(&new_path);
            }
        }
    }

    pub fn make_message_type(&self, event: &notify::Event) -> Option<MessageType> {
        let make_local_path = |path: &str| -> String {
            let path = path.strip_prefix(&self.root)
                .unwrap_or(path);
            path.strip_prefix("/")
                .unwrap_or(path)
                .to_string()
        };

        match event.kind {
            notify::EventKind::Create(_) => {
                assert!(event.paths.len() == 1, "More than one path in event");
                let path = event.paths[0].to_str();
                if path.is_none() {
                    eprintln!("Error: No path in event");
                    return None;
                }

                let path = path.unwrap();
                let contents = std::fs::read(&path).unwrap();
                let path = make_local_path(path);
                Some(MessageType::CreateEvent { path, contents })
            }

            notify::EventKind::Modify(_) => {
                assert!(event.paths.len() == 1, "More than one path in event");
                let path = event.paths[0].to_str();
                if path.is_none() {
                    eprintln!("Error: No path in event");
                    return None;
                }

                let path = path.unwrap();
                let contents = std::fs::read(&path).unwrap();
                let path = make_local_path(path);
                Some(MessageType::ModifyEvent { path, contents })
            }

            notify::EventKind::Remove(_) => {
                assert!(event.paths.len() == 1, "More than one path in event");
                let path = event.paths[0].to_str();
                if path.is_none() {
                    eprintln!("Error: No path in event");
                    return None;
                }

                let path = make_local_path(path.unwrap());
                Some(MessageType::DeleteEvent { path })
            }

            _ => None,
        }
    }

    pub fn make_msg_data(&self, event: &notify::Event) -> Option<Vec<u8>> {
        let msg = self.make_message_type(event);
        if msg.is_none() {
            return None;
        }

        Some(compose_data_message(&msg.unwrap()))
    }

    pub fn try_get_event(&mut self) -> Result<notify::Event, RecvTimeoutError> {
        let mut i: i64 = 3;
        loop {
            let receive_result = self.events.recv_timeout(std::time::Duration::from_millis(
                (i * 10).clamp(1, 30).try_into().unwrap(),
            ));
            if receive_result.is_err() {
                return Err(RecvTimeoutError::Timeout);
            }

            // eprintln!("Received event {:?}", receive_result);

            let event = receive_result.unwrap();
            let start = std::time::Instant::now();
            match event {
                Ok(event) => {
                    let mut allow_change = match event.kind {
                        notify::EventKind::Create(_) => {
                            self.index_files(); // Re-index files
                            self.has_file(&event)
                        }

                        notify::EventKind::Modify(e) => match e {
                            notify::event::ModifyKind::Any
                            | notify::event::ModifyKind::Metadata(_)
                            | notify::event::ModifyKind::Other => {
                                // eprintln!("Rejecting new event, ModifyKind is undesired ({:?})", e);
                                false
                            }
                            notify::event::ModifyKind::Data(_)
                            | notify::event::ModifyKind::Name(_) => {
                                let has_changes = self.has_changes_read(event.paths[0].to_str().unwrap());
                                if has_changes {
                                    // self.index_files(); // Re-index files
                                    self.has_file(&event)
                                } else {
                                    false
                                }
                            }
                        },

                        notify::EventKind::Remove(_) => self.has_file(&event),

                        notify::EventKind::Any
                        | notify::EventKind::Access(_)
                        | notify::EventKind::Other => false,
                    };

                    if allow_change {
                        let path = event.paths[0].to_str().unwrap();
                        if let Some(ignored_until) = self.ignore_files_until.get(path) {
                            if *ignored_until > FileWatcher::get_now() {
                                eprintln!("{} is ignored for another {}ms", path, ignored_until - FileWatcher::get_now());
                                allow_change = false;
                            } else {
                                eprintln!("{} has not been ignored for {}ms now.", path, FileWatcher::get_now() - ignored_until);
                                self.ignore_files_until.remove(path);
                            }
                        }
                    }

                    if !allow_change {
                        i -= 1;
                        continue;
                    }

                    let elapsed = start.elapsed();
                    let event_str = match event.kind {
                        notify::EventKind::Create(e) => format!("created ({:?})", e),
                        notify::EventKind::Modify(e) => format!("modified ({:?})", e),
                        notify::EventKind::Remove(e) => format!("removed ({:?})", e),
                        _ => "Unknown".to_string(),
                    };
                    eprintln!(
                        "Filesystem {} file '{:?}', took {}ms",
                        event_str,
                        event.paths.get(0).unwrap(),
                        elapsed.as_millis()
                    );
                    return Ok(event);
                }
                Err(err) => {
                    eprintln!("Error: {}", err);
                }
            }

            i -= 1;
        }
    }

    fn has_file(&self, event: &notify::Event) -> bool {
        if let Some(path) = event.paths.get(0) {
            let path_str = path.to_str().unwrap();
            // let path_str = path_str.strip_prefix(&self.root).unwrap_or(path_str);
            // let path_str = path_str.strip_prefix("/").unwrap_or(path_str);
            self.files.iter().any(|f| f == path_str)
        } else {
            eprintln!("Error: No path in event"); // ??
            false
        }
    }

    pub fn get_relative_files(&self) -> HashMap<String, Vec<u8>> {
        self.files
            .iter()
            .map(|file| {
                let path = Path::new(&file);
                let relative_path = path.strip_prefix(&self.root).unwrap_or(path);
                let relative_path = relative_path.strip_prefix("/").unwrap_or(relative_path);
                (
                    relative_path.to_string_lossy().to_string(),
                    std::fs::read(file).expect("Could not read file"),
                )
            })
            .collect::<HashMap<String, Vec<u8>>>()
    }

    fn index_files(&mut self) {
        let root = self.root.clone();
        self.files = Walk::new(root)
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().unwrap().is_file())
            .map(|entry| entry.path().to_string_lossy().to_string())
            .collect::<Vec<String>>();

        self.file_hashes = HashMap::new();
        for file in &self.files {
            let path = Path::new(file);
            if path.exists() {
                let contents = std::fs::read(file).unwrap();
                self.file_hashes.insert(file.to_string(), fnv1a64(&contents));
            } else {
                eprintln!("File does not exist: {}", file);
            }
        }
    }

    fn mark_as_modified(&mut self, path: &str) {
        self.ignore_files_until
            .insert(path.to_string(), FileWatcher::get_now() + 500);
        eprintln!("Ignoring file for half a second: {}", path);
    }

    fn has_changes(&self, path: &str, contents: &Vec<u8>) -> bool {
        let new_hash = fnv1a64(&contents);
        let old_hash = self.file_hashes.get(&path.to_string());
        let old_hash = *old_hash.unwrap_or(&0);
        old_hash != new_hash
    }

    fn has_changes_read(&self, path: &str) -> bool {
        let contents = std::fs::read(path).unwrap();
        self.has_changes(path, &contents)
    }

    fn get_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
}
