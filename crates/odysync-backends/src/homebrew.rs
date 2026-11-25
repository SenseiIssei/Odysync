//! Homebrew backend for macOS (and Linuxbrew).
//!
//! Homebrew has real machine-readable output, so unlike winget there is no
//! table parsing here. The safety story differs too: `brew upgrade <formula>`
//! always moves to the newest version in the tapped formula, and Homebrew has
//! no per-invocation version pin. So the offer is read again right before the
//! upgrade, without letting Homebrew update its index in between, and a target
//! that moved is refused.

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;
use serde::Deserialize;

use crate::guard;

const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(120);

/// Keeps `brew` from updating the index on its own, which would move the
/// target between the check and the upgrade.
const NO_AUTO_UPDATE: &str = "HOMEBREW_NO_AUTO_UPDATE=1";

/// A formula and a cask can share a name (`docker`), and `brew` then picks the
/// formula. So the id says which one it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Formula,
    Cask,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Kind::Formula => "formula:",
            Kind::Cask => "cask:",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Kind::Formula => "--formula",
            Kind::Cask => "--cask",
        }
    }
}

/// Split `cask:docker`. A name that could be an option is none.
fn parse_id(native: &str) -> Option<(Kind, &str)> {
    let (kind, name) = [Kind::Formula, Kind::Cask]
        .into_iter()
        .find_map(|k| Some((k, native.strip_prefix(k.prefix())?)))?;
    (!name.is_empty() && !name.trim_start().starts_with('-')).then_some((kind, name))
}

pub struct HomebrewBackend;

#[derive(Debug, Deserialize)]
struct Outdated {
    #[serde(default)]
    formulae: Vec<OutdatedItem>,
    #[serde(default)]
    casks: Vec<OutdatedItem>,
}

#[derive(Debug, Deserialize)]
struct OutdatedItem {
    name: String,
    #[serde(default)]
    installed_versions: Vec<String>,
    #[serde(default)]
    current_version: Option<String>,
    #[serde(default)]
    pinned: bool,
}

impl HomebrewBackend {
    pub fn new() -> Self {
        Self
    }

    /// What Homebrew would upgrade `name` to right now.
    async fn offered(kind: Kind, name: &str) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run(
            "env",
            &[
                NO_AUTO_UPDATE,
                "brew",
                "outdated",
                "--json=v2",
                kind.flag(),
                name,
            ],
            SCAN_TIMEOUT,
        )
        .await?;
        // With a name, `brew outdated` fails exactly when it is outdated.
        if out.code != 0 && out.code != 1 {
            return Err(guard::failed("brew outdated", out));
        }
        parse_outdated(&out.stdout)
    }
}

impl Default for HomebrewBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for HomebrewBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Homebrew
    }

    fn display_name(&self) -> &str {
        "Homebrew"
    }

    async fn is_available(&self) -> bool {
        !cfg!(windows) && proc::exists("brew", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // `brew update` refreshes the formula index; without it `outdated` is
        // answered from a stale local copy and misses everything published
        // since the last run.
        let _ = proc::run("brew", &["update", "--quiet"], SCAN_TIMEOUT).await;

        // Not with --greedy-auto-updates: the version Homebrew remembers for
        // an app that updates itself is the one it was installed with, so the
        // cask's version can be older than the app.
        let out = proc::run("brew", &["outdated", "--json=v2"], SCAN_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("brew outdated", out));
        }
        parse_outdated(&out.stdout)
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        let Some((kind, name)) = parse_id(&candidate.id.native) else {
            return Err(Error::invalid_package_id(
                &candidate.id.native,
                "expected formula:<name> or cask:<name>; scan again",
            ));
        };
        // Homebrew refuses to run as root and can corrupt the prefix's
        // ownership if forced; the policy layer already blocks this, but the
        // backend refuses independently so no caller can bypass it.
        if odysync_core::platform::is_elevated() {
            return Err(guard::refused(
                candidate,
                "Homebrew must not be run as root",
            ));
        }

        let offered = Self::offered(kind, name).await?;
        guard::same_target(
            candidate,
            offered
                .iter()
                .find(|c| c.id == candidate.id)
                .map(|c| c.available.raw()),
        )?;
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;

        let args = upgrade_args(kind, name);
        let out = proc::run("env", &args, INSTALL_TIMEOUT).await?;
        if out.success() {
            Ok(())
        } else {
            // No reinstall fallback here either.
            Err(guard::failed(format!("brew upgrade {name}"), out))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let Some((kind, name)) = parse_id(&candidate.id.native) else {
            return Ok(None);
        };
        let out = proc::run(
            "env",
            &[
                NO_AUTO_UPDATE,
                "brew",
                "list",
                "--versions",
                kind.flag(),
                name,
            ],
            QUERY_TIMEOUT,
        )
        .await?;

        if !out.success() {
            return Ok(None);
        }

        // Output is "name 1.2.3" or "name 1.2.3 1.2.4" when several are kept.
        Ok(parse_list_versions(&out.stdout))
    }
}

