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
    /// Backends that are off until named here.
    ///
    /// Only [`BackendKind::enabled_by_default`] returning `false` puts a
    /// backend in this position — today that is Windows feature upgrades.
    /// Listing one here is the user saying "yes, this specific thing too".
    pub enabled_backends: Vec<String>,
    /// Backends paused until a moment in time.
    ///
    /// Windows itself lets you defer updates for a few weeks, and the reason
    /// is sound: a cumulative update occasionally breaks something, and
    /// waiting for other people to find out first is a legitimate strategy.
    /// Expressing it as a deadline rather than an off switch is the part that
    /// matters — "off" gets set during one bad week and is still off a year
    /// later, which is how machines end up unpatched.
    pub paused_backends: Vec<PausedBackend>,
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
    /// Export the whole driver store before applying driver updates (Windows).
    ///
    /// **Off by default, and the reason is a measurement rather than a
    /// preference.** A full export is a copy of
    /// `C:\Windows\System32\DriverStore\FileRepository`, which was 4.7 GB
    /// across 5 229 files on the machine this was written against. Doing that
    /// automatically before every driver update, times
    /// [`driver_backup_keep`](Config::driver_backup_keep) generations, spends
    /// well over ten gigabytes on the disk of the machine the tool is supposed
    /// to be looking after.
    ///
    /// Windows already keeps the immediately previous driver for its own
    /// Device Manager rollback, so the marginal value of an automatic full
    /// export is smaller than its cost. `odysync drivers backup` takes one on
    /// demand, and the apply path says so when none exists.
    pub driver_backup_before_apply: bool,
    /// How many driver backups to keep when pruning.
    pub driver_backup_keep: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Profile {
    pub name: String,
    pub packages: Vec<String>,
}

/// A backend the user has deferred until a given moment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PausedBackend {
    /// Backend id, e.g. `windows-update`.
    pub backend: String,
    /// RFC 3339 instant at which the pause lapses.
    pub until: String,
    /// Free text, shown when the pause is what hid a backend.
    pub note: Option<String>,
}

