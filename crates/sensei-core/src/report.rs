//! Machine- and human-readable run reports.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{ApplyOutcome, PackageId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub started: DateTime<Utc>,
    pub finished: Option<DateTime<Utc>>,
    pub entries: Vec<ReportEntry>,
    /// Set when a backend signalled that a reboot is needed to finish.
    pub reboot_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportEntry {
    pub package: PackageId,
    pub name: String,
    pub outcome: ApplyOutcome,
}

impl Default for RunReport {
    fn default() -> Self {
        Self::new()
    }
}

impl RunReport {
    pub fn new() -> Self {
        Self {
            started: Utc::now(),
            finished: None,
            entries: Vec::new(),
            reboot_required: false,
        }
    }

    pub fn push(&mut self, package: PackageId, name: String, outcome: ApplyOutcome) {
        self.entries.push(ReportEntry {
            package,
            name,
            outcome,
        });
    }

    pub fn finish(&mut self) {
        self.finished = Some(Utc::now());
    }

    pub fn updated(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.outcome.is_success())
            .count()
    }

    pub fn failed(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| {
                matches!(
                    e.outcome,
                    ApplyOutcome::Failed { .. }
                        | ApplyOutcome::VerificationFailed { .. }
                        | ApplyOutcome::DidNotConverge { .. }
                )
            })
            .count()
    }

    pub fn skipped(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e.outcome, ApplyOutcome::Skipped { .. }))
            .count()
    }

    /// Process exit code: non-zero when anything actually failed. Skips are
    /// deliberate decisions, not failures, so they do not affect this.
    pub fn exit_code(&self) -> i32 {
        if self.failed() > 0 {
            1
        } else {
            0
        }
    }
}
