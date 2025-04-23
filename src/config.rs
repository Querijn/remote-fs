use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct ServerConfig {
    pub(crate) port: u16,
    pub(crate) location: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct ClientConfig {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) location: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) enum Config {
    Server(ServerConfig),
    Client(ClientConfig),
}

impl Config {
    pub(crate) fn new_server(port: u16, location: String) -> Self {
        Config::Server(ServerConfig { port, location })
    }

    pub(crate) fn new_client(host: String, port: u16, location: String) -> Self {
        Config::Client(ClientConfig { host, port, location })
    }

    pub(crate) fn from_file(file_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let file = std::fs::File::open(file_path)?;
        let config: Config = serde_json::from_reader(file)?;
        Ok(config)
    }

    pub(crate) fn to_file(&self, file_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let file = std::fs::File::create(file_path)?;
        serde_json::to_writer(file, self)?;
        Ok(())
    }

    pub(crate) fn create_server_config() {
        let config = Config::Server(ServerConfig {
            port: 8080,
            location: String::from("/path/to/server"),
        });
        config.to_file("config.json").unwrap();
    }

    pub(crate) fn create_client_config() {
        let config = Config::Client(ClientConfig {
            host: String::from("localhost"),
            port: 8080,
            location: String::from("/path/to/client"),
        });
        config.to_file("config.json").unwrap();
    }
}