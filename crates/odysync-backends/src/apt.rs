//! APT backend for Debian, Ubuntu and derivatives.
//!
//! APT is the safest of the three platforms by construction: packages are
//! signed at the repository level and `apt-get` verifies those signatures
//! itself, so integrity does not depend on us. What we add is exact version
//! pinning (`apt-get install pkg=version`), which keeps the install identical
//! to what the user approved in the plan even if the mirror moves underneath.

use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::Result;
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;
use odysync_core::version_native::Scheme;

use crate::guard;

const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

pub struct AptBackend;

impl AptBackend {
    pub fn new() -> Self {
        Self
    }

    /// Packages the user told APT to leave alone.
    async fn holds() -> Result<HashSet<String>> {
        let out = proc::run("apt-mark", &["showhold"], QUERY_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("apt-mark showhold", out));
        }
        Ok(parse_holds(&out.stdout))
    }
}

impl Default for AptBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for AptBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Apt
    }

    fn display_name(&self) -> &str {
        "APT"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("apt-get", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // Refreshing the index needs root; without it we still scan, just
        // against whatever the last `apt update` left behind.
        if odysync_core::platform::is_elevated() {
            let _ = proc::run(
                "apt-get",
                &["update", "-qq", "-o", "DPkg::Lock::Timeout=60"],
                SCAN_TIMEOUT,
            )
            .await;
        }

        let out = proc::run("apt", &["list", "--upgradable"], SCAN_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("apt list --upgradable", out));
        }

        let holds = Self::holds().await?;
        Ok(parse_upgradable(&out.stdout)
            .into_iter()
            .filter(|c| !is_held(&holds, &c.id.native))
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        let id = candidate.id.native.as_str();

        if is_held(&Self::holds().await?, id) {
            return Err(guard::refused(candidate, "the package is on hold in APT"));
        }
        let policy = proc::run("apt-cache", &["policy", "--", id], QUERY_TIMEOUT).await?;
        if !policy.success() {
            return Err(guard::failed(format!("apt-cache policy {id}"), policy));
        }
        guard::same_target(candidate, parse_policy_candidate(&policy.stdout).as_deref())?;
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;

        let args = install_args(id, candidate.available.raw());
        let out = proc::run("env", &args, INSTALL_TIMEOUT).await?;
        if out.success() {
            Ok(())
        } else {
            Err(guard::failed(
                format!("apt-get install {id}={}", candidate.available.raw()),
                out,
            ))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = proc::run(
            "dpkg-query",
            &[
                "-W",
                // dpkg-query expands the escape itself.
                "-f=${db:Status-Status} ${Version}\\n",
                "--",
                &candidate.id.native,
            ],
            QUERY_TIMEOUT,
        )
        .await?;

        if !out.success() {
            return Ok(None);
        }
        Ok(parse_installed(&out.stdout))
    }
}

/// The command line of one update, run through `env`.
///
/// `proc` sets no environment of its own, and without a frontend setting
/// debconf waits for an answer on a terminal nobody has.
fn install_args(id: &str, version: &str) -> Vec<String> {
    // Pin the exact version, and refuse anything that would remove other
    // packages to satisfy the upgrade. `--no-remove` turns a destructive
    // dependency resolution into a clean failure.
    [
        "DEBIAN_FRONTEND=noninteractive",
        "apt-get",
        "install",
        "--only-upgrade",
        "--no-remove",
        "-y",
        // Wait for another apt instead of failing on its lock.
        "-o",
        "DPkg::Lock::Timeout=60",
        "-o",
        "Dpkg::Options::=--force-confold",
        "--",
    ]
    .into_iter()
    .map(str::to_string)
    .chain([format!("{id}={version}")])
    .collect()
}

/// Parse `apt list --upgradable`.
///
/// Each line looks like:
/// `firefox/noble-updates 141.0+build1 amd64 [upgradable from: 140.0+build2]`
///
/// The same package can be installed for two architectures, so the id carries
/// the architecture (`libc6:i386`). A package of architecture `all` exists
/// once, and APT knows it by its bare name.
fn parse_upgradable(stdout: &str) -> Vec<UpdateCandidate> {
    let mut out = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        // The first line is "Listing..." and there may be warnings.
        let Some((head, tail)) = line.split_once("[upgradable from:") else {
            continue;
        };
        let installed = tail.trim().trim_end_matches(']').trim();
        let Some((name, rest)) = head.split_once('/') else {
            continue;
        };
        let name = name.trim();
        // rest: "noble-updates 141.0+build1 amd64"
        let mut fields = rest.split_whitespace();
        let (Some(_suite), Some(available), Some(arch)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if name.is_empty() || name.contains(char::is_whitespace) {
            continue;
        }

        let native = if arch == "all" {
            name.to_string()
        } else {
            format!("{name}:{arch}")
        };
        out.push(UpdateCandidate {
            // The bare name stays an alias, so that a hold on "firefox" keeps
            // working.
            id: PackageId::new(BackendKind::Apt, native).with_alias(name),
            name: name.to_string(),
            installed: Version::parse_as(installed, Scheme::Dpkg),
            available: Version::parse_as(available, Scheme::Dpkg),
            size_bytes: None,
            expected_sha256: None,
        });
    }

    out
}

/// Parse `apt-mark showhold`: one name per line, with the architecture when it
/// is not the machine's own.
fn parse_holds(stdout: &str) -> HashSet<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// A bare name holds the package on every architecture: which one is the
/// machine's own is not known here, and holding too much is the safe side.
fn is_held(holds: &HashSet<String>, id: &str) -> bool {
    let bare = id.split_once(':').map_or(id, |(name, _)| name);
    holds.contains(id) || holds.contains(bare)
}

/// The version APT would install, from `apt-cache policy`.
fn parse_policy_candidate(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("Candidate:"))
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "(none)")
        .map(str::to_string)
}

