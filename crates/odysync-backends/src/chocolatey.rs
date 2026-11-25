//! Chocolatey backend for Windows.
//!
//! Chocolatey is a Windows package manager that verifies package checksums
//! and signatures. We use `--limit-output` for machine-readable output.
//!
//! Commands used:
//!   - `choco outdated -r`: list outdated packages
//!   - `choco upgrade <pkg> --version <ver> --fail-on-not-installed -y -r`
//!   - `choco list --exact -r <pkg>`: read the installed version

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::Result;
use odysync_core::model::{BackendKind, InstalledPackage, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use crate::guard;

/// One request to the feed per installed package.
const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
/// Chocolatey stops an installer itself after 2700 seconds. Ours has to be
/// longer, or an installer is left running with nobody waiting for it.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(2700 + 600);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

pub struct ChocolateyBackend;

impl ChocolateyBackend {
    pub fn new() -> Self {
        Self
    }

    /// Run `choco list` so that it looks at this machine only.
    async fn local_list(args: &[&str]) -> Result<proc::Output> {
        let version = proc::run("choco", &["--version"], QUERY_TIMEOUT).await?;
        let all = local_list_args(&version.stdout, args);
        proc::run("choco", &all, SCAN_TIMEOUT).await
    }
}

impl Default for ChocolateyBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for ChocolateyBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Chocolatey
    }

    fn display_name(&self) -> &str {
        "Chocolatey"
    }

    async fn is_available(&self) -> bool {
        cfg!(windows) && proc::exists("choco", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("choco", &["outdated", "-r"], SCAN_TIMEOUT).await?;
        // 2 is "outdated packages have been found" where the user turned
        // enhanced exit codes on.
        if out.code != 0 && out.code != 2 {
            return Err(guard::failed("choco outdated", out));
        }
        Ok(parse_choco_outdated(&out.stdout))
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let out = Self::local_list(&["-r"]).await?;
        if !out.success() {
            return Err(guard::failed("choco list", out));
        }
        Ok(parse_choco_list(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;

        let args = upgrade_args(&candidate.id.native, candidate.available.raw());
        let out = proc::run("choco", &args, INSTALL_TIMEOUT).await?;
        if is_success(out.code) {
            Ok(())
        } else {
            Err(guard::failed(
                format!(
                    "choco upgrade {} {}",
                    candidate.id.native,
                    candidate.available.raw()
                ),
                out,
            ))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = Self::local_list(&["--exact", "-r", &candidate.id.native]).await?;
        if !out.success() {
            return Ok(None);
        }
        Ok(parse_choco_list(&out.stdout)
            .into_iter()
            .find(|p| p.id.native.eq_ignore_ascii_case(&candidate.id.native))
            .map(|p| p.version))
    }
}

/// Since version 2 `choco list` is local and no longer knows `--local-only`;
/// before that it searched the feed without it, and the newest version there
/// would have been read as the installed one.
fn local_list_args<'a>(choco_version: &str, args: &[&'a str]) -> Vec<&'a str> {
    let major: Option<u32> = choco_version
        .trim()
        .split('.')
        .next()
        .and_then(|m| m.parse().ok());
    let mut all = vec!["list"];
    if matches!(major, Some(0 | 1)) {
        all.push("--local-only");
    }
    all.extend(args);
    all
}

/// `--fail-on-not-installed`: without it, `choco upgrade` installs what is
/// missing.
fn upgrade_args<'a>(id: &'a str, version: &'a str) -> [&'a str; 8] {
    [
        "upgrade",
        id,
        "--version",
        version,
        "--fail-on-not-installed",
        "-y",
        "-r",
        "--no-progress",
    ]
}

/// 1641: success, reboot initiated. 3010: success, reboot required.
///
/// <https://docs.chocolatey.org/en-us/choco/commands/upgrade/#exit-codes>
fn is_success(code: i32) -> bool {
    matches!(code, 0 | 1641 | 3010)
}

/// Parse `choco outdated -r` output.
///
/// Format: `package_name|current_version|available_version|pinned?(true/false)`
fn parse_choco_outdated(stdout: &str) -> Vec<UpdateCandidate> {
    stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.trim().split('|').map(str::trim).collect();
            let (name, current, available) = match parts.as_slice() {
                // Pinned: the user's hold.
                [_, _, _, pinned, ..] if pinned.eq_ignore_ascii_case("true") => return None,
                [name, current, available, ..] => (*name, *current, *available),
                _ => return None,
            };
            // A package no feed knows is listed with its own version twice.
            if name.is_empty() || current == available {
                return None;
            }

            Some(UpdateCandidate {
                id: PackageId::new(BackendKind::Chocolatey, name),
                name: name.to_string(),
                installed: Version::parse(current),
                available: Version::parse(available),
                size_bytes: None,
                expected_sha256: None,
            })
        })
        .collect()
}

