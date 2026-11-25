//! Applies a plan and confirms each update actually landed.
//!
//! The runner is deliberately paranoid about one thing: a package manager
//! reporting exit code 0 does not prove the package changed. winget in
//! particular returns success for no-ops and for partially applied installs.
//! So every apply is followed by reading the installed version back, and an
//! update that did not converge is reported as a failure.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::backend::Backend;
use crate::error::Error;
use crate::history::UpdateHistory;
use crate::model::{ApplyOutcome, BackendKind, PlannedUpdate, SkipReason};
use crate::report::RunReport;
use crate::restore::RestorePointGuard;

/// Default number of retries when no explicit value is configured.
pub const DEFAULT_MAX_RETRIES: u32 = 2;

/// Applies planned updates using the supplied backends.
pub struct Runner<'a> {
    backends: HashMap<BackendKind, &'a dyn Backend>,
    dry_run: bool,
    history: Option<UpdateHistory>,
    max_retries: u32,
    cancel: Option<Arc<AtomicBool>>,
}

impl<'a> Runner<'a> {
    pub fn new(backends: impl IntoIterator<Item = &'a dyn Backend>, dry_run: bool) -> Self {
        Self {
            backends: backends.into_iter().map(|b| (b.kind(), b)).collect(),
            dry_run,
            history: None,
            max_retries: DEFAULT_MAX_RETRIES,
            cancel: None,
        }
    }

    /// Stop early once `flag` is set.
    ///
    /// The package being installed is finished: interrupting an installer is
    /// how machines break. Everything after it is reported as skipped.
    pub fn with_cancel(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }

    /// Enable persistent update history recording.
    pub fn with_history(mut self, history: UpdateHistory) -> Self {
        self.history = Some(history);
        self
    }

    /// Override how many times a retryable failure is retried.
    ///
    /// Wired to `Config::max_retries` so the setting in the UI has an effect.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Apply every actionable entry in `plan`, in order.
    ///
    /// Blocked entries are recorded with their reason and never executed.
    /// A failure on one package does not stop the rest — an updater that gives
    /// up halfway leaves the machine in a less consistent state than one that
    /// finishes and reports.
    ///
    /// When `restore_point` is `true` and this is not a dry run, a system
    /// restore point is created before the first apply (Windows only).
    pub async fn run(
        &mut self,
        plan: &[PlannedUpdate],
        report: &mut RunReport,
        restore_point: bool,
    ) {
        self.run_with_progress(plan, report, restore_point, None::<&dyn ProgressEmitter>)
            .await;
    }

    /// Like [`run`](Self::run) but emits progress events via `emitter`.
    ///
    /// `emitter` is an optional trait object that the Tauri layer implements
    /// to forward progress to the frontend. When `None`, behaves identically
    /// to `run`.
    pub async fn run_with_progress(
        &mut self,
        plan: &[PlannedUpdate],
        report: &mut RunReport,
        restore_point: bool,
        emitter: Option<&dyn ProgressEmitter>,
    ) {
        let has_actionable = plan.iter().any(|p| p.is_actionable());

        if restore_point && !self.dry_run && has_actionable {
            match RestorePointGuard::new("Odysync").await {
                Ok(guard) if guard.created() => {
                    tracing::info!("system restore point created before apply batch");
                }
                Ok(_) => {
                    tracing::info!("restore point not created (disabled or not elevated)");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "restore point creation error");
                }
            }
        }

        let total = plan.iter().filter(|p| p.is_actionable()).count();
        let mut current = 0usize;

        let emit =
            |emitter: Option<&dyn ProgressEmitter>, package: &str, done: usize, phase: &str| {
                if let Some(e) = emitter {
                    let percent = if total == 0 {
                        Some(100)
                    } else {
                        Some(((done as f64 / total as f64) * 100.0).round() as u8)
                    };
                    e.emit_progress(ProgressEvent {
                        package: package.to_string(),
                        current: done,
                        total,
                        phase: phase.to_string(),
                        percent,
                    });
                }
            };

