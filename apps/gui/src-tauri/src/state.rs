use odysync_core::config::Config;
use std::sync::Mutex;

pub struct AppState {
    pub config: Mutex<Config>,
    pub config_path: std::path::PathBuf,
}

impl AppState {
    pub fn new() -> Self {
        let path = Config::default_path().unwrap_or_else(|_| {
            std::path::PathBuf::from("config.json")
        });
        let config = Config::load(&path).unwrap_or_default();
        Self {
            config: Mutex::new(config),
            config_path: path,
        }
    }
}
