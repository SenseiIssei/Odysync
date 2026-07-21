//! Persistent update history.
//!
//! Records every apply attempt (success or failure) to a JSON file so the GUI
//! can show "recently updated" and the user can audit what happened and when.
//!
//! The history file lives in the Odysync config directory as `history.json`.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{ApplyOutcome, BackendKind, PackageId};

/// A single history entry recording the result of one update attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// When the apply was attempted.
    pub timestamp: DateTime<Utc>,
    /// Which package was being updated.
    pub package_id: String,
    /// Human-readable package name.
    pub package_name: String,
    /// Which backend handled the update.
    pub backend: BackendKind,
    /// Version before the update.
    pub from_version: String,
    /// Target version.
    pub to_version: String,
    /// Outcome of the attempt.
    pub outcome: HistoryOutcome,
}

/// Simplified outcome for history storage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryOutcome {
    Updated,
    Failed,
    Skipped,
    DidNotConverge,
}

impl From<&ApplyOutcome> for HistoryOutcome {
    fn from(o: &ApplyOutcome) -> Self {
        match o {
            ApplyOutcome::Updated { .. } => HistoryOutcome::Updated,
            ApplyOutcome::Failed { .. } => HistoryOutcome::Failed,
            ApplyOutcome::Skipped { .. } => HistoryOutcome::Skipped,
            ApplyOutcome::DidNotConverge { .. } => HistoryOutcome::DidNotConverge,
            ApplyOutcome::VerificationFailed { .. } => HistoryOutcome::Failed,
        }
    }
}

/// Manages the persistent update history file.
pub struct UpdateHistory {
    entries: Vec<HistoryEntry>,
    path: PathBuf,
    max_entries: usize,
}

impl UpdateHistory {
    /// Load history from the default config directory, creating the file if
    /// it does not exist.
    pub fn load() -> Self {
        let path = history_path();
        let entries = match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        Self {
            entries,
            path,
            max_entries: 500,
        }
    }

    /// Load from a specific path (useful for testing).
    pub fn load_from(path: PathBuf) -> Self {
        let entries = match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        Self {
            entries,
            path,
            max_entries: 500,
        }
    }

    /// Record a completed update attempt.
    pub fn record(
        &mut self,
        package_id: &PackageId,
        package_name: &str,
        from: &str,
        to: &str,
        outcome: &ApplyOutcome,
    ) {
        let entry = HistoryEntry {
            timestamp: Utc::now(),
            package_id: package_id.to_string(),
            package_name: package_name.to_string(),
            backend: package_id.backend,
            from_version: from.to_string(),
            to_version: to.to_string(),
            outcome: HistoryOutcome::from(outcome),
        };
        self.entries.push(entry);

        // Trim to max size, keeping the most recent entries.
        if self.entries.len() > self.max_entries {
            let excess = self.entries.len() - self.max_entries;
            self.entries.drain(0..excess);
        }
    }

    /// Persist the history to disk.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.entries)?;
        std::fs::write(&self.path, json)
    }

    /// Return all history entries, most recent last.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// Return entries for a specific backend.
    pub fn for_backend(&self, kind: BackendKind) -> Vec<&HistoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.backend == kind)
            .collect()
    }

    /// Return the most recent `n` entries.
    pub fn recent(&self, n: usize) -> Vec<&HistoryEntry> {
        let len = self.entries.len();
        let start = len.saturating_sub(n);
        self.entries[start..].iter().collect()
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

fn history_path() -> PathBuf {
    // Must match Config::default_path's qualifier, or history lands in a
    // different directory tree from the config it is documented to sit beside.
    let dir = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    dir.join("history.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn history_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");

        let mut history = UpdateHistory::load_from(path.clone());
        history.record(
            &PackageId::new(BackendKind::Winget, "test.package"),
            "Test Package",
            "1.0.0",
            "2.0.0",
            &ApplyOutcome::Updated {
                from: "1.0.0".into(),
                to: "2.0.0".into(),
            },
        );
        history.save().unwrap();

        let loaded = UpdateHistory::load_from(path);
        assert_eq!(loaded.entries().len(), 1);
        assert_eq!(loaded.entries()[0].package_name, "Test Package");
        assert_eq!(loaded.entries()[0].outcome, HistoryOutcome::Updated);
    }

    #[test]
    fn history_trims_to_max_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");

        let mut history = UpdateHistory::load_from(path);
        history.max_entries = 3;
        for i in 0..10 {
            history.record(
                &PackageId::new(BackendKind::Winget, format!("pkg.{i}")),
                &format!("Package {i}"),
                "1.0.0",
                "2.0.0",
                &ApplyOutcome::Updated {
                    from: "1.0.0".into(),
                    to: "2.0.0".into(),
                },
            );
        }
        assert_eq!(history.entries().len(), 3);
        // Should keep the last 3 entries.
        assert_eq!(history.entries()[0].package_name, "Package 7");
        assert_eq!(history.entries()[2].package_name, "Package 9");
    }

    #[test]
    fn history_filters_by_backend() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");

        let mut history = UpdateHistory::load_from(path);
        history.record(
            &PackageId::new(BackendKind::Winget, "pkg.winget"),
            "Winget Package",
            "1.0.0",
            "2.0.0",
            &ApplyOutcome::Updated {
                from: "1.0.0".into(),
                to: "2.0.0".into(),
            },
        );
        history.record(
            &PackageId::new(BackendKind::Apt, "pkg.apt"),
            "Apt Package",
            "1.0.0",
            "2.0.0",
            &ApplyOutcome::Failed { detail: "test".into() },
        );

        let winget_entries = history.for_backend(BackendKind::Winget);
        assert_eq!(winget_entries.len(), 1);
        assert_eq!(winget_entries[0].package_name, "Winget Package");

        let apt_entries = history.for_backend(BackendKind::Apt);
        assert_eq!(apt_entries.len(), 1);
        assert_eq!(apt_entries[0].outcome, HistoryOutcome::Failed);
    }

    #[test]
    fn history_returns_recent_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");

        let mut history = UpdateHistory::load_from(path);
        for i in 0..5 {
            history.record(
                &PackageId::new(BackendKind::Winget, format!("pkg.{i}")),
                &format!("Package {i}"),
                "1.0.0",
                "2.0.0",
                &ApplyOutcome::Updated {
                    from: "1.0.0".into(),
                    to: "2.0.0".into(),
                },
            );
        }

        let recent = history.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].package_name, "Package 3");
        assert_eq!(recent[1].package_name, "Package 4");
    }
}