fn upgrade_args(kind: Kind, name: &str) -> [&str; 6] {
    [
        NO_AUTO_UPDATE,
        "brew",
        "upgrade",
        "--quiet",
        kind.flag(),
        name,
    ]
}

/// Parse `brew outdated --json=v2`. Pinned packages are the user's holds.
fn parse_outdated(stdout: &str) -> Result<Vec<UpdateCandidate>> {
    let parsed: Outdated = serde_json::from_str(stdout.trim())
        .map_err(|e| Error::parse("brew outdated --json=v2", e.to_string()))?;
    let formulae = parsed.formulae.into_iter().map(|i| (Kind::Formula, i));
    let casks = parsed.casks.into_iter().map(|i| (Kind::Cask, i));
    Ok(formulae
        .chain(casks)
        .filter(|(_, item)| !item.pinned)
        .map(|(kind, item)| UpdateCandidate {
            // The bare name stays an alias, so that a hold on "wget" keeps
            // working.
            id: PackageId::new(
                BackendKind::Homebrew,
                format!("{}{}", kind.prefix(), item.name),
            )
            .with_alias(item.name.as_str()),
            // Homebrew can report several installed versions side by side, in
            // no promised order.
            installed: Version::parse(
                guard::newest(
                    item.installed_versions.iter().map(String::as_str),
                    Version::parse,
                )
                .unwrap_or(""),
            ),
            available: Version::parse(item.current_version.as_deref().unwrap_or("")),
            name: item.name,
            size_bytes: None,
            expected_sha256: None,
        })
        .collect())
}