impl PausedBackend {
    /// Whether this pause is still in force at `now`.
    ///
    /// An `until` that cannot be parsed is treated as **expired**, not as
    /// permanent. A typo in a hand-edited config must not silently stop
    /// security updates forever; failing open here means the worst case is an
    /// update the user wanted to defer, rather than one they never receive.
    pub fn is_active_at(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        match chrono::DateTime::parse_from_rfc3339(self.until.trim()) {
            Ok(until) => now < until.with_timezone(&chrono::Utc),
            Err(e) => {
                tracing::warn!(
                    backend = %self.backend,
                    until = %self.until,
                    error = %e,
                    "unparseable pause deadline; treating the pause as expired"
                );
                false
            }
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            disabled_backends: Vec::new(),
            enabled_backends: Vec::new(),
            paused_backends: Vec::new(),
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
            driver_backup_before_apply: false,
            driver_backup_keep: 2,
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
    ///
    /// An explicit disable always wins, so a user who turns something off does
    /// not have it turned back on by an opt-in list they edited earlier.
    pub fn backend_enabled(&self, kind: BackendKind) -> bool {
        self.backend_enabled_at(kind, chrono::Utc::now())
    }

    /// [`backend_enabled`](Config::backend_enabled) as of a given moment.
    ///
    /// Split out so the pause logic is testable without waiting for a clock.
    pub fn backend_enabled_at(
        &self,
        kind: BackendKind,
        now: chrono::DateTime<chrono::Utc>,
    ) -> bool {
        if self
            .disabled_backends
            .iter()
            .any(|d| d.eq_ignore_ascii_case(kind.id()))
        {
            return false;
        }

        if self.pause_for_at(kind, now).is_some() {
            return false;
        }

        kind.enabled_by_default()
            || self
                .enabled_backends
                .iter()
                .any(|e| e.eq_ignore_ascii_case(kind.id()))
    }

    /// The pause currently hiding `kind`, if one is.
    ///
    /// Returned rather than just tested so a front-end can say *why* a backend
    /// is missing and when it comes back — "paused until 3 August" is
    /// actionable, "no updates found" is misleading.
    pub fn pause_for(&self, kind: BackendKind) -> Option<&PausedBackend> {
        self.pause_for_at(kind, chrono::Utc::now())
    }

    fn pause_for_at(
        &self,
        kind: BackendKind,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<&PausedBackend> {
        self.paused_backends
            .iter()
            .find(|p| p.backend.eq_ignore_ascii_case(kind.id()) && p.is_active_at(now))
    }

    /// Pause `kind` until `until`, replacing any existing pause for it.
    pub fn pause_backend(
        &mut self,
        kind: BackendKind,
        until: chrono::DateTime<chrono::Utc>,
        note: Option<String>,
    ) {
        self.paused_backends
            .retain(|p| !p.backend.eq_ignore_ascii_case(kind.id()));
        self.paused_backends.push(PausedBackend {
            backend: kind.id().to_string(),
            until: until.to_rfc3339(),
            note,
        });
    }

    /// Lift any pause on `kind`. Returns whether one was in the list.
    pub fn resume_backend(&mut self, kind: BackendKind) -> bool {
        let before = self.paused_backends.len();
        self.paused_backends
            .retain(|p| !p.backend.eq_ignore_ascii_case(kind.id()));
        self.paused_backends.len() != before
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
        cfg.disabled_backends.push("pip".into());
        cfg.enabled_backends.push("winget".into());
        cfg.save(&path).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.policy.exclude, vec!["Mozilla.Firefox".to_string()]);
        assert!(!loaded.backend_enabled(BackendKind::Pip));
        assert!(loaded.backend_enabled(BackendKind::Winget));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn feature_upgrades_are_off_until_named() {
        let mut cfg = Config::default();
        assert!(
            !cfg.backend_enabled(BackendKind::WindowsFeatureUpdate),
            "a Windows version upgrade must never be on by default"
        );

        cfg.enabled_backends.push("windows-feature-update".into());
        assert!(cfg.backend_enabled(BackendKind::WindowsFeatureUpdate));
    }

    /// Software updates must work on a machine without winget, so nothing
    /// that shells out to winget may be on unless the user asked for it.
    #[test]
    fn winget_and_everything_delivered_through_it_is_opt_in() {
        let mut cfg = Config::default();
        for kind in BackendKind::ALL {
            if kind.delivered_through_winget() {
                assert!(!cfg.backend_enabled(*kind), "{kind} is on by default");
            }
        }
        assert!(cfg.backend_enabled(BackendKind::Direct));
        assert!(cfg.backend_enabled(BackendKind::NvidiaGpu));

        cfg.enabled_backends.push("winget".into());
        assert!(cfg.backend_enabled(BackendKind::Winget));
    }

    #[test]
    fn disabling_beats_opting_in() {
        let cfg = Config {
            enabled_backends: vec!["windows-feature-update".into()],
            disabled_backends: vec!["windows-feature-update".into()],
            ..Config::default()
        };
        assert!(!cfg.backend_enabled(BackendKind::WindowsFeatureUpdate));
    }

    #[test]
    fn ordinary_backends_need_no_opt_in() {
        let cfg = Config::default();
        for kind in [
            BackendKind::Direct,
            BackendKind::WindowsUpdate,
            BackendKind::WindowsDefenderUpdate,
            BackendKind::WindowsDrivers,
        ] {
            assert!(cfg.backend_enabled(kind), "{kind} should be on by default");
        }
    }

    fn at(text: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn a_pause_hides_a_backend_until_it_lapses() {
        let mut cfg = Config::default();
        cfg.pause_backend(
            BackendKind::WindowsUpdate,
            at("2026-08-03T00:00:00Z"),
            Some("waiting out a bad cumulative".into()),
        );

        assert!(!cfg.backend_enabled_at(BackendKind::WindowsUpdate, at("2026-07-27T12:00:00Z")));
        // One second past the deadline the backend is back, with no further
        // action from the user. That is the point of a deadline over a switch.
        assert!(cfg.backend_enabled_at(BackendKind::WindowsUpdate, at("2026-08-03T00:00:01Z")));
    }

    #[test]
    fn a_pause_only_affects_the_backend_it_names() {
        let mut cfg = Config::default();
        cfg.pause_backend(BackendKind::WindowsUpdate, at("2026-08-03T00:00:00Z"), None);
        assert!(cfg.backend_enabled_at(
            BackendKind::WindowsDefenderUpdate,
            at("2026-07-27T12:00:00Z")
        ));
    }

    #[test]
    fn pausing_twice_replaces_rather_than_stacks() {
        let mut cfg = Config::default();
        cfg.pause_backend(BackendKind::WindowsUpdate, at("2026-08-03T00:00:00Z"), None);
        cfg.pause_backend(BackendKind::WindowsUpdate, at("2026-09-01T00:00:00Z"), None);
        assert_eq!(cfg.paused_backends.len(), 1);
        assert!(!cfg.backend_enabled_at(BackendKind::WindowsUpdate, at("2026-08-15T00:00:00Z")));
    }

    #[test]
    fn resuming_reports_whether_a_pause_was_lifted() {
        let mut cfg = Config::default();
        assert!(!cfg.resume_backend(BackendKind::WindowsUpdate));
        cfg.pause_backend(BackendKind::WindowsUpdate, at("2026-08-03T00:00:00Z"), None);
        assert!(cfg.resume_backend(BackendKind::WindowsUpdate));
        assert!(cfg.backend_enabled(BackendKind::WindowsUpdate));
    }

    /// A hand-edited config with a broken date must not stop security updates
    /// forever. Failing open costs the user a deferral they wanted; failing
    /// closed costs them every patch from then on.
    #[test]
    fn an_unparseable_deadline_is_treated_as_expired() {
        let cfg = Config {
            paused_backends: vec![PausedBackend {
                backend: "windows-update".into(),
                until: "next tuesday".into(),
                note: None,
            }],
            ..Config::default()
        };
        assert!(cfg.backend_enabled_at(BackendKind::WindowsUpdate, at("2026-07-27T12:00:00Z")));
    }

    #[test]
    fn the_active_pause_is_retrievable_so_a_ui_can_explain_itself() {
        let mut cfg = Config::default();
        cfg.pause_backend(
            BackendKind::WindowsUpdate,
            at("2026-08-03T00:00:00Z"),
            Some("waiting out a bad cumulative".into()),
        );
        let pause = cfg
            .pause_for_at(BackendKind::WindowsUpdate, at("2026-07-27T12:00:00Z"))
            .expect("pause should be reported");
        assert_eq!(pause.note.as_deref(), Some("waiting out a bad cumulative"));
        assert!(
            cfg.pause_for_at(BackendKind::WindowsUpdate, at("2026-08-04T00:00:00Z"))
                .is_none()
        );
    }

    #[test]
    fn disabling_still_beats_an_expired_pause() {
        let cfg = Config {
            disabled_backends: vec!["windows-update".into()],
            paused_backends: vec![PausedBackend {
                backend: "windows-update".into(),
                until: "2020-01-01T00:00:00Z".into(),
                note: None,
            }],
            ..Config::default()
        };
        assert!(!cfg.backend_enabled_at(BackendKind::WindowsUpdate, at("2026-07-27T12:00:00Z")));
    }

    /// An existing user's config already has `disabled-backends: []` on disk.
    /// Had feature upgrades been gated by a default entry in that list, this
    /// case would silently enable them for everyone who ran an older build.
    #[test]
    fn an_existing_config_file_still_gates_feature_upgrades() {
        let cfg: Config = serde_json::from_str(r#"{"disabled-backends": []}"#).unwrap();
        assert!(!cfg.backend_enabled(BackendKind::WindowsFeatureUpdate));
    }
}
