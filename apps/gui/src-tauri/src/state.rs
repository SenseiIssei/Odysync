use std::sync::{Arc, Mutex, MutexGuard};

use odysync_backends::ProbedBackend;
use odysync_core::config::Config;
use odysync_core::model::UpdateCandidate;

/// Cached scan results keyed by backend kind ID string.
pub type ScanCache = std::collections::HashMap<String, Vec<UpdateCandidate>>;

pub struct AppState {
    config: Mutex<Config>,
    pub config_path: std::path::PathBuf,
    /// Set when the config file on disk was unreadable at startup, so the UI
    /// can say so instead of silently presenting defaults.
    pub config_load_error: Option<String>,
    /// Last scan results so `apply` can match candidates without re-scanning.
    scan_cache: Mutex<ScanCache>,
    /// Availability probe results. Probing every backend costs roughly one
    /// process spawn each (~36 on Windows, several of them PowerShell/CIM), so
    /// it is done once and reused until explicitly invalidated.
    backends: tokio::sync::Mutex<Option<Arc<Vec<ProbedBackend>>>>,
}

/// Lock a mutex, recovering from poisoning.
///
/// A panic in one command must not brick every later command. The data behind
/// these locks is a plain config/cache with no invariant that a mid-update
/// panic could break, so taking the value back is safe and strictly better
/// than propagating the panic forever.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AppState {
    pub fn new() -> Self {
        let path =
            Config::default_path().unwrap_or_else(|_| std::path::PathBuf::from("config.json"));

        let (config, config_load_error) = match Config::load(&path) {
            Ok(c) => (c, None),
            Err(e) => {
                // Never overwrite a config we could not parse: move it aside so
                // the user still has their holds and exclusions to recover.
                let backup = path.with_extension("json.corrupt");
                let saved = std::fs::rename(&path, &backup).is_ok();
                tracing::error!(
                    error = %e,
                    backup = %backup.display(),
                    saved,
                    "config file could not be read; starting from defaults"
                );
                let detail = if saved {
                    format!("{e} The previous file was kept at {}.", backup.display())
                } else {
                    e.to_string()
                };
                (Config::default(), Some(detail))
            }
        };

        Self {
            config: Mutex::new(config),
            config_path: path,
            config_load_error,
            scan_cache: Mutex::new(ScanCache::new()),
            backends: tokio::sync::Mutex::new(None),
        }
    }

    /// A snapshot of the current config.
    pub fn config(&self) -> Config {
        lock(&self.config).clone()
    }

    /// Replace the in-memory config. Callers are responsible for persisting.
    pub fn set_config(&self, config: Config) {
        *lock(&self.config) = config;
    }

    /// Persist `config` and adopt it as the in-memory config.
    ///
    /// Ordering matters: if the write fails, memory keeps the previous value so
    /// what the UI shows still matches what is on disk.
    pub fn save_config(&self, config: Config) -> odysync_core::error::Result<()> {
        config.save(&self.config_path)?;
        self.set_config(config);
        Ok(())
    }

    pub fn scan_cache(&self) -> ScanCache {
        lock(&self.scan_cache).clone()
    }

    pub fn replace_scan_cache(&self, cache: ScanCache) {
        *lock(&self.scan_cache) = cache;
    }

    /// Probe results for every compiled-in backend, probing on first use.
    pub async fn probed_backends(&self) -> Arc<Vec<ProbedBackend>> {
        let mut slot = self.backends.lock().await;
        if let Some(cached) = slot.as_ref() {
            return Arc::clone(cached);
        }
        let config = self.config();
        tracing::info!("probing backends (first use)");
        let probed = Arc::new(odysync_backends::probe_backends(&config).await);
        *slot = Some(Arc::clone(&probed));
        probed
    }

    /// Drop the cached probe so the next call re-detects. Called after the set
    /// of disabled backends changes, or on an explicit user refresh.
    pub async fn invalidate_backends(&self) {
        *self.backends.lock().await = None;
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
