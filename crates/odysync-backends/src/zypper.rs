//! Zypper backend for SUSE/openSUSE.
//!
//! Zypper verifies package signatures at the repository level. The table that
//! `list-updates` prints is made for people, so the XML output is read instead.
//!
//! Commands used:
//!   - `zypper --non-interactive --xmlout list-updates`: list available updates
//!   - `zypper --non-interactive install name.arch=edition`: zypper's way to
//!     update to one exact version
//!   - `rpm -q`: read the installed version

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

pub struct ZypperBackend;

impl ZypperBackend {
    pub fn new() -> Self {
        Self
    }

    async fn list_updates() -> Result<Vec<UpdateCandidate>> {
        // Refreshing the repositories needs root; without it zypper would stop
        // instead of reading what it already has.
        let mut args = vec!["--non-interactive"];
        if !odysync_core::platform::is_elevated() {
            args.push("--no-refresh");
        }
        args.extend(["--xmlout", "list-updates"]);
        let out = proc::run("zypper", &args, SCAN_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("zypper list-updates", out));
        }
        parse_list_updates(&out.stdout)
    }
}

impl Default for ZypperBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for ZypperBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Zypper
    }

    fn display_name(&self) -> &str {
        "Zypper"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("zypper", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        Self::list_updates().await
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;

        let offered = Self::list_updates().await?;
        guard::same_target(
            candidate,
            offered
                .iter()
                .find(|c| c.id == candidate.id)
                .map(|c| c.available.raw()),
        )?;
        // `install` also installs what is missing, so this check is what makes
        // it an update.
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;

        let args = install_args(&candidate.id.native, candidate.available.raw());
        let out = proc::run("zypper", &args, INSTALL_TIMEOUT).await?;
        if is_success(out.code) {
            Ok(())
        } else {
            Err(guard::failed(
                format!("zypper install {}", candidate.id.native),
                out,
            ))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        crate::dnf::rpm_installed(&candidate.id.native).await
    }
}

/// Capability syntax from zypper(8): `NAME[.ARCH][OP EDITION]`, for example
/// `zypper=0.8.8-2`.
///
/// Without `--oldpackage` zypper refuses to go back, and in non-interactive
/// mode it refuses whatever it would have to ask: a licence, a vendor change,
/// a removal.
fn install_args(name_arch: &str, edition: &str) -> Vec<String> {
    [
        "--non-interactive",
        "install",
        // Nothing beyond what the update needs.
        "--no-recommends",
        "--",
    ]
    .into_iter()
    .map(str::to_string)
    .chain([format!("{name_arch}={edition}")])
    .collect()
}

/// 102 and 103 say that the update is in place and a restart is needed.
///
/// <https://github.com/openSUSE/zypper/blob/master/doc/zypper.8.txt>
fn is_success(code: i32) -> bool {
    matches!(code, 0 | 102 | 103)
}

/// Parse `zypper --xmlout list-updates`.
///
/// The update's attributes are written by `xmlPrintOtherUpdateOn`:
/// `edition-old` appears only when the version changes.
fn parse_list_updates(stdout: &str) -> Result<Vec<UpdateCandidate>> {
    let doc = roxmltree::Document::parse(stdout.trim())
        .map_err(|e| Error::parse("zypper --xmlout list-updates", e.to_string()))?;

    Ok(doc
        .descendants()
        .filter(|n| n.has_tag_name("update") && n.attribute("kind") == Some("package"))
        .filter_map(|n| {
            let name = n.attribute("name")?;
            let arch = n.attribute("arch")?;
            let edition = n.attribute("edition")?;
            let installed = n.attribute("edition-old")?;
            Some(UpdateCandidate {
                // The bare name stays an alias, so that a hold on "firefox"
                // keeps working.
                id: PackageId::new(BackendKind::Zypper, format!("{name}.{arch}")).with_alias(name),
                name: name.to_string(),
                installed: Version::parse_as(installed, Scheme::Rpm),
                available: Version::parse_as(edition, Scheme::Rpm),
                size_bytes: None,
                expected_sha256: None,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang_backends::assert_refuses_bad_input;

    /// Built from zypper's own writer, which is the only documentation of the
    /// attributes:
    /// https://github.com/openSUSE/zypper/blob/master/src/update.cc
    const XML: &str = r#"<?xml version='1.0'?>
<stream>
<message type="info">Loading repository data...</message>
<message type="info">Reading installed packages...</message>
<update-status version="0.6">
<update-list>
<update kind="package" name="MozillaFirefox" edition="141.0-1.1" arch="x86_64" edition-old="140.0-1.1"><summary>Mozilla Firefox Web Browser</summary><description>A &amp; B</description><license></license><source url="https://download.opensuse.org/update/" alias="repo-update"/></update>
<update kind="package" name="libopenssl3" edition="1:3.2.2-6.1" arch="x86_64" edition-old="1:3.2.1-2.1"><summary>OpenSSL</summary></update>
<update kind="package" name="archchange" edition="1.0-1" arch="noarch" arch-old="x86_64"><summary>same version</summary></update>
<update kind="patch" name="openSUSE-2025-1" edition="1" arch="noarch"><summary>patch</summary></update>
</update-list>
</update-status>
</stream>
"#;

    #[test]
    fn parses_zypper_xml() {
        for text in [XML.to_string(), XML.replace('\n', "\r\n")] {
            let found = parse_list_updates(&text).unwrap();
            assert_eq!(found.len(), 2);
            assert_eq!(found[0].id.native, "MozillaFirefox.x86_64");
            assert_eq!(found[0].id.alias.as_deref(), Some("MozillaFirefox"));
            assert_eq!(found[0].installed.raw(), "140.0-1.1");
            assert_eq!(found[0].available.raw(), "141.0-1.1");
            assert_eq!(found[1].available.raw(), "1:3.2.2-6.1");
            assert!(
                found
                    .iter()
                    .all(|c| c.installed.is_upgrade_to(&c.available))
            );
        }
    }

    #[test]
    fn an_empty_list_is_no_updates_and_broken_xml_is_an_error() {
        let none = "<?xml version='1.0'?>\n<stream>\n<update-status version=\"0.6\">\n\
                    <update-list>\n</update-list>\n</update-status>\n</stream>\n";
        assert!(parse_list_updates(none).unwrap().is_empty());
        assert!(parse_list_updates("").is_err());
        assert!(parse_list_updates("S | Repository | Name\n").is_err());
    }

    #[test]
    fn the_update_is_pinned_and_never_goes_back() {
        let args = install_args("libopenssl3.x86_64", "1:3.2.2-6.1");
        assert_eq!(
            args,
            [
                "--non-interactive",
                "install",
                "--no-recommends",
                "--",
                "libopenssl3.x86_64=1:3.2.2-6.1"
            ]
        );
        assert!(!args.iter().any(|a| a == "--oldpackage" || a == "--force"));
    }

    #[test]
    fn a_needed_restart_is_a_success() {
        assert!(is_success(0));
        assert!(is_success(102));
        assert!(is_success(103));
        for code in [-1, 1, 4, 8, 104, 105, 106, 107] {
            assert!(!is_success(code), "{code}");
        }
    }

    #[test]
    fn a_moved_target_is_refused() {
        let found = parse_list_updates(XML).unwrap();
        assert!(guard::same_target(&found[0], Some("141.0-1.1")).is_ok());
        assert!(guard::same_target(&found[0], Some("141.0-2.1")).is_err());
        assert!(guard::same_target(&found[0], None).is_err());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = ZypperBackend::new();
        assert_eq!(b.kind(), BackendKind::Zypper);
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(
            &ZypperBackend::new(),
            "MozillaFirefox.x86_64",
            Some(Scheme::Rpm),
        )
        .await;
    }
}
