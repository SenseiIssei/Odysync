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

    /// Render the report as human-readable text, suitable for a `.txt` file.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("Sensei's Updater — Run Report\n");
        out.push_str(&format!("Started:  {}\n", self.started));
        out.push_str(&format!(
            "Finished: {}\n",
            self.finished.map(|t| t.to_string()).unwrap_or_else(|| "-".into())
        ));
        out.push('\n');

        if self.reboot_required {
            out.push_str("Reboot required: yes\n\n");
        }

        for entry in &self.entries {
            let line = match &entry.outcome {
                ApplyOutcome::Updated { from, to } => {
                    format!("  ok   {} {} -> {}", entry.name, from, to)
                }
                ApplyOutcome::DidNotConverge { expected, actual } => {
                    format!(
                        "  !!   {} reported success but is at {actual}, expected {expected}",
                        entry.name
                    )
                }
                ApplyOutcome::VerificationFailed { detail } => {
                    format!("  !!   {} failed verification: {detail}", entry.name)
                }
                ApplyOutcome::Failed { detail } => {
                    format!("  !!   {} failed: {detail}", entry.name)
                }
                ApplyOutcome::Skipped { reason } => {
                    format!("  --   {} skipped: {reason}", entry.name)
                }
            };
            out.push_str(&line);
            out.push('\n');
        }

        out.push_str(&format!(
            "\nSummary: {} updated, {} failed, {} skipped\n",
            self.updated(),
            self.failed(),
            self.skipped()
        ));

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BackendKind, PackageId, SkipReason};

    #[test]
    fn text_report_includes_summary_counts() {
        let mut report = RunReport::new();
        report.push(
            PackageId::new(BackendKind::Winget, "Mozilla.Firefox"),
            "Firefox".into(),
            ApplyOutcome::Updated {
                from: "1.0".into(),
                to: "2.0".into(),
            },
        );
        report.push(
            PackageId::new(BackendKind::Winget, "Vendor.App"),
            "App".into(),
            ApplyOutcome::Skipped {
                reason: SkipReason::Excluded,
            },
        );
        report.finish();

        let text = report.to_text();
        assert!(text.contains("1 updated"));
        assert!(text.contains("1 skipped"));
        assert!(text.contains("Firefox 1.0 -> 2.0"));
        assert!(text.contains("App skipped"));
    }
}
