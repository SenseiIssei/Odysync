use odysync_core::config::Config;
use odysync_core::model::UpdateCandidate;
use std::sync::Mutex;

/// Cached scan results keyed by backend kind ID string.
pub type ScanCache = std::collections::HashMap<String, Vec<UpdateCandidate>>;

pub struct AppState {
    pub config: Mutex<Config>,
    pub config_path: std::path::PathBuf,
    /// Last scan results so `apply` can match candidates without re-scanning.
    pub scan_cache: Mutex<ScanCache>,
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
            scan_cache: Mutex::new(ScanCache::new()),
        }
    }
}