        for planned in plan {
            let candidate = &planned.candidate;

            if let Some(reason) = &planned.blocked_by {
                report.push(
                    candidate.id.clone(),
                    candidate.name.clone(),
                    ApplyOutcome::Skipped {
                        reason: reason.clone(),
                    },
                );
                continue;
            }

            if self.cancelled() {
                report.push(
                    candidate.id.clone(),
                    candidate.name.clone(),
                    ApplyOutcome::Skipped {
                        reason: SkipReason::Cancelled,
                    },
                );
                continue;
            }

            let Some(backend) = self.backends.get(&candidate.id.backend) else {
                report.push(
                    candidate.id.clone(),
                    candidate.name.clone(),
                    ApplyOutcome::Failed {
                        detail: format!("no backend registered for {}", candidate.id.backend),
                    },
                );
                continue;
            };

            if self.dry_run {
                report.push(
                    candidate.id.clone(),
                    candidate.name.clone(),
                    ApplyOutcome::Updated {
                        from: candidate.installed.raw().to_string(),
                        to: candidate.available.raw().to_string(),
                    },
                );
                continue;
            }

            tracing::info!(
                package = %candidate.id,
                from = %candidate.installed,
                to = %candidate.available,
                "applying update"
            );

            // `current` = items finished so far, so the bar reflects real
            // progress. The previous code emitted `current + 1` before doing
            // the work, so the last item showed 100% while it was still
            // installing and no "done" event ever followed.
            emit(emitter, &candidate.name, current, "installing");

            let outcome = self.apply_with_retry(*backend, planned).await;

            if !outcome.is_success() {
                tracing::warn!(package = %candidate.id, ?outcome, "update did not succeed");
            }

            if let Some(history) = &mut self.history {
                history.record(
                    &candidate.id,
                    &candidate.name,
                    candidate.installed.raw(),
                    candidate.available.raw(),
                    &outcome,
                );
                // After every package, not once at the end: a run that is
                // interrupted half-way must still leave a record of the half
                // that happened.
                if let Err(e) = history.save() {
                    tracing::warn!(error = %e, "failed to save update history");
                }
            }

            report.push(candidate.id.clone(), candidate.name.clone(), outcome);
            current += 1;
            emit(emitter, &candidate.name, current, "installing");
        }

        // Ask the OS whether it now wants a restart, rather than trusting a
        // backend to say so. `reboot_required` had three display sites and no
        // assignment anywhere, so the banners could never appear; asking here
        // covers every backend at once, including an installer that quietly
        // queued a pending file rename. A dry run changed nothing, so it
        // cannot have created a new reason to restart — but the machine may
        // already have been waiting on one before we started, and hiding that
        // would be the same omission in a smaller form.
        if !self.dry_run && has_actionable {
            report.reboot_required = crate::platform::reboot_pending();
        }

        // A terminal event so the UI can settle on "done" rather than being
        // left at whatever the last per-item update happened to be.
        emit(emitter, "", current, "done");

