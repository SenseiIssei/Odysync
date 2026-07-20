//! On-disk configuration, stored as JSON in the platform's config directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::BackendKind;
use crate::policy::Policy;

/// The full user configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub policy: Policy,
    /// Backends to skip entirely. Empty means "use everything available".
    pub disabled_backends: Vec<String>,
    /// Named sets of packages, for updating a subset at a time.
    pub profiles: Vec<Profile>,
    /// Take a system restore point (Windows) before applying anything.
    pub restore_point: bool,
    /// Scan interval in hours for background scanning (0 = manual only).
    pub scan_interval_hours: u32,
    /// Max concurrent backend scans/applys.
    pub concurrency: u32,
    /// HTTP proxy URL for web crawlers and registry requests.
    pub proxy_url: Option<String>,
    /// Automatically apply updates after scanning (dangerous).
    pub auto_apply: bool,
    /// Show desktop notifications for scan results and apply completion.
    pub notifications: bool,
    /// Skip pre-release versions (alias for policy.stable_only, kept for UI).
    pub skip_prerelease: bool,
    /// Retry failed updates up to N times.
    pub max_retries: u32,
    /// Timeout for individual backend operations in seconds.
    pub backend_timeout_secs: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Profile {
    pub name: String,
    pub packages: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            disabled_backends: Vec::new(),
            profiles: Vec::new(),
            restore_point: true,
            scan_interval_hours: 24,
            concurrency: 4,
            proxy_url: None,
            auto_apply: false,
            notifications: true,
            skip_prerelease: true,
            max_retries: 2,
            backend_timeout_secs: 120,
        }
    }
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
