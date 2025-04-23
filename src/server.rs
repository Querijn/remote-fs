use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use crate::message_handler::{ read_msg, write_msg, MessageType };

pub(crate) async fn run(port: u16, root: &str) {
    let listener = TcpListener::bind(("0.0.0.0", port)).await.unwrap();
    let file_watcher = Arc::new(Mutex::new(crate::FileWatcher::new(root).unwrap()));
    eprintln!("Server listening on port {}", port);
    let clients: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Vec<u8>>>>> = Arc::new(Mutex::new(HashMap::new()));
    let writer_clients = clients.clone();
    let writer_file_watcher = file_watcher.clone();

    tokio::spawn(async move {
        eprintln!("File watcher started watching {}", writer_file_watcher.lock().unwrap().root);
        loop {
            let event = writer_file_watcher.lock().unwrap().try_get_event();
            if event.is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                continue;
            }

            let clients = writer_clients.lock().unwrap();
            eprintln!("Transmitting event to {} clients..", clients.len());

            let event_data = writer_file_watcher.lock().unwrap().make_msg_data(&event.unwrap());
            if event_data.is_none() {
                eprintln!("Failed to serialize event");
                continue;
            }

            let event_data = event_data.unwrap();
            for (addr, tx) in clients.iter() {
                if let Err(_) = tx.send(event_data.clone()) {
                    eprintln!("Failed to send event to {}", addr);
                }
            }
        }
    });

    loop {
        let (mut stream, addr) = listener.accept().await.unwrap();
        let addr = addr.to_string();
        let addr_read = addr.clone();
        eprintln!("Client connected: {}", addr);

        // Send initial sync
        let files = file_watcher.lock().unwrap().get_relative_files();
        let sync_event = MessageType::Sync { files };
        write_msg(&mut stream, &sync_event).await.unwrap();

        let (reader, mut writer) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();

        clients.lock().unwrap().insert(addr.clone(), tx);

        // Writer task
        let write_clients = clients.clone();
        tokio::spawn(async move {
            eprintln!("Client writer waiting for commands: {}", addr);
            while let Some(data) = rx.recv().await {
                eprintln!("Client writer received data, {} bytes", data.len());
                if writer.write_all(&data).await.is_err() {
                    eprintln!("Failed to write to client {}", addr);
                    break;
                }
            }
            eprintln!("Client writer closed: {}", addr);
            write_clients.lock().unwrap().remove(&addr);
        });

        // Reader task
        let file_watcher_reader = file_watcher.clone();
        tokio::spawn(async move {
            let mut reader = reader;
            loop {
                let msg = read_msg(&mut reader).await;
                if let Err(e) = msg {
                    eprintln!("Failed to read message from {}: {:?}", addr_read, e);
                    if e.is_disconnected() {
                        break;
                    }
                    continue;
                }

                let msg = msg.unwrap();
                file_watcher_reader.lock().unwrap().handle_message(&msg, true);
            }

            eprintln!("Client reader closed: {}", addr_read);
        });
    }
}