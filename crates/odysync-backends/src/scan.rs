//! Asking every source at once, with a time limit each and a way to stop.
//!
//! The command line, the daemon and the app each had their own copy of this
//! loop, and all three shared its weaknesses: the slowest source decided how
//! long anyone waited, nothing was shown until all of them had answered, a
//! source that hung was waited for, and a scan could not be stopped.

use std::time::{Duration, Instant};

use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, UpdateCandidate};
use tokio::sync::watch;

/// Windows Update may need minutes for its first search after a start. Cutting
/// it off at the general limit would hide security updates exactly on the
/// machines that were off for a while, so it has a floor of its own.
const WINDOWS_UPDATE_MIN: Duration = Duration::from_secs(5 * 60);

/// What one source answered.
pub struct SourceScan {
    pub kind: BackendKind,
    pub name: String,
    pub elapsed: Duration,
    pub result: Result<Vec<UpdateCandidate>>,
}

/// Where a source is right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceState {
    Started,
    Done,
    Failed,
}

/// One step of a scan, for a progress display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanProgress {
    pub backend: String,
    pub name: String,
    pub state: SourceState,
    /// Updates this source reported.
    pub found: usize,
    /// Sources finished so far, this one included once it is.
    pub done: usize,
    pub total: usize,
}

/// Stops a scan from outside it. Cheap to clone.
#[derive(Clone)]
pub struct ScanCancel(watch::Sender<bool>);

impl ScanCancel {
    pub fn new() -> Self {
        Self(watch::Sender::new(false))
    }

    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    /// Resolves once [`cancel`](Self::cancel) was called.
    async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        // Fails only when every sender is gone, and `self` is one.
        let _ = rx.wait_for(|c| *c).await;
    }
}

impl Default for ScanCancel {
    fn default() -> Self {
        Self::new()
    }
}

/// How long `kind` may take when the general limit is `general`.
pub fn time_limit(kind: BackendKind, general: Duration) -> Duration {
    match kind {
        BackendKind::WindowsUpdate
        | BackendKind::WindowsDefenderUpdate
        | BackendKind::WindowsFeatureUpdate
        | BackendKind::WindowsDrivers => general.max(WINDOWS_UPDATE_MIN),
        _ => general,
    }
}

/// Ask every source, all at the same time.
///
/// A source that fails, takes longer than its limit or is still busy when the
/// scan is cancelled contributes an error instead of holding up the others.
/// Dropping its future stops the program it was running.
pub async fn scan_sources(
    backends: &[&dyn Backend],
    limit: Duration,
    cancel: &ScanCancel,
    progress: &(dyn Fn(ScanProgress) + Sync),
) -> Vec<SourceScan> {
    let total = backends.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let report = |b: &dyn Backend, state, found, done| {
        progress(ScanProgress {
            backend: b.kind().id().to_string(),
            name: b.display_name().to_string(),
            state,
            found,
            done,
            total,
        });
    };

    futures::future::join_all(backends.iter().map(|b| {
        let (done, report) = (&done, &report);
        async move {
            report(*b, SourceState::Started, 0, done.load(Relaxed));
            let started = Instant::now();
            let limit = time_limit(b.kind(), limit);
            let result = tokio::select! {
                result = tokio::time::timeout(limit, b.scan()) => match result {
                    Ok(result) => result,
                    Err(_) => Err(Error::CommandTimeout {
                        command: b.display_name().to_string(),
                        seconds: limit.as_secs(),
                    }),
                },
                () = cancel.cancelled() => Err(Error::Cancelled),
            };
            let elapsed = started.elapsed();
            let finished = done.fetch_add(1, Relaxed) + 1;
            match &result {
                Ok(found) => {
                    tracing::info!(
                        backend = %b.kind(),
                        count = found.len(),
                        ms = elapsed.as_millis() as u64,
                        "scan complete"
                    );
                    report(*b, SourceState::Done, found.len(), finished);
                }
                Err(e) => {
                    tracing::warn!(
                        backend = %b.kind(),
                        error = %e,
                        ms = elapsed.as_millis() as u64,
                        "scan failed"
                    );
                    report(*b, SourceState::Failed, 0, finished);
                }
            }
            SourceScan {
                kind: b.kind(),
                name: b.display_name().to_string(),
                elapsed,
                result,
            }
        }
    }))
    .await
}

