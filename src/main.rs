mod file_watcher;
mod server;
mod client;
mod message_handler;
mod config;

use file_watcher::FileWatcher;
use config::Config;

static PORT: u16 = 5343;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let config = match args[1].as_str() {
        "server" => {
            if args.len() != 3 {
                eprintln!("Usage: {} server <path>", args[0]);
                std::process::exit(1);
            }

            let path = &args[2];
            Config::new_server(PORT, path.to_string())
        },

        "client" => {
            if args.len() != 4 {
                eprintln!("Usage: {} client <host> <path>", args[0]);
                std::process::exit(1);
            }

            let host = &args[2];
            let path = &args[3];
            Config::new_client(host.to_string(), PORT, path.to_string())
        },

        file => {
            let exists = std::fs::exists(file);
            if let Ok(_) = exists {
                match Config::from_file(file) {
                    Ok(config) => config,
                    Err(e) => {
                        eprintln!("Failed to load config file: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            else if let Err(e) = exists {
                eprintln!("Failed to check if config file '{}' exists: {}", file, e);
                std::process::exit(1);
            }
            else {
                eprintln!("Config file '{}' does not exist", file);
                std::process::exit(1);
            }
        }
    };

    start_from_config(&config).await;
}

async fn start_from_config(config: &Config) {
    match config {
        Config::Server(server_config) => {
            let path = &server_config.location;
            if !std::path::Path::new(path).exists() {
                eprintln!("Path '{}' does not exist", path);
                std::process::exit(1);
            }

            server::run(server_config.port, path).await;
        }

        Config::Client(client_config) => {
            let ip = client_config.host.parse::<std::net::IpAddr>();
            if ip.is_err() {
                eprintln!("Invalid IP address: '{}'", client_config.host);
                std::process::exit(1);
            }

            let ip = ip.unwrap();
            if ip.is_unspecified() {
                eprintln!("Unspecified IP address {} is not allowed", ip);
                std::process::exit(1);
            }

            let path = &client_config.location;
            if !std::path::Path::new(path).exists() {
                if let Err(e) = std::fs::create_dir_all(path) {
                    eprintln!("Failed to create directory '{}': {}", path, e);
                    std::process::exit(1);
                }
            }

            client::run(client_config.host.as_str(), path).await;
        }

        _ => {
            eprintln!("Unsupported config type");
            std::process::exit(1);
        }
    }
}
