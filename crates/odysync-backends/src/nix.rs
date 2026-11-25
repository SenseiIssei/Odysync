//! Nix backend for Linux and macOS.
//!
//! A profile entry remembers the flake it came from. `nix profile upgrade`
//! moves it to whatever that flake offers now and takes no version, so the
//! version is read from the flake right before, and a target that moved is
//! refused.
//!
//! Commands used:
//!   - `nix profile list --json`: installed packages (nix 2.20 or newer)
//!   - `nix eval --raw <flake>#<attribute>.version`: the version on offer
//!   - `nix profile upgrade <name>`: upgrade one entry
//!
//! Reference: <https://nix.dev/manual/nix/latest/command-ref/new-cli/nix3-profile>

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use crate::guard;

/// Evaluating a flake can mean downloading the whole of nixpkgs first.
const EVAL_TIMEOUT: Duration = Duration::from_secs(300);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

/// The commands are still marked experimental; this enables them for one call
/// instead of editing the user's configuration.
const FEATURES: [&str; 2] = ["--extra-experimental-features", "nix-command flakes"];

pub struct NixBackend;

impl NixBackend {
    pub fn new() -> Self {
        Self
    }

    async fn profile() -> Result<Vec<NixElement>> {
        let out = proc::run(
            "nix",
            &["profile", "list", "--json", FEATURES[0], FEATURES[1]],
            QUERY_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(guard::failed("nix profile list", out));
        }
        parse_profile(&out.stdout)
    }

    /// The version the entry's flake offers now, or `None` when the entry has
    /// no flake to ask.
    async fn offered(element: &NixElement) -> Result<Option<String>> {
        let Some(target) = element.version_attribute() else {
            return Ok(None);
        };
        let out = proc::run(
            "nix",
            &[
                "eval",
                "--raw",
                "--refresh",
                FEATURES[0],
                FEATURES[1],
                &target,
            ],
            EVAL_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(guard::failed(format!("nix eval {target}"), out));
        }
        let version = out.stdout.trim();
        Ok((!version.is_empty()).then(|| version.to_string()))
    }
}

