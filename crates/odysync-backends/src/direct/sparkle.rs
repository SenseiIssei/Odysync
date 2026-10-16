//! Sparkle appcasts — how most Mac apps outside the App Store update.
//!
//! An app that embeds Sparkle names its update feed in its own `Info.plist`
//! (`SUFeedURL`) and, for any app built in the last several years, the
//! Ed25519 public key its updates are signed with (`SUPublicEDKey`). That key
//! comes from the copy of the app already on the disk, so the trust anchor is
//! the app the user installed — not the feed, not Odysync.
//!
//! This module is the pure half: reading a feed, choosing the release to
//! offer, and checking an archive's signature. It runs in tests everywhere.

use base64::Engine as _;

/// One installable release from an appcast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppcastRelease {
    /// `sparkle:version` — the build number Sparkle compares with
    /// `CFBundleVersion`.
    pub build: String,
    /// `sparkle:shortVersionString` — what the user sees.
    pub short_version: Option<String>,
    pub url: String,
    pub ed_signature: Option<String>,
    pub length: Option<u64>,
    pub minimum_system_version: Option<String>,
}

impl AppcastRelease {
    /// The version to show and compare: the human one when there is one.
    pub fn display_version(&self) -> &str {
        self.short_version.as_deref().unwrap_or(&self.build)
    }
}

const SPARKLE_NS: &str = "http://www.andymatuschak.org/xml-namespaces/sparkle";

/// Every full-update release in an appcast, in feed order.
///
/// Skipped, deliberately:
/// * items on a named `sparkle:channel` — those are betas the user would have
///   opted into inside the app, which Odysync cannot see;
/// * delta updates — they patch one specific build in place and need
///   Sparkle's own machinery to apply;
/// * enclosures that are not HTTPS.
pub fn parse_appcast(xml: &str) -> Vec<AppcastRelease> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let sparkle = |node: roxmltree::Node, name: &str| -> Option<String> {
        node.children()
            .find(|c| c.tag_name().name() == name && c.tag_name().namespace() == Some(SPARKLE_NS))
            .and_then(|c| c.text())
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };

    doc.descendants()
        .filter(|n| n.has_tag_name("item"))
        .filter_map(|item| {
            if sparkle(item, "channel").is_some() {
                return None;
            }
            // The item's own enclosure, not one inside `sparkle:deltas`.
            let enclosure = item.children().find(|c| c.has_tag_name("enclosure"))?;
            let attr = |name: &str| {
                enclosure
                    .attribute((SPARKLE_NS, name))
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            };
            let url = enclosure.attribute("url")?.trim().to_string();
            if !url.starts_with("https://") {
                return None;
            }
            let build = sparkle(item, "version").or_else(|| attr("version"))?;
            Some(AppcastRelease {
                build,
                short_version: sparkle(item, "shortVersionString")
                    .or_else(|| attr("shortVersionString")),
                url,
                ed_signature: attr("edSignature"),
                length: enclosure
                    .attribute("length")
                    .and_then(|l| l.trim().parse().ok())
                    .filter(|l| *l > 0),
                minimum_system_version: sparkle(item, "minimumSystemVersion"),
            })
        })
        .collect()
}

