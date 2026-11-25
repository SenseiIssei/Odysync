//! DNF backend for Fedora, RHEL, Rocky, and AlmaLinux.
//!
//! DNF verifies package signatures at the repository level, so integrity is
//! handled below us. We pin the exact target with a full NEVRA
//! (`name-epoch:version-release.arch`) to prevent DNF from resolving "latest"
//! on its own.
//!
//! Commands used:
//!   - `dnf check-update --refresh -q`: list available updates (exit 100 = updates exist)
//!   - `rpm -qa --qf '%{NAME}.%{ARCH} %{EVR}\n'`: what is installed, which check-update does not say
//!   - `dnf upgrade -y NEVRA`: update one package; unlike `install` it refuses a missing one

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;
use odysync_core::version_native::Scheme;

use crate::guard;

const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

pub struct DnfBackend;

impl DnfBackend {
    pub fn new() -> Self {
        Self
    }

    /// Run `dnf check-update` and read its table.
    async fn check_update(args: &[&str]) -> Result<Vec<(String, String)>> {
        let mut all = vec!["check-update", "-q"];
        all.extend(args);
        let out = proc::run("dnf", &all, SCAN_TIMEOUT).await?;
        // Exit 100 = updates available, 0 = no updates.
        if out.code != 0 && out.code != 100 {
            return Err(guard::failed("dnf check-update", out));
        }
        Ok(parse_check_update(&out.stdout))
    }
}

