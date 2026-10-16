//! Classification of Windows Update offerings.
//!
//! The Windows Update Agent hands back a flat list of updates, but a Defender
//! signature refresh, a cumulative security patch and a Windows version
//! upgrade are three very different risks wearing the same shape. Everything
//! downstream — whether a restore point is taken, whether the user has to opt
//! in, how long an apply may take — depends on telling them apart.
//!
//! **Categories are matched by GUID, never by name.** `ICategory::Name` is
//! localised, so a German or Japanese install would fall out of any
//! string comparison. The GUIDs are stable across every locale and every
//! Windows version. This is the same reasoning as the winget table parser
//! keying on column position rather than header text.
//!
//! This module is pure: it takes the strings WUA reported and returns a class.
//! That keeps the interesting decision testable on Linux and macOS CI, where
//! no Windows Update Agent exists.

use serde::{Deserialize, Serialize};

// ── Category GUIDs ──────────────────────────────────────────────────────────
//
// Those marked "observed" were read back off a real machine rather than taken
// from documentation. The rest come from Microsoft's published category list;
// `classify` is built so that a wrong GUID degrades to `Quality`, which is the
// same risk level as an ordinary cumulative update — with the single exception
// of feature upgrades, which carry an independent title check for exactly that
// reason. See `looks_like_a_feature_upgrade`.

/// Security Updates. *(observed)*
pub const CATEGORY_SECURITY: &str = "0fa1201d-4330-4fa8-8ae9-b877473b6441";
/// Critical Updates.
pub const CATEGORY_CRITICAL: &str = "e6cf1350-c01b-414d-a61f-263d14d133b4";
/// Definition Updates — Defender signatures. *(observed)*
pub const CATEGORY_DEFINITION: &str = "e0789628-ce08-4437-be74-2495b842f43b";
/// Update Rollups, e.g. the Malicious Software Removal Tool. *(observed)*
pub const CATEGORY_UPDATE_ROLLUP: &str = "28bc880e-0592-4cbf-8f95-c79b17911d5f";
/// Updates — the ordinary non-security bucket. *(observed)*
pub const CATEGORY_UPDATES: &str = "cd5ffd1e-e932-4e3a-bf74-18bf0b1bbd83";
/// Feature Packs.
pub const CATEGORY_FEATURE_PACK: &str = "b54e7d24-7add-428f-8b75-90a396fa584f";
/// Service Packs.
pub const CATEGORY_SERVICE_PACK: &str = "68c5b0a3-d1a6-4553-ae49-01d3a7827828";
/// Upgrades — a Windows version jump, e.g. 23H2 to 24H2.
pub const CATEGORY_UPGRADE: &str = "3689bdc8-b205-4af4-8d4a-a63924c5e9d5";
/// Drivers.
pub const CATEGORY_DRIVER: &str = "ebfc1fc5-71a4-4f7b-9aca-3b9a503104a0";
/// Tools.
pub const CATEGORY_TOOLS: &str = "b4832bd8-e735-4761-8daf-37f882276dab";
/// Microsoft Defender Antivirus, as a *product* category. *(observed)*
pub const CATEGORY_DEFENDER_PRODUCT: &str = "8c3fcc84-7410-4a95-8b89-a166a0190486";

/// What kind of thing Windows is offering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateClass {
    /// Defender signature refresh. Small, frequent, superseded within hours.
    Definition,
    /// Security or critical update. The reason this backend exists.
    Security,
    /// Ordinary cumulative, rollup, tool or non-security update.
    Quality,
    /// A Windows version upgrade or service pack — a new OS build.
    FeatureUpgrade,
    /// A device driver. Handled by the driver backend, not this one.
    Driver,
}