impl Default for NixBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for NixBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Nix
    }

    fn display_name(&self) -> &str {
        "Nix"
    }

    async fn is_available(&self) -> bool {
        (cfg!(target_os = "linux") || cfg!(target_os = "macos"))
            && proc::exists("nix", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let mut found = Vec::new();
        for element in Self::profile().await? {
            let offered = match Self::offered(&element).await {
                Ok(Some(v)) => v,
                Ok(None) => continue,
                Err(e) => {
                    // Not every package has a version attribute.
                    tracing::warn!(package = %element.name, error = %e, "nix eval failed");
                    continue;
                }
            };
            let installed = element.installed.unwrap_or_default();
            if installed == offered {
                continue;
            }
            found.push(UpdateCandidate {
                id: PackageId::new(BackendKind::Nix, element.name.as_str()),
                name: element.name,
                installed: Version::parse(&installed),
                available: Version::parse(&offered),
                size_bytes: None,
                expected_sha256: None,
            });
        }
        Ok(found)
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        let name = candidate.id.native.as_str();
        let element = Self::profile()
            .await?
            .into_iter()
            .find(|e| e.name == name)
            .ok_or_else(|| guard::refused(candidate, "no longer in the Nix profile"))?;
        guard::still_older(candidate, element.installed.as_deref())?;
        guard::same_target(candidate, Self::offered(&element).await?.as_deref())?;

        let out = proc::run(
            "nix",
            &["profile", "upgrade", FEATURES[0], FEATURES[1], name],
            INSTALL_TIMEOUT,
        )
        .await?;
        if out.success() {
            Ok(())
        } else {
            Err(guard::failed(format!("nix profile upgrade {name}"), out))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        Ok(Self::profile()
            .await?
            .into_iter()
            .find(|e| e.name == candidate.id.native)
            .and_then(|e| e.installed))
    }
}

/// One profile entry.
#[derive(Debug, PartialEq, Eq)]
struct NixElement {
    /// What `nix profile upgrade` knows it by.
    name: String,
    installed: Option<String>,
    original_url: Option<String>,
    attr_path: Option<String>,
}

impl NixElement {
    /// `flake:nixpkgs#legacyPackages.x86_64-linux.hello.version`.
    fn version_attribute(&self) -> Option<String> {
        let url = self.original_url.as_deref()?;
        let attr = self.attr_path.as_deref()?;
        let plain = |s: &str| {
            !s.is_empty()
                && !s.starts_with('-')
                && !s.chars().any(|c| c.is_control() || c.is_whitespace())
        };
        (plain(url) && plain(attr) && !url.contains('#')).then(|| format!("{url}#{attr}.version"))
    }
}

/// Parse `nix profile list --json`.
///
/// Since nix 2.20 (manifest version 3) the elements are an object keyed by
/// name. Older versions wrote an array whose position was the name, which
/// changes with every removal, so nothing is read from those.
///
/// <https://github.com/NixOS/nix/blob/master/src/nix/profile.cc>
fn parse_profile(json: &str) -> Result<Vec<NixElement>> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        #[serde(default)]
        elements: serde_json::Value,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Element {
        #[serde(default)]
        store_paths: Vec<String>,
        original_url: Option<String>,
        attr_path: Option<String>,
    }

    let manifest: Manifest = serde_json::from_str(json.trim())
        .map_err(|e| Error::parse("nix profile list --json", e.to_string()))?;
    if !manifest.elements.is_object() {
        return Ok(Vec::new());
    }
    let elements: BTreeMap<String, Element> = serde_json::from_value(manifest.elements)
        .map_err(|e| Error::parse("nix profile list --json", e.to_string()))?;
    Ok(elements
        .into_iter()
        .map(|(name, e)| NixElement {
            name,
            installed: store_version(&e.store_paths),
            original_url: e.original_url,
            attr_path: e.attr_path,
        })
        .collect())
}

