use tokio::net::TcpStream;
use tokio::io::AsyncWriteExt;
use crate::message_handler::read_msg;
use std::fs::create_dir_all;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(crate) async fn run(addr: &str, root: &str) {

    // Ensure the folder exists
    if !Path::new(root).exists() {
        create_dir_all(root).unwrap_or_else(|_| {
            eprintln!("Failed to create directory: {}", root);
            std::process::exit(1);
        });
    }

    let addr = format!("{}:5343", addr);
    let stream = TcpStream::connect(&addr).await;
    if stream.is_err() {
        eprintln!("Failed to connect to {}: {:?}", addr, stream.err().unwrap());
        std::process::exit(1);
    }
    let stream = stream.unwrap();
    let file_watcher = Arc::new(Mutex::new(crate::FileWatcher::new(root).unwrap()));
    let file_watcher2 = file_watcher.clone();
    let (reader, mut writer) = stream.into_split();

    // Just keep waiting for file events, and pass those on to the other side
    tokio::spawn(async move {
        eprintln!("File watcher started watching '{}'", file_watcher.lock().unwrap().root);
        loop {
            let event = file_watcher.lock().unwrap().try_get_event();
            if event.is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                continue;
            }

            let event = event.unwrap();
            let event_data = file_watcher.lock().unwrap().make_msg_data(&event);
            if event_data.is_none() {
                // eprintln!("Failed to serialize {:?} event", event);
                continue;
            }

            let result = writer.write_all(&event_data.unwrap()).await;
            if result.is_err() {
                eprintln!("Failed to send  {:?} event", event);
            }
        }
    });

    // Server file update reader
    let mut reader = reader;
    loop {
        let msg = read_msg(&mut reader).await;
        if let Err(e) = msg {
            eprintln!("Failed to read message from {}: {:?}", addr, e);
            if e.is_disconnected() {
                break;
            }
            continue;
        }

        let msg = msg.unwrap();
        file_watcher2.lock().unwrap().handle_message(&msg, false);
    }

    eprintln!("Reader closed");
    std::process::exit(0);
}