/// Read `<status> <version>` lines.
///
/// A removed package keeps its version in dpkg's database as long as its
/// configuration files exist, so only the status `installed` counts. A bare
/// name can match two architectures; when they differ there is no single
/// answer.
fn parse_installed(stdout: &str) -> Option<String> {
    let mut versions: Vec<&str> = stdout
        .lines()
        .filter_map(|l| l.trim().strip_prefix("installed "))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect();
    versions.dedup();
    match versions.as_slice() {
        [one] => Some((*one).to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang_backends::assert_refuses_bad_input;

    fn crlf(text: &str) -> String {
        text.replace('\n', "\r\n")
    }

    #[test]
    fn parses_a_standard_upgradable_line() {
        let out = "Listing...\n\
firefox/noble-updates 141.0+build1 amd64 [upgradable from: 140.0+build2]\n";
        for text in [out.to_string(), crlf(out)] {
            let rows = parse_upgradable(&text);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id.native, "firefox:amd64");
            assert_eq!(rows[0].id.alias.as_deref(), Some("firefox"));
            assert_eq!(rows[0].name, "firefox");
            assert_eq!(rows[0].available.raw(), "141.0+build1");
            assert_eq!(rows[0].installed.raw(), "140.0+build2");
        }
    }

    #[test]
    fn the_listing_header_and_blank_lines_are_ignored() {
        assert!(parse_upgradable("Listing...\n\n").is_empty());
        assert!(parse_upgradable("").is_empty());
    }

    #[test]
    fn lines_without_an_upgrade_marker_are_ignored() {
        // Plain `apt list` output must not be mistaken for upgradable packages.
        let out = "Listing...\nfirefox/noble,now 140.0 amd64 [installed]\n";
        assert!(parse_upgradable(out).is_empty());
    }

    #[test]
    fn two_architectures_of_a_package_are_two_packages() {
        let out = "Listing...\n\
libc6/bookworm-security 2.36-9+deb12u9 amd64 [upgradable from: 2.36-9+deb12u8]\n\
libc6/bookworm-security 2.36-9+deb12u9 i386 [upgradable from: 2.36-9+deb12u8]\n\
tzdata/bookworm-updates,bookworm-security 2025a-0+deb12u1 all [upgradable from: 2024b-0+deb12u1]\n";
        for text in [out.to_string(), crlf(out)] {
            let rows = parse_upgradable(&text);
            let ids: Vec<&str> = rows.iter().map(|r| r.id.native.as_str()).collect();
            assert_eq!(ids, ["libc6:amd64", "libc6:i386", "tzdata"]);
            // A security update is an upgrade by dpkg's rules.
            assert!(rows.iter().all(|r| r.installed.is_upgrade_to(&r.available)));
        }
    }

    #[test]
    fn epoch_versions_survive_parsing() {
        // Debian epochs contain a colon and must not be truncated.
        let out = "Listing...\n\
tzdata/noble 2:2024a-1 all [upgradable from: 2:2023d-1]\n";
        let rows = parse_upgradable(out);
        assert_eq!(rows[0].available.raw(), "2:2024a-1");
        assert_eq!(rows[0].installed.raw(), "2:2023d-1");
        assert!(rows[0].installed.is_upgrade_to(&rows[0].available));
    }

    #[test]
    fn holds_cover_the_package_they_name() {
        for text in ["docker-ce\nlibc6:i386\n", "docker-ce\r\nlibc6:i386\r\n"] {
            let holds = parse_holds(text);
            assert_eq!(holds.len(), 2);
            assert!(is_held(&holds, "docker-ce:amd64"));
            assert!(is_held(&holds, "docker-ce"));
            assert!(is_held(&holds, "libc6:i386"));
            assert!(!is_held(&holds, "libc6:amd64"));
        }
        assert!(parse_holds("\n").is_empty());
    }

    #[test]
    fn the_candidate_is_read_from_apt_cache_policy() {
        // https://manpages.debian.org/apt-cache, format of `policy`.
        let out = "\
openssl:
  Installed: 3.0.14-1~deb12u2
  Candidate: 3.0.15-1~deb12u1
  Version table:
     3.0.15-1~deb12u1 500
        500 http://deb.debian.org/debian-security bookworm-security/main amd64 Packages
 *** 3.0.14-1~deb12u2 100
        100 /var/lib/dpkg/status
";
        for text in [out.to_string(), crlf(out)] {
            assert_eq!(
                parse_policy_candidate(&text).as_deref(),
                Some("3.0.15-1~deb12u1")
            );
        }
        assert_eq!(
            parse_policy_candidate("x:\n  Installed: (none)\n  Candidate: (none)\n"),
            None
        );
        assert_eq!(parse_policy_candidate(""), None);
    }

    #[test]
    fn a_moved_candidate_is_refused() {
        let rows = parse_upgradable(
            "openssl/x 3.0.15-1~deb12u1 amd64 [upgradable from: 3.0.14-1~deb12u2]",
        );
        assert!(guard::same_target(&rows[0], Some("3.0.15-1~deb12u1")).is_ok());
        assert!(guard::same_target(&rows[0], Some("3.0.16-1~deb12u1")).is_err());
        assert!(guard::same_target(&rows[0], None).is_err());
    }

    #[test]
    fn a_removed_package_is_not_installed() {
        assert_eq!(
            parse_installed("installed 3.0.15-1~deb12u1\n").as_deref(),
            Some("3.0.15-1~deb12u1")
        );
        assert_eq!(
            parse_installed("installed 3.0.15-1~deb12u1\r\n").as_deref(),
            Some("3.0.15-1~deb12u1")
        );
        assert_eq!(parse_installed("config-files 3.0.15-1~deb12u1\n"), None);
        assert_eq!(
            parse_installed("installed 1.0\ninstalled 1.0\n").as_deref(),
            Some("1.0")
        );
        assert_eq!(parse_installed("installed 1.0\ninstalled 1.1\n"), None);
        assert_eq!(parse_installed(""), None);
    }

    #[test]
    fn the_install_pins_waits_for_the_lock_and_asks_nothing() {
        let args = install_args("libc6:i386", "2.36-9+deb12u9");
        assert_eq!(args[0], "DEBIAN_FRONTEND=noninteractive");
        assert_eq!(args[1], "apt-get");
        assert!(args.iter().any(|a| a == "--only-upgrade"));
        assert!(args.iter().any(|a| a == "--no-remove"));
        assert!(args.iter().any(|a| a == "DPkg::Lock::Timeout=60"));
        assert_eq!(
            args[args.len() - 2..],
            ["--".to_string(), "libc6:i386=2.36-9+deb12u9".to_string()]
        );
        assert!(
            !args
                .iter()
                .any(|a| a == "--force-yes" || a.starts_with("--allow-"))
        );
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(&AptBackend::new(), "firefox:amd64", Some(Scheme::Dpkg)).await;
    }
}