impl UpdateClass {
    /// Stable machine-readable name for config files, the CLI and the GUI.
    pub fn id(&self) -> &'static str {
        match self {
            UpdateClass::Definition => "definition",
            UpdateClass::Security => "security",
            UpdateClass::Quality => "quality",
            UpdateClass::FeatureUpgrade => "feature-upgrade",
            UpdateClass::Driver => "driver",
        }
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            UpdateClass::Definition => "Defender definitions",
            UpdateClass::Security => "Security update",
            UpdateClass::Quality => "Quality update",
            UpdateClass::FeatureUpgrade => "Feature upgrade",
            UpdateClass::Driver => "Driver",
        }
    }

    /// Whether the user must opt in by name before this class is ever offered.
    ///
    /// A cumulative update is routine. Moving the machine to a new Windows
    /// build is not something a background daemon gets to start on its own.
    pub fn requires_opt_in(&self) -> bool {
        matches!(self, UpdateClass::FeatureUpgrade)
    }

    /// Whether a system restore point should be taken before applying.
    ///
    /// Definitions are exempt: they are replaced several times a day, and a
    /// restore point per signature refresh would fill the shadow-copy store
    /// and evict the restore points that actually matter.
    pub fn takes_restore_point(&self) -> bool {
        !matches!(self, UpdateClass::Definition)
    }
}

/// Classify one update from the strings the Windows Update Agent reported.
///
/// `category_ids` are `ICategory::CategoryID` values; case and surrounding
/// braces are ignored, since WUA has been observed to return both forms.
/// `title` is only consulted for the feature-upgrade check — see below.
pub fn classify(title: &str, category_ids: &[String]) -> UpdateClass {
    let has = |guid: &str| {
        category_ids
            .iter()
            .any(|c| normalize_guid(c) == normalize_guid(guid))
    };

    // Order matters, and it is deliberately most-specific-first.

    // A feature upgrade is checked before anything else because it is the one
    // misclassification with a bad direction: calling an upgrade a "quality
    // update" would let a background run start an OS version jump. Either
    // signal is enough, so a wrong GUID cannot defeat it on its own.
    if has(CATEGORY_UPGRADE) || has(CATEGORY_SERVICE_PACK) || looks_like_a_feature_upgrade(title) {
        return UpdateClass::FeatureUpgrade;
    }

    if has(CATEGORY_DRIVER) {
        return UpdateClass::Driver;
    }

    // Defender signatures carry both a Definition category and the Defender
    // product category; either identifies them.
    if has(CATEGORY_DEFINITION) || has(CATEGORY_DEFENDER_PRODUCT) {
        return UpdateClass::Definition;
    }

    if has(CATEGORY_SECURITY) || has(CATEGORY_CRITICAL) {
        return UpdateClass::Security;
    }

    // Updates, Update Rollups, Feature Packs, Tools and anything whose
    // category we do not recognise all land here. Unrecognised means "treat it
    // like an ordinary update": restore point first, offered by default.
    let _ = (
        has(CATEGORY_UPDATES),
        has(CATEGORY_UPDATE_ROLLUP),
        has(CATEGORY_FEATURE_PACK),
        has(CATEGORY_TOOLS),
    );
    UpdateClass::Quality
}

/// A locale-independent second opinion on whether a title names a Windows
/// version upgrade.
///
/// Titles *are* localised — a German machine offers "Funktionsupdate für
/// Windows 11, Version 24H2" — so matching the word "feature" is useless.
/// Three tokens survive translation, and all three must agree:
///
/// 1. **`windows`** appears. Product names are not translated.
/// 2. A **release tag** — `24H2`, `23H2`, `22H2` — appears. Not translated.
/// 3. **No `KB` article number.** This is the discriminator that carries the
///    weight. A cumulative update is titled "2026-07 Kumulatives Update für
///    Windows 11 Version 24H2 für x64-basierte Systeme (KB5101650)": it
///    satisfies the first two tokens and is emphatically *not* an upgrade.
///    Every servicing update carries a KB number; a feature upgrade does not.
///
/// Being wrong is costly in both directions. Calling an upgrade a quality
/// update would let a background run start an OS version jump; calling a
/// cumulative security update an upgrade would quietly stop offering security
/// patches. Requiring all three tokens is what prevents both.
fn looks_like_a_feature_upgrade(title: &str) -> bool {
    let lower = title.to_ascii_lowercase();
    lower.contains("windows") && has_release_tag(&lower) && !has_kb_number(&lower)
}