/// Extract the newest version from `brew list --versions` output.
///
/// The versions are directory names in directory order, which sorts 1.10
/// before 1.9.
fn parse_list_versions(stdout: &str) -> Option<String> {
    let line = stdout.lines().find(|l| !l.trim().is_empty())?;
    let versions = line.split_whitespace().skip(1);
    guard::newest(versions, Version::parse).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang_backends::assert_refuses_bad_input;

    #[test]
    fn reads_a_single_installed_version() {
        assert_eq!(parse_list_versions("wget 1.21.4\n"), Some("1.21.4".into()));
        assert_eq!(
            parse_list_versions("wget 1.21.4\r\n"),
            Some("1.21.4".into())
        );
    }

    #[test]
    fn picks_the_newest_when_several_are_kept() {
        assert_eq!(
            parse_list_versions("openssl 3.1.0 3.2.1\n"),
            Some("3.2.1".into())
        );
        assert_eq!(
            parse_list_versions("openssl@3 3.10.0 3.9.1 3.9.1_1\n"),
            Some("3.10.0".into())
        );
    }

    #[test]
    fn empty_output_yields_none() {
        assert_eq!(parse_list_versions(""), None);
        assert_eq!(parse_list_versions("\n  \n"), None);
    }

    #[test]
    fn a_name_with_no_version_yields_none() {
        assert_eq!(parse_list_versions("somepkg\n"), None);
    }

    /// Keys as `json_info` and `Cask#outdated_info` write them:
    /// https://github.com/Homebrew/brew/blob/HEAD/Library/Homebrew/cmd/outdated.rb
    const OUTDATED: &str = r#"{
  "formulae": [
    {"name": "wget", "installed_versions": ["1.21.10", "1.21.9"], "current_version": "1.24.5", "pinned": false, "pinned_version": null},
    {"name": "docker", "installed_versions": ["27.0.1"], "current_version": "27.1.0", "pinned": false, "pinned_version": null},
    {"name": "node@20", "installed_versions": ["20.1.0"], "current_version": "20.2.0", "pinned": true, "pinned_version": "20.1.0"}
  ],
  "casks": [
    {"name": "docker", "installed_versions": ["4.30.0"], "current_version": "4.31.0", "pinned": false, "pinned_version": null}
  ]
}"#;

    #[test]
    fn formulae_and_casks_keep_their_kind_and_pins_are_respected() {
        for text in [OUTDATED.to_string(), OUTDATED.replace('\n', "\r\n")] {
            let found = parse_outdated(&text).unwrap();
            let ids: Vec<&str> = found.iter().map(|c| c.id.native.as_str()).collect();
            assert_eq!(ids, ["formula:wget", "formula:docker", "cask:docker"]);
            assert_ne!(found[1].id, found[2].id);
            assert_eq!(found[2].id.alias.as_deref(), Some("docker"));
            // 1.21.10 is newer than 1.21.9, whatever the order of the list.
            assert_eq!(found[0].installed.raw(), "1.21.10");
            assert_eq!(found[2].available.raw(), "4.31.0");
        }
        assert!(parse_outdated("Error: something").is_err());
    }

    #[test]
    fn missing_version_fields_do_not_fail_parsing() {
        // A package with no known current version must still parse, so policy
        // can reject it with a clear reason instead of the scan erroring out.
        let json = r#"{"formulae": [{"name": "mystery"}], "casks": []}"#;
        let found = parse_outdated(json).unwrap();
        assert_eq!(found.len(), 1);
        assert!(!found[0].available.is_known());
        assert!(!found[0].installed.is_known());
    }

    #[test]
    fn the_kind_is_part_of_the_command() {
        assert_eq!(parse_id("formula:wget"), Some((Kind::Formula, "wget")));
        assert_eq!(
            parse_id("formula:user/tap/tool"),
            Some((Kind::Formula, "user/tap/tool"))
        );
        assert_eq!(parse_id("cask:docker"), Some((Kind::Cask, "docker")));
        assert_eq!(parse_id("wget"), None);
        assert_eq!(parse_id("cask:"), None);
        assert_eq!(parse_id("cask:--force"), None);
        assert_eq!(parse_id("formula: -f"), None);

        let args = upgrade_args(Kind::Cask, "docker");
        assert_eq!(args[0], "HOMEBREW_NO_AUTO_UPDATE=1");
        assert_eq!(args[args.len() - 2..], ["--cask", "docker"]);
        assert!(
            !args
                .iter()
                .any(|a| a.contains("force") || a.contains("greedy"))
        );
    }

    #[test]
    fn a_moved_target_is_refused() {
        let found = parse_outdated(OUTDATED).unwrap();
        assert!(guard::same_target(&found[0], Some("1.24.5")).is_ok());
        assert!(guard::same_target(&found[0], Some("1.25.0")).is_err());
        assert!(guard::same_target(&found[0], None).is_err());
        // Upgraded by hand in the meantime.
        assert!(guard::still_older(&found[0], Some("1.25.0")).is_err());
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        let backend = HomebrewBackend::new();
        assert_refuses_bad_input(&backend, "formula:wget", None).await;
        // An id from before the kind was part of it, and an option behind the
        // prefix.
        for id in ["wget", "cask:--force"] {
            let candidate = UpdateCandidate {
                id: PackageId::new(BackendKind::Homebrew, id),
                name: id.into(),
                installed: Version::parse("1.0"),
                available: Version::parse("2.0"),
                size_bytes: None,
                expected_sha256: None,
            };
            assert!(matches!(
                backend.apply(&candidate).await.unwrap_err(),
                Error::InvalidPackageId { .. }
            ));
        }
    }
}
