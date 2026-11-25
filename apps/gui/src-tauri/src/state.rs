use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use odysync_backends::ProbedBackend;
use odysync_backends::scan::ScanCancel;
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
    /// Set while updates are being installed.
    applying: AtomicBool,
    /// Asks the running install to stop after the current package.
    cancel_apply: Arc<AtomicBool>,
    /// The scan in progress, so that it can be stopped.
    scan: Mutex<Option<ScanCancel>>,
    /// Counts installs that changed the machine. A scan that started before
    /// one of them describes a state that is gone.
    generation: std::sync::atomic::AtomicU64,
    /// The next scan was started by the timer, not by a person.
    background_scan: AtomicBool,
}

/// What the app answers when something must wait for an install to end. The
/// front-end recognises it by this exact text.
pub const APPLY_RUNNING: &str = "apply-running";

/// Held while updates are being installed; releases the claim when dropped.
pub struct ApplyClaim<'a>(&'a AppState);

impl Drop for ApplyClaim<'_> {
    fn drop(&mut self) {
        self.0.applying.store(false, Ordering::Release);
    }
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
        Self::with_config(
            Config::default_path().unwrap_or_else(|_| std::path::PathBuf::from("config.json")),
        )
    }

    /// The state for the config file at `path`.
    pub fn with_config(path: std::path::PathBuf) -> Self {
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

        odysync_backends::direct::http::set_proxy(config.proxy_url.as_deref());

        Self {
            config: Mutex::new(config),
            config_path: path,
            config_load_error,
            scan_cache: Mutex::new(ScanCache::new()),
            backends: tokio::sync::Mutex::new(None),
            applying: AtomicBool::new(false),
            cancel_apply: Arc::new(AtomicBool::new(false)),
            scan: Mutex::new(None),
            generation: std::sync::atomic::AtomicU64::new(0),
            background_scan: AtomicBool::new(false),
        }
    }

    /// Whether updates are being installed right now.
    pub fn is_applying(&self) -> bool {
        self.applying.load(Ordering::Acquire)
    }

    /// Claim the right to install, which only one run has at a time.
    pub fn claim_apply(&self) -> Result<ApplyClaim<'_>, String> {
        if self.applying.swap(true, Ordering::AcqRel) {
            return Err(APPLY_RUNNING.to_string());
        }
        self.cancel_apply.store(false, Ordering::Relaxed);
        Ok(ApplyClaim(self))
    }

    /// The flag the running install watches.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_apply)
    }

    pub fn cancel_apply(&self) {
        self.cancel_apply.store(true, Ordering::Relaxed);
    }

    /// Register a scan that is about to start.
    pub fn begin_scan(&self) -> (ScanCancel, u64) {
        let cancel = ScanCancel::new();
        *lock(&self.scan) = Some(cancel.clone());
        (cancel, self.generation.load(Ordering::Acquire))
    }

    pub fn cancel_scan(&self) {
        if let Some(scan) = lock(&self.scan).as_ref() {
            scan.cancel();
        }
    }

    /// Adopt what a scan found, unless something was installed meanwhile.
    pub fn adopt_scan(&self, cache: ScanCache, generation: u64) {
        let mut slot = lock(&self.scan_cache);
        if self.generation.load(Ordering::Acquire) == generation {
            *slot = cache;
        }
    }

    /// Something was installed: what scans found so far no longer holds.
    pub fn forget_scans(&self) {
        let mut slot = lock(&self.scan_cache);
        self.generation.fetch_add(1, Ordering::AcqRel);
        slot.clear();
    }

    pub fn mark_background_scan(&self) {
        self.background_scan.store(true, Ordering::Relaxed);
    }

    /// Whether the scan that just ended was the timer's; asking resets it.
    pub fn take_background_scan(&self) -> bool {
        self.background_scan.swap(false, Ordering::Relaxed)
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
        odysync_backends::direct::http::set_proxy(config.proxy_url.as_deref());
        self.set_config(config);
        Ok(())
    }

    pub fn scan_cache(&self) -> ScanCache {
        lock(&self.scan_cache).clone()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Never the real config file: an unreadable one would be moved aside.
    fn state() -> AppState {
        AppState::with_config(std::env::temp_dir().join("odysync-no-such-config.json"))
    }

    #[test]
    fn only_one_install_at_a_time() {
        let state = state();
        let first = state.claim_apply().expect("free");
        assert!(state.is_applying());
        assert_eq!(state.claim_apply().err().as_deref(), Some(APPLY_RUNNING));
        drop(first);
        assert!(!state.is_applying());
        assert!(state.claim_apply().is_ok());
    }

    #[test]
    fn a_new_install_starts_uncancelled() {
        let state = state();
        state.cancel_apply();
        let _claim = state.claim_apply().expect("free");
        assert!(!state.cancel_flag().load(Ordering::Relaxed));
        state.cancel_apply();
        assert!(state.cancel_flag().load(Ordering::Relaxed));
    }

    /// A scan that was already running when something was installed would
    /// otherwise bring back the list from before.
    #[test]
    fn a_scan_from_before_an_install_is_dropped() {
        let state = state();
        let found = || ScanCache::from([("direct".to_string(), Vec::new())]);
        let (_, generation) = state.begin_scan();
        state.forget_scans();
        state.adopt_scan(found(), generation);
        assert!(state.scan_cache().is_empty());
        let (_, generation) = state.begin_scan();
        state.adopt_scan(found(), generation);
        assert_eq!(state.scan_cache().len(), 1);
    }
}