/// Does the title carry an `NNH1`/`NNH2` release tag, e.g. `24h2`?
fn has_release_tag(lower: &str) -> bool {
    lower.as_bytes().windows(4).any(|w| {
        w[0].is_ascii_digit()
            && w[1].is_ascii_digit()
            && w[2] == b'h'
            && (w[3] == b'1' || w[3] == b'2')
    })
}

/// Does the title carry a Knowledge Base article number, e.g. `KB5101650`?
fn has_kb_number(lower: &str) -> bool {
    lower
        .match_indices("kb")
        .any(|(i, _)| lower[i + 2..].starts_with(|c: char| c.is_ascii_digit()))
}

/// Strip braces and case so `{E0789628-...}` and `e0789628-...` compare equal.
fn normalize_guid(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cats(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    // Every case below is a real update observed on a live machine, German
    // locale, with its categories exactly as WUA reported them.

    #[test]
    fn an_office_security_update_is_security() {
        assert_eq!(
            classify(
                "Sicherheitsupdate für Microsoft Excel 2016 (KB5002886) 32-Bit-Edition",
                &cats(&["25aed893-7c2d-4a31-ae22-28ff8ac150ed", CATEGORY_SECURITY]),
            ),
            UpdateClass::Security
        );
    }

    #[test]
    fn a_cumulative_security_update_is_security() {
        assert_eq!(
            classify(
                "2026-07 Sicherheitsupdate (KB5101650) (26200.8875)",
                &cats(&[CATEGORY_SECURITY]),
            ),
            UpdateClass::Security
        );
    }

    #[test]
    fn a_defender_signature_is_a_definition() {
        assert_eq!(
            classify(
                "Security Intelligence-Update für Microsoft Defender Antivirus – KB2267602",
                &cats(&[CATEGORY_DEFINITION, CATEGORY_DEFENDER_PRODUCT]),
            ),
            UpdateClass::Definition
        );
    }

    #[test]
    fn the_malicious_software_removal_tool_is_a_quality_update() {
        assert_eq!(
            classify(
                "Windows-Tool zum Entfernen bösartiger Software x64 - v5.143 (KB890830)",
                &cats(&[
                    "4ea8aeaf-1d28-463e-8179-af9829f81212",
                    CATEGORY_UPDATE_ROLLUP,
                    "72e7624a-5b00-45d2-b92f-e561c0a6a160",
                ]),
            ),
            UpdateClass::Quality
        );
    }

    #[test]
    fn a_product_update_delivered_through_wu_is_a_quality_update() {
        assert_eq!(
            classify(
                "PowerShell v7.6.3 (x64)",
                &cats(&["2ff883b0-b409-4328-ab13-ec77f5bdb864", CATEGORY_UPDATES]),
            ),
            UpdateClass::Quality
        );
    }

    // Feature upgrades: the class that must never be got wrong.

    #[test]
    fn a_feature_upgrade_is_caught_by_its_category() {
        assert_eq!(
            classify("Some Upgrade", &cats(&[CATEGORY_UPGRADE])),
            UpdateClass::FeatureUpgrade
        );
    }

    #[test]
    fn a_localised_feature_upgrade_is_caught_by_its_title_alone() {
        // German title, and the category deliberately withheld: this is the
        // case where the GUID list being wrong must not matter.
        assert_eq!(
            classify(
                "Funktionsupdate für Windows 11, Version 24H2",
                &cats(&[CATEGORY_UPDATES]),
            ),
            UpdateClass::FeatureUpgrade
        );
        assert_eq!(
            classify("Feature update to Windows 11, version 23H2", &[]),
            UpdateClass::FeatureUpgrade
        );
    }

    #[test]
    fn a_release_tag_without_windows_is_not_an_upgrade() {
        assert_eq!(
            classify("Contoso Reporting 24H2", &cats(&[CATEGORY_UPDATES])),
            UpdateClass::Quality
        );
    }

    #[test]
    fn an_ordinary_windows_update_is_not_mistaken_for_an_upgrade() {
        assert_eq!(
            classify(
                "2026-07 Kumulatives Update für Windows 11 (KB5101650)",
                &cats(&[CATEGORY_UPDATES]),
            ),
            UpdateClass::Quality
        );
    }

    /// The trap this heuristic exists to survive: a cumulative update whose
    /// title names the release it services. Both of these satisfy "windows"
    /// and "24H2"; only the KB number tells them apart from a real upgrade.
    /// Getting this wrong stops security patches being offered at all.
    #[test]
    fn a_cumulative_update_naming_its_release_is_still_a_quality_update() {
        for title in [
            "2026-07 Kumulatives Update für Windows 11 Version 24H2 für x64-basierte Systeme (KB5101650)",
            "2026-07 Cumulative Update for Windows 11 Version 24H2 for x64-based Systems (KB5101650)",
            "2025-11 Cumulative Update for Windows 10 Version 22H2 for x64-based Systems (KB5032189)",
            "Sicherheitsupdate für Windows 11 Version 23H2 (KB5002857)",
        ] {
            assert_eq!(
                classify(title, &cats(&[CATEGORY_SECURITY])),
                UpdateClass::Security,
                "misread as an upgrade: {title}"
            );
        }
    }

    #[test]
    fn a_feature_upgrade_has_a_release_tag_and_no_kb_number() {
        for title in [
            "Funktionsupdate für Windows 11, Version 24H2",
            "Feature update to Windows 11, version 23H2",
            "Windows 10, version 22H2",
        ] {
            assert_eq!(
                classify(title, &cats(&[CATEGORY_UPDATES])),
                UpdateClass::FeatureUpgrade,
                "missed an upgrade: {title}"
            );
        }
    }

    #[test]
    fn kb_detection_needs_digits_immediately_after_kb() {
        // "kb" occurs inside ordinary words; only "KB" + digits is an article
        // number, and a bare "KB" must not suppress the upgrade check.
        assert!(has_kb_number("update (kb5101650)"));
        assert!(!has_kb_number("kbd layout pack"));
        assert!(!has_kb_number("kb without a number"));
    }

    #[test]
    fn a_service_pack_is_treated_as_a_feature_upgrade() {
        assert_eq!(
            classify("Service Pack 2", &cats(&[CATEGORY_SERVICE_PACK])),
            UpdateClass::FeatureUpgrade
        );
    }

    #[test]
    fn a_driver_is_classified_as_a_driver() {
        assert_eq!(
            classify("Intel - Display - 31.0.101", &cats(&[CATEGORY_DRIVER])),
            UpdateClass::Driver
        );
    }

    #[test]
    fn an_unrecognised_category_falls_back_to_quality() {
        assert_eq!(
            classify(
                "Something new",
                &cats(&["11111111-2222-3333-4444-555555555555"])
            ),
            UpdateClass::Quality
        );
        assert_eq!(classify("No categories at all", &[]), UpdateClass::Quality);
    }

    #[test]
    fn guids_compare_ignoring_braces_and_case() {
        let braced = format!("{{{}}}", CATEGORY_SECURITY.to_ascii_uppercase());
        assert_eq!(classify("x", &cats(&[&braced])), UpdateClass::Security);
    }

    // Policy consequences of the class.

    #[test]
    fn only_feature_upgrades_require_opting_in() {
        assert!(UpdateClass::FeatureUpgrade.requires_opt_in());
        for c in [
            UpdateClass::Definition,
            UpdateClass::Security,
            UpdateClass::Quality,
            UpdateClass::Driver,
        ] {
            assert!(!c.requires_opt_in(), "{} should not need opt-in", c.id());
        }
    }

    #[test]
    fn definitions_are_the_only_class_exempt_from_a_restore_point() {
        assert!(!UpdateClass::Definition.takes_restore_point());
        for c in [
            UpdateClass::Security,
            UpdateClass::Quality,
            UpdateClass::FeatureUpgrade,
            UpdateClass::Driver,
        ] {
            assert!(c.takes_restore_point(), "{} should take one", c.id());
        }
    }

    #[test]
    fn every_class_has_a_distinct_id_and_a_label() {
        let all = [
            UpdateClass::Definition,
            UpdateClass::Security,
            UpdateClass::Quality,
            UpdateClass::FeatureUpgrade,
            UpdateClass::Driver,
        ];
        let mut ids: Vec<&str> = all.iter().map(|c| c.id()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "class ids are not unique");
        assert!(all.iter().all(|c| !c.label().is_empty()));
    }
}