        report.finish();
    }

    /// Apply an update, retrying what failed before anything was changed.
    ///
    /// Retries up to `self.max_retries` times with exponential backoff
    /// (1s, 2s, 4s, ...) when [`Error::is_retryable`] says so. A program that
    /// ran into its time limit is never started again; whether it got there
    /// after all is read back from the machine.
    async fn apply_with_retry(
        &self,
        backend: &dyn Backend,
        planned: &PlannedUpdate,
    ) -> ApplyOutcome {
        let max_retries = self.max_retries;
        let candidate = &planned.candidate;

        for attempt in 0..=max_retries {
            match backend.apply(candidate).await {
                Ok(()) => return self.confirm(backend, planned).await,
                Err(e @ Error::CommandTimeout { .. }) => {
                    tracing::warn!(
                        package = %candidate.id,
                        error = %e,
                        "time limit reached; checking what is installed"
                    );
                    return match self.confirm(backend, planned).await {
                        done @ ApplyOutcome::Updated { .. } => done,
                        _ => ApplyOutcome::Failed {
                            detail: format!(
                                "{}; it was stopped and not started again",
                                e.sanitize()
                            ),
                        },
                    };
                }
                Err(e) => {
                    if e.is_retryable() && attempt < max_retries {
                        // Cap the shift so a large configured retry count
                        // cannot overflow into a multi-hour sleep.
                        let delay = std::time::Duration::from_secs(1u64 << attempt.min(6));
                        tracing::warn!(
                            package = %candidate.id,
                            attempt = attempt + 1,
                            max_retries,
                            delay_secs = delay.as_secs(),
                            error = %e.sanitize(),
                            "retrying transient error"
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return ApplyOutcome::Failed {
                        detail: e.sanitize(),
                    };
                }
            }
        }

        // Unreachable: the loop always returns.
        ApplyOutcome::Failed {
            detail: "exhausted retries".into(),
        }
    }

    /// Read the installed version back and check it matches what we asked for.
    async fn confirm(&self, backend: &dyn Backend, planned: &PlannedUpdate) -> ApplyOutcome {
        let candidate = &planned.candidate;
        let expected = &candidate.available;

        let actual_raw = match backend.installed_version(candidate).await {
            Ok(Some(v)) => v,
            Ok(None) => {
                return ApplyOutcome::DidNotConverge {
                    expected: expected.raw().to_string(),
                    actual: "unreadable".to_string(),
                };
            }
            Err(e) => {
                return ApplyOutcome::DidNotConverge {
                    expected: expected.raw().to_string(),
                    actual: format!("could not read back: {e}"),
                };
            }
        };

        let actual = expected.parse_like(&actual_raw);

        // Accept "at least what we asked for": some installers normalise the
        // version string, and a package that jumped further ahead is still not
        // the failure mode we are guarding against.
        let converged = match expected.compare(&actual) {
            Some(std::cmp::Ordering::Equal) | Some(std::cmp::Ordering::Less) => true,
            Some(std::cmp::Ordering::Greater) => false,
            // Unparseable readback: fall back to an exact string match so we
            // do not fail a package whose versioning we simply cannot model.
            None => actual_raw.trim() == expected.raw().trim(),
        };

        if converged {
            ApplyOutcome::Updated {
                from: candidate.installed.raw().to_string(),
                to: actual_raw,
            }
        } else {
            ApplyOutcome::DidNotConverge {
                expected: expected.raw().to_string(),
                actual: actual_raw,
            }
        }
    }
}

/// A progress event emitted during apply, suitable for serialization to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProgressEvent {
    pub package: String,
    pub current: usize,
    pub total: usize,
    pub phase: String,
    pub percent: Option<u8>,
}