/// The newest release this Mac can run, judged by build number like Sparkle.
pub fn newest_compatible(releases: &[AppcastRelease], os_version: &str) -> Option<AppcastRelease> {
    use odysync_core::version::Version;
    let os = Version::parse(os_version);
    releases
        .iter()
        .filter(|r| match &r.minimum_system_version {
            None => true,
            Some(min) => matches!(
                Version::parse(min).compare(&os),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            ),
        })
        .max_by(|a, b| {
            Version::parse(&a.build)
                .compare(&Version::parse(&b.build))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
}

/// Which two version strings to compare for an installed app and a release.
///
/// Sparkle itself compares build numbers (`CFBundleVersion` against
/// `sparkle:version`). Build numbers are what decide, but they make poor
/// labels — "62 → 64" tells a user nothing — so when both sides carry a
/// human version and those differ, the human versions are used. When the
/// human versions are equal (a rebuild), the build numbers are.
pub fn comparable_versions(
    installed_short: Option<&str>,
    installed_build: &str,
    release: &AppcastRelease,
) -> (String, String) {
    match (installed_short, release.short_version.as_deref()) {
        (Some(a), Some(b)) if a.trim() != b.trim() => (a.trim().into(), b.trim().into()),
        _ => (
            installed_build.trim().to_string(),
            release.build.trim().to_string(),
        ),
    }
}

/// Check a Sparkle EdDSA signature over the whole archive.
///
/// Both values are base64, as Sparkle writes them: a 32-byte Ed25519 public
/// key and a 64-byte signature over the archive's raw bytes.
pub fn verify_ed_signature(public_key_b64: &str, signature_b64: &str, data: &[u8]) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let engine = base64::engine::general_purpose::STANDARD;
    let Ok(key) = engine.decode(public_key_b64.trim()) else {
        return false;
    };
    let Ok(sig) = engine.decode(signature_b64.trim()) else {
        return false;
    };
    let (Ok(key), Ok(sig)) = (<[u8; 32]>::try_from(key), <[u8; 64]>::try_from(sig)) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&key) else {
        return false;
    };
    key.verify(data, &Signature::from_bytes(&sig)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from Rectangle's real appcast: a full release with deltas
    /// nested beside it, and an older release.
    const RECTANGLE: &str = r#"<?xml version="1.0" standalone="yes"?>
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle" version="2.0">
  <channel>
    <title>Rectangle</title>
    <item>
      <title>2.0.3</title>
      <sparkle:version>110</sparkle:version>
      <sparkle:shortVersionString>2.0.3</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure url="https://github.com/rxhanson/Rectangle/releases/download/v2.0.3/Rectangle2.0.3.dmg" length="4730793" type="application/octet-stream" sparkle:edSignature="T+3C4wzJesAJxkWyu+9QxAWPT1ntX+ifY4z2MqvpIxWC89COxIKzypWsL8h39aZjCHxDyBr7ZzauFcpQN3pDCQ=="/>
      <sparkle:deltas>
        <enclosure url="https://github.com/rxhanson/Rectangle/releases/download/v2.0.3/Rectangle110-109.delta" sparkle:deltaFrom="109" length="363770" sparkle:edSignature="r5WaaefcFdQwCxXEhFa8Nml5ks705DSlFUlCnKccs4+HUhUxIN1E71/r8uug4FCHidpKhqgle4UZUGg6DRYuBQ=="/>
      </sparkle:deltas>
    </item>
    <item>
      <title>2.0.2</title>
      <sparkle:version>109</sparkle:version>
      <sparkle:shortVersionString>2.0.2</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure url="https://github.com/rxhanson/Rectangle/releases/download/v2.0.2/Rectangle2.0.2.dmg" length="4679140" sparkle:edSignature="/t5M83i3dHxRJ8Abov1eUCATvpIUw8UwImfJrShQkHUglP01FBQM+rt0HbTCFR/D1iD7p1Aleone/ytS7C3KCw=="/>
    </item>
    <item>
      <title>2.1.0 beta</title>
      <sparkle:version>111</sparkle:version>
      <sparkle:channel>beta</sparkle:channel>
      <enclosure url="https://example.com/beta.dmg" sparkle:edSignature="x"/>
    </item>
  </channel>
</rss>"#;

    /// Maccy's style: versions as enclosure attributes, no EdDSA signature.
    const MACCY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel><item>
    <title>2.7.1</title>
    <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
    <enclosure url="https://github.com/p0deje/Maccy/releases/download/2.7.1/Maccy.app.zip"
      sparkle:version="62" sparkle:shortVersionString="2.7.1" length="0"/>
  </item></channel>
</rss>"#;

    #[test]
    fn full_releases_are_read_and_deltas_and_betas_are_not() {
        let releases = parse_appcast(RECTANGLE);
        assert_eq!(releases.len(), 2, "{releases:?}");
        assert_eq!(releases[0].build, "110");
        assert_eq!(releases[0].short_version.as_deref(), Some("2.0.3"));
        assert!(releases[0].url.ends_with("Rectangle2.0.3.dmg"));
        assert_eq!(releases[0].length, Some(4_730_793));
        assert!(releases[0]
            .ed_signature
            .as_deref()
            .unwrap()
            .starts_with("T+3C4w"));
    }

    #[test]
    fn versions_may_live_on_the_enclosure_instead() {
        let releases = parse_appcast(MACCY);
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].build, "62");
        assert_eq!(releases[0].display_version(), "2.7.1");
        assert_eq!(releases[0].ed_signature, None);
        // `length="0"` means unknown, not empty.
        assert_eq!(releases[0].length, None);
    }

    #[test]
    fn the_newest_release_this_os_can_run_is_chosen() {
        let releases = parse_appcast(RECTANGLE);
        assert_eq!(newest_compatible(&releases, "15.1").unwrap().build, "110");
        assert_eq!(newest_compatible(&releases, "14.0").unwrap().build, "110");
        // Too old a macOS: nothing is offered rather than an app that will
        // not launch.
        assert!(newest_compatible(&releases, "13.6").is_none());
    }

    #[test]
    fn plaintext_enclosures_are_ignored() {
        let xml = MACCY.replace("https://github.com", "http://github.com");
        assert!(parse_appcast(&xml).is_empty());
        assert!(parse_appcast("not xml").is_empty());
    }

    #[test]
    fn human_versions_are_compared_when_they_differ() {
        let r = &parse_appcast(RECTANGLE)[0];
        assert_eq!(
            comparable_versions(Some("2.0.2"), "109", r),
            ("2.0.2".into(), "2.0.3".into())
        );
        // A rebuild with the same human version is decided by build number.
        assert_eq!(
            comparable_versions(Some("2.0.3"), "108", r),
            ("108".into(), "110".into())
        );
        assert_eq!(
            comparable_versions(None, "108", r),
            ("108".into(), "110".into())
        );
    }

    #[test]
    fn an_ed25519_signature_is_checked_against_the_apps_own_key() {
        use ed25519_dalek::{Signer, SigningKey};
        let engine = base64::engine::general_purpose::STANDARD;
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let public = engine.encode(key.verifying_key().to_bytes());
        let archive = b"the bytes of an update archive";
        let signature = engine.encode(key.sign(archive).to_bytes());

        assert!(verify_ed_signature(&public, &signature, archive));
        // One changed byte, a different key, or garbage: all refused.
        assert!(!verify_ed_signature(
            &public,
            &signature,
            b"the bytes of an update archivE"
        ));
        let other = engine.encode(
            SigningKey::from_bytes(&[9u8; 32])
                .verifying_key()
                .to_bytes(),
        );
        assert!(!verify_ed_signature(&other, &signature, archive));
        assert!(!verify_ed_signature("not base64!", &signature, archive));
        assert!(!verify_ed_signature(&public, "c2hvcnQ=", archive));
    }

    /// Against a real Sparkle release: Rectangle's 2.0.3 delta (360 KB),
    /// its published signature, and the public key from Rectangle's
    /// Info.plist. Proves the format matches what Sparkle's tooling emits.
    /// Network-dependent, so not part of the default run.
    #[test]
    #[ignore = "downloads a real release from GitHub"]
    fn a_real_sparkle_signature_verifies() {
        let data = reqwest::blocking_get(
            "https://github.com/rxhanson/Rectangle/releases/download/v2.0.3/Rectangle110-109.delta",
        );
        assert!(verify_ed_signature(
            "lpt9M3PhocbZ3MZiLH+crEqRfU11kfoNzGxSqiEIdvM=",
            "r5WaaefcFdQwCxXEhFa8Nml5ks705DSlFUlCnKccs4+HUhUxIN1E71/r8uug4FCHidpKhqgle4UZUGg6DRYuBQ==",
            &data
        ));
    }

    mod reqwest {
        pub fn blocking_get(url: &str) -> Vec<u8> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let client =
                    crate::direct::http::client(None, std::time::Duration::from_secs(60)).unwrap();
                client
                    .get(url)
                    .send()
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap()
                    .to_vec()
            })
        }
    }
}