/// Parse `choco list -r` output.
///
/// Format: `package_name|version`, one per line. Lines without a pipe are
/// banner or summary text ("3 packages installed.") and are skipped rather
/// than guessed at.
fn parse_choco_list(stdout: &str) -> Vec<InstalledPackage> {
    stdout
        .lines()
        .filter_map(|line| {
            let (name, version) = line.trim().split_once('|')?;
            let name = name.trim();
            let version = version.split('|').next().unwrap_or("").trim();
            if name.is_empty() {
                return None;
            }
            Some(InstalledPackage {
                id: PackageId::new(BackendKind::Chocolatey, name),
                name: name.to_string(),
                version: version.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang_backends::assert_refuses_bad_input;

    fn crlf(text: &str) -> String {
        text.replace('\n', "\r\n")
    }

    #[test]
    fn parses_choco_list() {
        let output = "\
chocolatey|2.2.2
firefox|141.0
7zip|24.09
";
        for text in [output.to_string(), crlf(output)] {
            let installed = parse_choco_list(&text);
            assert_eq!(installed.len(), 3);
            assert_eq!(installed[0].id.native, "chocolatey");
            assert_eq!(installed[0].version, "2.2.2");
            assert_eq!(installed[1].name, "firefox");
            assert_eq!(installed[2].version, "24.09");
        }
    }

    #[test]
    fn choco_list_skips_lines_without_a_pipe() {
        let output = "firefox|141.0\n3 packages installed.\n\n";
        let installed = parse_choco_list(output);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].id.native, "firefox");
    }

    #[test]
    fn choco_list_empty_output_yields_empty_vec() {
        assert!(parse_choco_list("").is_empty());
    }

    #[test]
    fn local_list_is_local_in_both_major_versions() {
        assert_eq!(
            local_list_args("2.4.3\r\n", &["--exact", "-r", "7zip"]),
            ["list", "--exact", "-r", "7zip"]
        );
        assert_eq!(
            local_list_args("1.4.0\n", &["-r"]),
            ["list", "--local-only", "-r"]
        );
        assert_eq!(local_list_args("", &["-r"]), ["list", "-r"]);
    }

    #[test]
    fn parses_choco_outdated() {
        // The four columns come from `OutputOutdatedRow(id, version,
        // availableVersion, pinned)`; the last row is how a package is written
        // that no feed knows.
        // https://github.com/chocolatey/choco/blob/develop/src/chocolatey/infrastructure.app/services/NugetService.cs
        let output = "\
firefox|140.0|141.0|false
7zip|23.01|24.07|false
nodejs|20.0.0|22.0.0|true
mytool|1.0.0|1.0.0|false
";
        for text in [output.to_string(), crlf(output)] {
            let candidates = parse_choco_outdated(&text);
            assert_eq!(candidates.len(), 2);
            assert_eq!(candidates[0].id.native, "firefox");
            assert_eq!(candidates[0].installed.raw(), "140.0");
            assert_eq!(candidates[0].available.raw(), "141.0");
            assert_eq!(candidates[1].id.native, "7zip");
        }
    }

    #[test]
    fn skips_malformed_lines() {
        let output = "firefox|140.0|141.0|false\nbad line\n";
        let candidates = parse_choco_outdated(output);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_choco_outdated("").is_empty());
    }

    #[test]
    fn the_upgrade_is_pinned_and_never_installs() {
        let args = upgrade_args("firefox", "141.0");
        assert_eq!(args[..4], ["upgrade", "firefox", "--version", "141.0"]);
        assert!(args.contains(&"--fail-on-not-installed"));
        assert!(
            !args
                .iter()
                .any(|a| a.contains("force") || a.contains("downgrade"))
        );
    }

    #[test]
    fn a_restart_is_a_success_and_everything_else_is_not() {
        for code in [0, 1641, 3010] {
            assert!(is_success(code));
        }
        // 350 and 1604: a pending restart stopped it, nothing was installed.
        for code in [-1, 1, 2, 350, 1603, 1604, 1618] {
            assert!(!is_success(code), "{code}");
        }
    }

    #[test]
    fn our_timeout_is_longer_than_chocolateys() {
        assert!(INSTALL_TIMEOUT.as_secs() > 2700);
    }

    #[test]
    fn a_package_that_is_gone_or_newer_is_refused() {
        let found = parse_choco_outdated("firefox|140.0|141.0|false\n");
        assert!(guard::still_older(&found[0], Some("140.0")).is_ok());
        assert!(guard::still_older(&found[0], Some("141.0")).is_err());
        assert!(guard::still_older(&found[0], None).is_err());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = ChocolateyBackend::new();
        assert_eq!(b.kind(), BackendKind::Chocolatey);
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(&ChocolateyBackend::new(), "firefox", None).await;
    }
}