impl Default for DnfBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for DnfBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Dnf
    }

    fn display_name(&self) -> &str {
        "DNF"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("dnf", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let updates = Self::check_update(&["--refresh"]).await?;
        if updates.is_empty() {
            return Ok(Vec::new());
        }
        let out = proc::run(
            "rpm",
            &["-qa", "--qf", "%{NAME}.%{ARCH} %{EVR}\\n"],
            QUERY_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(guard::failed("rpm -qa", out));
        }
        Ok(join(updates, &parse_rpm_list(&out.stdout)))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        let id = candidate.id.native.as_str();
        let Some(spec) = nevra(id, candidate.available.raw()) else {
            return Err(Error::invalid_package_id(
                id,
                "expected name.arch; scan again",
            ));
        };

        let offered = Self::check_update(&[id]).await?;
        guard::same_target(
            candidate,
            offered
                .iter()
                .find(|(na, _)| na == id)
                .map(|(_, evr)| evr.as_str()),
        )?;
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;

        let out = proc::run("dnf", &["upgrade", "-y", &spec], INSTALL_TIMEOUT).await?;
        if out.success() {
            Ok(())
        } else {
            Err(guard::failed(format!("dnf upgrade {spec}"), out))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        rpm_installed(&candidate.id.native).await
    }
}

/// The installed version of `name.arch`, the newest one when several are
/// installed side by side, as kernels are. Shared with zypper.
pub(crate) async fn rpm_installed(name_arch: &str) -> Result<Option<String>> {
    let out = proc::run(
        "rpm",
        &["-q", "--qf", "%{NAME}.%{ARCH} %{EVR}\\n", name_arch],
        QUERY_TIMEOUT,
    )
    .await?;
    if !out.success() {
        return Ok(None);
    }
    Ok(parse_rpm_list(&out.stdout).remove(name_arch))
}

/// Whether `arch` is one an RPM can have.
fn is_arch(arch: &str) -> bool {
    matches!(
        arch,
        "noarch"
            | "x86_64"
            | "x86_64_v2"
            | "x86_64_v3"
            | "x86_64_v4"
            | "i386"
            | "i486"
            | "i586"
            | "i686"
            | "aarch64"
            | "armv7hl"
            | "armv7hnl"
            | "armv6hl"
            | "ppc64"
            | "ppc64le"
            | "s390x"
            | "riscv64"
            | "loongarch64"
    )
}

/// `openssl-libs.x86_64` + `1:3.2.2-6.fc40` = `openssl-libs-1:3.2.2-6.fc40.x86_64`.
fn nevra(name_arch: &str, evr: &str) -> Option<String> {
    let (name, arch) = name_arch.rsplit_once('.')?;
    (!name.is_empty() && is_arch(arch)).then(|| format!("{name}-{evr}.{arch}"))
}

/// Parse `dnf check-update -q`, from dnf 4 and dnf 5: `name.arch` and EVR.
///
/// ```text
/// Upgrades
/// openssl-libs.x86_64    1:3.2.2-6.fc40    updates
///
/// Obsoleting packages
/// new-name.noarch        2.0-1.fc40        updates
///     old-name.noarch    1.0-1.fc40        @updates
/// ```
///
/// The heading in the middle is written "Obsoleting Packages" by dnf 4, which
/// has no first heading. What follows replaces packages under another name and
/// is not an update of the one installed, so reading stops there. Dnf 4 also
/// wraps a name that is too long for its column onto a line of its own.
fn parse_check_update(stdout: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    let mut wrapped: Option<String> = None;

    for line in stdout.lines() {
        if line.trim().eq_ignore_ascii_case("obsoleting packages") {
            break;
        }
        let indented = line.starts_with(char::is_whitespace);
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (name_arch, evr) = match (fields.as_slice(), wrapped.take()) {
            ([name, evr, _repo], _) if !indented => (name.to_string(), *evr),
            ([evr, _repo], Some(name)) if indented => (name, *evr),
            ([name], _) if !indented => {
                wrapped = Some(name.to_string());
                continue;
            }
            _ => continue,
        };
        let known_arch = name_arch
            .rsplit_once('.')
            .is_some_and(|(name, arch)| !name.is_empty() && is_arch(arch));
        // Every RPM has a release; prose does not look like one.
        if known_arch && evr.contains('-') && Scheme::Rpm.accepts(evr) {
            keep_newest(&mut found, name_arch, evr);
        }
    }

    found
}

/// Add a pair, or raise the version of the one that is already there.
fn keep_newest(list: &mut Vec<(String, String)>, name_arch: String, evr: &str) {
    match list.iter_mut().find(|(na, _)| *na == name_arch) {
        Some((_, old)) => {
            if Scheme::Rpm.compare(old, evr) == Some(std::cmp::Ordering::Less) {
                *old = evr.to_string();
            }
        }
        None => list.push((name_arch, evr.to_string())),
    }
}

/// Parse `%{NAME}.%{ARCH} %{EVR}` lines, keeping the newest of each.
fn parse_rpm_list(stdout: &str) -> HashMap<String, String> {
    let mut found: HashMap<String, String> = HashMap::new();
    for line in stdout.lines() {
        let mut fields = line.split_whitespace();
        let (Some(na), Some(evr), None) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        if !Scheme::Rpm.accepts(evr) {
            continue;
        }
        let newer = found
            .get(na)
            .is_none_or(|old| Scheme::Rpm.compare(old, evr) == Some(std::cmp::Ordering::Less));
        if newer {
            found.insert(na.to_string(), evr.to_string());
        }
    }
    found
}

/// Put the installed version next to the offered one. An offer for a package
/// that is not installed under that name is not an update.
fn join(
    updates: Vec<(String, String)>,
    installed: &HashMap<String, String>,
) -> Vec<UpdateCandidate> {
    updates
        .into_iter()
        .filter_map(|(name_arch, evr)| {
            let current = installed.get(&name_arch)?;
            let name = name_arch.rsplit_once('.')?.0.to_string();
            Some(UpdateCandidate {
                // The bare name stays an alias, so that a hold on "firefox"
                // keeps working.
                id: PackageId::new(BackendKind::Dnf, name_arch.as_str()).with_alias(name.as_str()),
                name,
                installed: Version::parse_as(current, Scheme::Rpm),
                available: Version::parse_as(&evr, Scheme::Rpm),
                size_bytes: None,
                expected_sha256: None,
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

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    /// Layout from dnf 4's `output.py` (`simpleList`, `listPkgs`):
    /// https://github.com/rpm-software-management/dnf/blob/master/dnf/cli/output.py
    const DNF4: &str = "
firefox.x86_64                       141.0-1.fc40                        updates
openssl-libs.x86_64                  1:3.2.2-6.fc40                      updates
kernel.x86_64                        6.8.0-50.fc40                       updates
python3-a-package-with-a-really-long-name.noarch
                                     2.0-3.fc40                          updates
Obsoleting Packages
new-name.noarch                      2.0-1.fc40                          updates
    old-name.noarch                  1.0-1.fc40                          @updates
Security: kernel-core-6.8.0-50.fc40.x86_64 is an installed security update
";

    /// Layout from dnf 5's `package_list_sections.cpp`:
    /// https://github.com/rpm-software-management/dnf5/blob/main/libdnf5-cli/output/package_list_sections.cpp
    const DNF5: &str = "Upgrades
firefox.x86_64      141.0-1.fc43   updates
openssl-libs.x86_64 1:3.2.2-6.fc43 updates
openssl-libs.i686   1:3.2.2-6.fc43 updates

Obsoleting packages
new-name.noarch     2.0-1.fc43     updates
    old-name.noarch 1.0-1.fc43     updates

";

    #[test]
    fn parses_dnf4_output_including_wrapped_names() {
        for text in [DNF4.to_string(), crlf(DNF4)] {
            let found = parse_check_update(&text);
            assert_eq!(
                found,
                pairs(&[
                    ("firefox.x86_64", "141.0-1.fc40"),
                    ("openssl-libs.x86_64", "1:3.2.2-6.fc40"),
                    ("kernel.x86_64", "6.8.0-50.fc40"),
                    (
                        "python3-a-package-with-a-really-long-name.noarch",
                        "2.0-3.fc40"
                    ),
                ])
            );
        }
    }

    #[test]
    fn parses_dnf5_output() {
        for text in [DNF5.to_string(), crlf(DNF5)] {
            let found = parse_check_update(&text);
            assert_eq!(
                found,
                pairs(&[
                    ("firefox.x86_64", "141.0-1.fc43"),
                    ("openssl-libs.x86_64", "1:3.2.2-6.fc43"),
                    ("openssl-libs.i686", "1:3.2.2-6.fc43"),
                ])
            );
        }
    }

    #[test]
    fn skips_non_package_lines() {
        let output = "\
Last metadata expiration check: 0:01:23 ago
# comment
3 updates can be installed
firefox.x86_64            141.0-1.fc40       updates
";
        assert_eq!(
            parse_check_update(output),
            pairs(&[("firefox.x86_64", "141.0-1.fc40")])
        );
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_check_update("").is_empty());
    }

    #[test]
    fn the_installed_version_comes_from_rpm() {
        let rpm = "\
firefox.x86_64 140.0-2.fc40
openssl-libs.x86_64 1:3.2.1-2.fc40
kernel.x86_64 6.8.0-40.fc40
kernel.x86_64 6.8.0-9.fc40
gpg-pubkey.(none) a15b79cc-63d04c2c
";
        for text in [rpm.to_string(), crlf(rpm)] {
            let installed = parse_rpm_list(&text);
            // The newest of the kernels that are installed side by side.
            assert_eq!(installed["kernel.x86_64"], "6.8.0-40.fc40");
            let found = join(parse_check_update(DNF4), &installed);
            let ids: Vec<&str> = found.iter().map(|c| c.id.native.as_str()).collect();
            // The package that is not installed is not an update.
            assert_eq!(
                ids,
                ["firefox.x86_64", "openssl-libs.x86_64", "kernel.x86_64"]
            );
            assert_eq!(found[1].installed.raw(), "1:3.2.1-2.fc40");
            assert_eq!(found[1].id.alias.as_deref(), Some("openssl-libs"));
            assert!(
                found
                    .iter()
                    .all(|c| c.installed.is_upgrade_to(&c.available))
            );
        }
    }

    #[test]
    fn the_spec_names_exactly_one_build() {
        assert_eq!(
            nevra("openssl-libs.x86_64", "1:3.2.2-6.fc40").as_deref(),
            Some("openssl-libs-1:3.2.2-6.fc40.x86_64")
        );
        assert_eq!(
            nevra("java-21-openjdk.1.noarch", "1-1").as_deref(),
            Some("java-21-openjdk.1-1-1.noarch")
        );
        assert_eq!(nevra("firefox", "1-1"), None);
        assert_eq!(nevra("firefox.banana", "1-1"), None);
    }

    #[test]
    fn a_moved_target_is_refused() {
        let installed = parse_rpm_list("firefox.x86_64 140.0-2.fc40\n");
        let found = join(pairs(&[("firefox.x86_64", "141.0-1.fc40")]), &installed);
        assert!(guard::same_target(&found[0], Some("141.0-1.fc40")).is_ok());
        assert!(guard::same_target(&found[0], Some("141.0-2.fc40")).is_err());
        assert!(guard::still_older(&found[0], Some("141.0-1.fc40")).is_err());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = DnfBackend::new();
        assert_eq!(b.kind(), BackendKind::Dnf);
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(&DnfBackend::new(), "firefox.x86_64", Some(Scheme::Rpm)).await;
    }
}
