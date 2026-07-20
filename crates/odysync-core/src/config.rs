//! On-disk configuration, stored as JSON in the platform's config directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::BackendKind;
use crate::policy::Policy;

/// The full user configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub policy: Policy,
    /// Backends to skip entirely. Empty means "use everything available".
    pub disabled_backends: Vec<String>,
    /// Named sets of packages, for updating a subset at a time.
    pub profiles: Vec<Profile>,
    /// Take a system restore point (Windows) before applying anything.
    pub restore_point: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Profile {
    pub name: String,
    pub packages: Vec<String>,
}

impl Config {
    /// The config file path for the current user, e.g.
    /// `%APPDATA%\Odysync\config.json` or `~/.config/odysync/config.json`.
    pub fn default_path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
            .ok_or_else(|| Error::Config("could not resolve a config directory".into()))?;
        Ok(dirs.config_dir().join("config.json"))
    }

    /// Load config from `path`, returning defaults when the file is absent.
    ///
    /// A *corrupt* file is an error rather than a silent reset — losing a
    /// user's holds and exclusions without telling them could let a held
    /// package update.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                Error::Config(format!(
                    "{} is not valid config: {e}. Fix or delete it; refusing to \
                     continue with default settings so holds are not lost.",
                    path.display()
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Write config to `path`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        // Write-then-rename so an interrupted save cannot truncate the config.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Whether `kind` should be used during this run.
    pub fn backend_enabled(&self, kind: BackendKind) -> bool {
        !self
            .disabled_backends
            .iter()
            .any(|d| d.eq_ignore_ascii_case(kind.id()))
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let cfg = Config::load(Path::new("does-not-exist-xyz.json")).unwrap();
        assert!(cfg.policy.stable_only);
        assert!(cfg.policy.require_known_versions);
    }

    #[test]
    fn corrupt_file_is_an_error_not_a_silent_reset() {
        let dir = std::env::temp_dir().join("odysync-cfg-test-corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, "{ this is not json").unwrap();

        let err = Config::load(&path).unwrap_err();
        assert!(matches!(err, Error::Config(_)));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join("odysync-cfg-test-roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");

        let mut cfg = Config::default();
        cfg.policy.exclude.push("Mozilla.Firefox".into());
        cfg.disabled_backends.push("msstore".into());
        cfg.save(&path).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.policy.exclude, vec!["Mozilla.Firefox".to_string()]);
        assert!(!loaded.backend_enabled(BackendKind::MsStore));
        assert!(loaded.backend_enabled(BackendKind::Winget));

        std::fs::remove_dir_all(&dir).ok();
    }
}