/// The version in a package's store paths, when all of them agree.
fn store_version(paths: &[String]) -> Option<String> {
    let mut versions: Vec<String> = paths.iter().filter_map(|p| path_version(p)).collect();
    versions.dedup();
    match versions.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// `/nix/store/<hash>-gdb-11.1-man` = `11.1`.
///
/// Nix's own rule: the version starts after the first dash that a digit
/// follows. Outputs other than the first one put their name behind it.
fn path_version(path: &str) -> Option<String> {
    const OUTPUTS: [&str; 9] = [
        "-bin", "-dev", "-doc", "-man", "-lib", "-out", "-info", "-debug", "-devdoc",
    ];
    let (_hash, mut name) = path.rsplit('/').next()?.split_once('-')?;
    for output in OUTPUTS {
        name = name.strip_suffix(output).unwrap_or(name);
    }
    let (start, _) = name
        .match_indices('-')
        .find(|(i, _)| name[i + 1..].starts_with(|n: char| n.is_ascii_digit()))?;
    Some(name[start + 1..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang_backends::assert_refuses_bad_input;

    /// Keys as `ProfileManifest::toJSON` writes them; the packages are the
    /// manual's own examples:
    /// https://nix.dev/manual/nix/latest/command-ref/new-cli/nix3-profile-list
    const PROFILE: &str = r#"{
  "elements": {
    "blender-bin": {
      "active": true, "attrPath": "packages.x86_64-linux.default", "originalUrl": "flake:blender-bin",
      "outputs": null, "priority": 5,
      "storePaths": ["/nix/store/rlgr8vjhcv6v2rv7ljgl0pr6g74r0cg9-blender-bin-3.1.2"],
      "url": "github:edolstra/nix-warez/91f2ffee657bf834e4475865ae336e2379282d34?dir=blender"
    },
    "gdb": {
      "active": true, "attrPath": "legacyPackages.x86_64-linux.gdb", "originalUrl": "flake:nixpkgs",
      "outputs": ["out", "man"], "priority": 5,
      "storePaths": ["/nix/store/i6i08pl20rh0lm46g38wk3bfnvhdl43d-gdb-11.1", "/nix/store/i6i08pl20rh0lm46g38wk3bfnvhdl43e-gdb-11.1-man"],
      "url": "github:NixOS/nixpkgs/7b38b03d76ab71bdc8dc325e3f6338d984cc35ca"
    },
    "local": {
      "active": true, "priority": 5,
      "storePaths": ["/nix/store/i6i08pl20rh0lm46g38wk3bfnvhdl43d-my-script"]
    }
  },
  "version": 3
}"#;

    #[test]
    fn parses_the_profile_as_nix_2_20_writes_it() {
        for text in [PROFILE.to_string(), PROFILE.replace('\n', "\r\n")] {
            let elements = parse_profile(&text).unwrap();
            assert_eq!(elements.len(), 3);
            assert_eq!(elements[0].name, "blender-bin");
            assert_eq!(elements[0].installed.as_deref(), Some("3.1.2"));
            assert_eq!(elements[1].name, "gdb");
            assert_eq!(elements[1].installed.as_deref(), Some("11.1"));
            assert_eq!(
                elements[1].version_attribute().as_deref(),
                Some("flake:nixpkgs#legacyPackages.x86_64-linux.gdb.version")
            );
            // Installed from a store path: nothing to ask, nothing to update.
            assert_eq!(elements[2].installed, None);
            assert_eq!(elements[2].version_attribute(), None);
        }
    }

    #[test]
    fn old_manifests_and_garbage() {
        let old =
            r#"{"version":2,"elements":[{"active":true,"storePaths":["/nix/store/x-gdb-11.1"]}]}"#;
        assert!(parse_profile(old).unwrap().is_empty());
        assert!(parse_profile("{}").unwrap().is_empty());
        assert!(parse_profile("").is_err());
        assert!(parse_profile("0 flake:nixpkgs#gdb").is_err());
    }

    #[test]
    fn versions_in_store_paths() {
        let v = |p: &str| path_version(&format!("/nix/store/abc123-{p}"));
        assert_eq!(v("firefox-141.0").as_deref(), Some("141.0"));
        assert_eq!(v("python3-3.12.4").as_deref(), Some("3.12.4"));
        assert_eq!(v("curl-8.5.0-bin").as_deref(), Some("8.5.0"));
        assert_eq!(v("my-script"), None);
        let paths = |list: &[&str]| -> Vec<String> {
            list.iter().map(|p| format!("/nix/store/abc-{p}")).collect()
        };
        assert_eq!(store_version(&paths(&["a-1.0", "b-2.0"])), None);
    }

    #[test]
    fn urls_that_could_be_options_are_not_evaluated() {
        let element = |url: &str, attr: &str| NixElement {
            name: "x".into(),
            installed: None,
            original_url: Some(url.into()),
            attr_path: Some(attr.into()),
        };
        assert!(element("--option", "a").version_attribute().is_none());
        assert!(
            element("flake:nixpkgs", "a b")
                .version_attribute()
                .is_none()
        );
        assert!(element("flake:x#y", "a").version_attribute().is_none());
        assert!(element("", "a").version_attribute().is_none());
    }

    #[test]
    fn a_moved_target_is_refused() {
        let c = UpdateCandidate {
            id: PackageId::new(BackendKind::Nix, "gdb"),
            name: "gdb".into(),
            installed: Version::parse("11.1"),
            available: Version::parse("12.1"),
            size_bytes: None,
            expected_sha256: None,
        };
        assert!(guard::same_target(&c, Some("12.1")).is_ok());
        assert!(guard::same_target(&c, Some("13.1")).is_err());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = NixBackend::new();
        assert_eq!(b.kind(), BackendKind::Nix);
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(&NixBackend::new(), "gdb", None).await;
    }
}
