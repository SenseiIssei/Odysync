//! Applies a plan and confirms each update actually landed.
//!
//! The runner is deliberately paranoid about one thing: a package manager
//! reporting exit code 0 does not prove the package changed. winget in
//! particular returns success for no-ops and for partially applied installs.
//! So every apply is followed by reading the installed version back, and an
//! update that did not converge is reported as a failure.

use std::collections::HashMap;

use crate::backend::Backend;
use crate::model::{ApplyOutcome, BackendKind, PlannedUpdate};
use crate::report::RunReport;
use crate::restore::RestorePointGuard;
use crate::version::Version;

/// Applies planned updates using the supplied backends.
pub struct Runner<'a> {
    backends: HashMap<BackendKind, &'a dyn Backend>,
    dry_run: bool,
}

impl<'a> Runner<'a> {
    pub fn new(backends: impl IntoIterator<Item = &'a dyn Backend>, dry_run: bool) -> Self {
        Self {
            backends: backends.into_iter().map(|b| (b.kind(), b)).collect(),
            dry_run,
        }
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
        &self,
        plan: &[PlannedUpdate],
        report: &mut RunReport,
        restore_point: bool,
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

            let outcome = match backend.apply(candidate).await {
                Ok(()) => self.confirm(*backend, planned).await,
                Err(e) => ApplyOutcome::Failed {
                    detail: e.to_string(),
                },
            };

            if !outcome.is_success() {
                tracing::warn!(package = %candidate.id, ?outcome, "update did not succeed");
            }

            report.push(candidate.id.clone(), candidate.name.clone(), outcome);
        }

        report.finish();
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
                }
            }
            Err(e) => {
                return ApplyOutcome::DidNotConverge {
                    expected: expected.raw().to_string(),
                    actual: format!("could not read back: {e}"),
                }
            }
        };

        let actual = Version::parse(&actual_raw);

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