/// Trait for emitting progress events to the frontend.
///
/// The Tauri layer implements this to forward events via `app.emit()`.
pub trait ProgressEmitter: Send + Sync {
    fn emit_progress(&self, event: ProgressEvent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use crate::model::{PackageId, UpdateCandidate};
    use crate::version::Version;
    use std::sync::atomic::AtomicUsize;

    /// A backend whose `apply` fails the way a test asks for, and which counts
    /// how often it was started.
    struct Fake {
        error: fn() -> Option<Error>,
        installed: &'static str,
        applies: AtomicUsize,
        cancel: Option<Arc<AtomicBool>>,
    }

    impl Fake {
        fn new(error: fn() -> Option<Error>, installed: &'static str) -> Self {
            Self {
                error,
                installed,
                applies: AtomicUsize::new(0),
                cancel: None,
            }
        }

        fn applies(&self) -> usize {
            self.applies.load(Ordering::Relaxed)
        }
    }

    #[async_trait::async_trait]
    impl Backend for Fake {
        fn kind(&self) -> BackendKind {
            BackendKind::Direct
        }
        fn display_name(&self) -> &str {
            "fake"
        }
        async fn is_available(&self) -> bool {
            true
        }
        async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
            Ok(Vec::new())
        }
        async fn apply(&self, _: &UpdateCandidate) -> Result<()> {
            self.applies.fetch_add(1, Ordering::Relaxed);
            if let Some(flag) = &self.cancel {
                flag.store(true, Ordering::Relaxed);
            }
            match (self.error)() {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
        async fn installed_version(&self, _: &UpdateCandidate) -> Result<Option<String>> {
            Ok(Some(self.installed.to_string()))
        }
    }

    fn planned(name: &str) -> PlannedUpdate {
        PlannedUpdate {
            candidate: UpdateCandidate {
                id: PackageId::new(BackendKind::Direct, format!("user:{name}")),
                name: name.into(),
                installed: Version::parse("1.0"),
                available: Version::parse("1.1"),
                size_bytes: None,
                expected_sha256: None,
            },
            blocked_by: None,
        }
    }

    fn timed_out() -> Option<Error> {
        Some(Error::CommandTimeout {
            command: "installer".into(),
            seconds: 1,
        })
    }

    async fn run(fake: &Fake, plan: &[PlannedUpdate]) -> RunReport {
        let mut report = RunReport::new();
        Runner::new([fake as &dyn Backend], false)
            .run(plan, &mut report, false)
            .await;
        report
    }

    #[tokio::test]
    async fn an_installer_that_timed_out_is_not_started_again() {
        let fake = Fake::new(timed_out, "1.0");
        let report = run(&fake, &[planned("a")]).await;
        assert_eq!(fake.applies(), 1);
        match &report.entries[0].outcome {
            ApplyOutcome::Failed { detail } => assert!(detail.contains("not started again")),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn an_installer_that_timed_out_but_finished_is_a_success() {
        let fake = Fake::new(timed_out, "1.1");
        let report = run(&fake, &[planned("a")]).await;
        assert_eq!(fake.applies(), 1);
        assert_eq!(report.updated(), 1);
    }

    #[tokio::test]
    async fn a_failed_installer_stays_failed() {
        let fake = Fake::new(
            || {
                Some(Error::CommandFailed {
                    command: "installer".into(),
                    code: 143,
                    stderr: String::new(),
                })
            },
            "1.0",
        );
        let report = run(&fake, &[planned("a")]).await;
        assert_eq!(fake.applies(), 1);
        assert_eq!(report.failed(), 1);
    }

    #[tokio::test]
    async fn exit_code_zero_without_a_new_version_is_not_success() {
        let fake = Fake::new(|| None, "1.0");
        let report = run(&fake, &[planned("a")]).await;
        assert!(matches!(
            report.entries[0].outcome,
            ApplyOutcome::DidNotConverge { .. }
        ));
    }

    #[tokio::test]
    async fn cancelling_finishes_the_current_package_and_skips_the_rest() {
        let flag = Arc::new(AtomicBool::new(false));
        let mut fake = Fake::new(|| None, "1.1");
        fake.cancel = Some(Arc::clone(&flag));
        let mut report = RunReport::new();
        Runner::new([&fake as &dyn Backend], false)
            .with_cancel(flag)
            .run(&[planned("a"), planned("b")], &mut report, false)
            .await;
        assert_eq!(fake.applies(), 1);
        assert_eq!(report.updated(), 1);
        assert_eq!(
            report.entries[1].outcome,
            ApplyOutcome::Skipped {
                reason: SkipReason::Cancelled
            }
        );
    }

    /// A run that dies half-way must leave a record of the half that happened.
    #[tokio::test]
    async fn history_is_on_disk_after_every_package() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        struct Watcher(std::path::PathBuf, AtomicUsize);
        impl ProgressEmitter for Watcher {
            fn emit_progress(&self, event: ProgressEvent) {
                if event.phase == "installing" {
                    let n = UpdateHistory::load_from(self.0.clone()).entries().len();
                    self.1.fetch_max(n, Ordering::Relaxed);
                }
            }
        }
        let watcher = Watcher(path.clone(), AtomicUsize::new(0));
        let fake = Fake::new(|| None, "1.1");
        let mut report = RunReport::new();
        Runner::new([&fake as &dyn Backend], false)
            .with_history(UpdateHistory::load_from(path))
            .run_with_progress(
                &[planned("a"), planned("b")],
                &mut report,
                false,
                Some(&watcher),
            )
            .await;
        assert_eq!(watcher.1.load(Ordering::Relaxed), 2);
    }
}