use std::sync::atomic::Ordering::Relaxed;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct Slow(BackendKind, Duration);

    #[async_trait]
    impl Backend for Slow {
        fn kind(&self) -> BackendKind {
            self.0
        }
        fn display_name(&self) -> &str {
            self.0.id()
        }
        async fn is_available(&self) -> bool {
            true
        }
        async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
            tokio::time::sleep(self.1).await;
            Ok(Vec::new())
        }
        async fn apply(&self, _: &UpdateCandidate) -> Result<()> {
            unreachable!("a scan never applies")
        }
        async fn installed_version(&self, _: &UpdateCandidate) -> Result<Option<String>> {
            Ok(None)
        }
    }

    fn refs(all: &[Slow]) -> Vec<&dyn Backend> {
        all.iter().map(|b| b as &dyn Backend).collect()
    }

    const MS: Duration = Duration::from_millis(1);

    #[tokio::test]
    async fn one_hanging_source_does_not_hold_up_the_rest() {
        let all = [
            Slow(BackendKind::Pip, 10 * MS),
            Slow(BackendKind::Npm, 60_000 * MS),
        ];
        let started = Instant::now();
        let out = scan_sources(&refs(&all), 200 * MS, &ScanCancel::new(), &|_| {}).await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(out[0].result.is_ok());
        assert!(matches!(out[1].result, Err(Error::CommandTimeout { .. })));
    }

    #[tokio::test]
    async fn cancelling_keeps_what_was_already_answered() {
        let all = [
            Slow(BackendKind::Pip, 10 * MS),
            Slow(BackendKind::Npm, 60_000 * MS),
        ];
        let cancel = ScanCancel::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(200 * MS).await;
            stop.cancel();
        });
        let started = Instant::now();
        let out = scan_sources(&refs(&all), Duration::from_secs(60), &cancel, &|_| {}).await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(out[0].result.is_ok());
        assert!(matches!(out[1].result, Err(Error::Cancelled)));
    }

    #[tokio::test]
    async fn cancelling_before_the_start_is_not_lost() {
        let cancel = ScanCancel::new();
        cancel.cancel();
        let all = [Slow(BackendKind::Npm, 60_000 * MS)];
        let out = scan_sources(&refs(&all), Duration::from_secs(60), &cancel, &|_| {}).await;
        assert!(out[0].result.is_err());
    }

    #[tokio::test]
    async fn progress_names_every_source_and_counts_up_to_the_total() {
        let all = [
            Slow(BackendKind::Pip, 5 * MS),
            Slow(BackendKind::Npm, 30 * MS),
            Slow(BackendKind::Cargo, 60_000 * MS),
        ];
        let seen = Mutex::new(Vec::new());
        scan_sources(&refs(&all), 300 * MS, &ScanCancel::new(), &|p| {
            seen.lock().unwrap().push(p)
        })
        .await;
        let seen = seen.into_inner().unwrap();
        let count = |s| seen.iter().filter(|p| p.state == s).count();
        assert_eq!(count(SourceState::Started), 3);
        assert_eq!(count(SourceState::Done), 2);
        assert_eq!(count(SourceState::Failed), 1);
        assert_eq!(seen.last().unwrap().done, 3);
        assert!(seen.iter().all(|p| p.total == 3));
    }

    #[test]
    fn windows_update_gets_the_time_it_needs() {
        let general = Duration::from_secs(120);
        assert_eq!(time_limit(BackendKind::Pip, general), general);
        assert_eq!(
            time_limit(BackendKind::WindowsUpdate, general),
            WINDOWS_UPDATE_MIN
        );
        let long = Duration::from_secs(3600);
        assert_eq!(time_limit(BackendKind::WindowsUpdate, long), long);
    }
}
