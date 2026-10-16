//! Whether a newer Odysync exists — and deliberately not installing it.
//!
//! ## Why this checks but does not install
//!
//! `ARCHITECTURE.md` criticises v1 for installing a third-party PowerShell
//! module from PSGallery at runtime as Administrator, "trusting the
//! repository — a supply-chain hole". An unattended self-update that downloads
//! an **unsigned** executable and runs it as Administrator is the same hole
//! with our own name on it, and it would be worse: v1 at least trusted a
//! repository with review, whereas this would trust whatever answered the
//! request.
//!
//! So the order is fixed and is not a matter of taste: **code signing first,
//! automatic self-update second.** Until releases are signed
//! (`docs/SIGNING.md`), this module tells the user a release exists, shows the
//! published SHA-256, and links to it. That is genuinely most of the value —
//! nobody updates software they do not know is out of date — without opening
//! the hole.
//!
//! When signing lands, [`UpdateCheck::is_installable`] is where the decision
//! changes, and the verification it will need already exists in
//! `odysync-verify`.

use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use odysync_core::version::Version;

/// Where releases are published.
const RELEASES_API: &str = "https://api.github.com/repos/SenseiIssei/Odysync/releases/latest";

/// GitHub rejects unidentified callers, and a version here makes our traffic
/// legible in their logs if it ever misbehaves.
const USER_AGENT: &str = concat!("Odysync/", env!("CARGO_PKG_VERSION"));

const TIMEOUT: Duration = Duration::from_secs(30);

/// The result of asking whether a newer release exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheck {
    /// The version running right now.
    pub current: String,
    /// The newest published release, when one could be read.
    pub latest: Option<String>,
    /// Whether `latest` is genuinely newer, by the same version algebra the
    /// policy engine uses — not a string comparison.
    pub is_newer: bool,
    /// Human-facing page for the release.
    pub release_url: Option<String>,
    /// The published SHA-256 of the Windows installer, when the release has
    /// one, so a user can verify a manual download.
    pub installer_sha256: Option<String>,
    /// Whether Odysync would be willing to install this itself.
    pub is_installable: bool,
    /// Why not, when it is not.
    pub not_installable_reason: Option<String>,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Ask GitHub for the newest release and compare it with this build.
pub async fn check(proxy_url: Option<&str>) -> Result<UpdateCheck> {
    let current = env!("CARGO_PKG_VERSION").to_string();

    let mut builder = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT);
    if let Some(proxy) = proxy_url {
        if let Ok(proxy) = reqwest::Proxy::all(proxy) {
            builder = builder.proxy(proxy);
        }
    }
    let client = builder.build()?;

    let response = client
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;

    if !response.status().is_success() {
        anyhow::bail!(
            "GitHub returned HTTP {} for the release check",
            response.status()
        );
    }

    let release: GhRelease = response.json().await?;
    Ok(evaluate(
        &current,
        &release.tag_name,
        &release.html_url,
        &release.assets,
    ))
}

/// The pure half: decide what a release means for the running build.
fn evaluate(current: &str, tag: &str, url: &str, assets: &[GhAsset]) -> UpdateCheck {
    // Tags are `v2.1.0`; `Version::parse` strips the `v` itself, but comparing
    // the raw strings would not.
    let current_version = Version::parse(current);
    let latest_version = Version::parse(tag);

    // `is_upgrade_to` returns false when either side is unknown, which is the
    // right answer: an unparseable tag must not be announced as an upgrade.
    let is_newer = current_version.is_upgrade_to(&latest_version);

    let installer_sha256 = assets
        .iter()
        .find(|a| a.name.to_ascii_lowercase().ends_with(".exe.sha256"))
        .map(|a| a.browser_download_url.clone());

    UpdateCheck {
        current: current.to_string(),
        latest: Some(tag.to_string()),
        is_newer,
        release_url: Some(url.to_string()),
        installer_sha256,
        // Flipped on once releases are signed and the signature is checked
        // before execution. Until then, refusing is the whole point.
        is_installable: false,
        not_installable_reason: Some(
            "Odysync's own releases are not code-signed yet, so it will not download and \
             run one automatically — that would be the same supply-chain hole the v1 \
             rewrite existed to close. Install it yourself from the release page."
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> GhAsset {
        GhAsset {
            name: name.into(),
            browser_download_url: format!("https://example.invalid/{name}"),
        }
    }

    #[test]
    fn a_newer_tag_is_recognised_despite_the_v_prefix() {
        let c = evaluate("2.1.0", "v2.2.0", "https://example.invalid", &[]);
        assert!(c.is_newer);
        assert_eq!(c.latest.as_deref(), Some("v2.2.0"));
    }

    #[test]
    fn the_same_version_is_not_an_update() {
        assert!(!evaluate("2.1.0", "v2.1.0", "u", &[]).is_newer);
    }

    #[test]
    fn an_older_published_release_is_not_an_update() {
        // The case a string comparison gets wrong: "2.9.0" > "2.10.0"
        // lexically, so a naive check would offer a downgrade.
        assert!(!evaluate("2.10.0", "v2.9.0", "u", &[]).is_newer);
        assert!(evaluate("2.9.0", "v2.10.0", "u", &[]).is_newer);
    }

    #[test]
    fn an_unparseable_tag_is_never_announced_as_an_update() {
        for tag in ["nightly", "", "latest"] {
            assert!(
                !evaluate("2.1.0", tag, "u", &[]).is_newer,
                "announced {tag:?} as an update"
            );
        }
    }

    #[test]
    fn the_installer_checksum_is_surfaced_when_published() {
        let assets = [
            asset("Odysync_2.2.0_x64-setup.exe"),
            asset("Odysync_2.2.0_x64-setup.exe.sha256"),
        ];
        let c = evaluate("2.1.0", "v2.2.0", "u", &assets);
        assert!(c
            .installer_sha256
            .as_deref()
            .is_some_and(|u| u.ends_with(".exe.sha256")));
    }

    #[test]
    fn a_release_without_a_checksum_reports_none_rather_than_the_installer() {
        let assets = [asset("Odysync_2.2.0_x64-setup.exe")];
        assert_eq!(
            evaluate("2.1.0", "v2.2.0", "u", &assets).installer_sha256,
            None
        );
    }

    /// The invariant this module exists to hold. If this test ever needs
    /// changing, code signing had better have landed first.
    #[test]
    fn odysync_refuses_to_install_its_own_updates_while_releases_are_unsigned() {
        let c = evaluate("2.1.0", "v2.2.0", "u", &[]);
        assert!(!c.is_installable);
        let reason = c.not_installable_reason.expect("a refusal needs a reason");
        assert!(reason.to_lowercase().contains("signed"));
    }
}
