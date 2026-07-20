//! Loose version parsing and comparison.
//!
//! Package managers in the wild do not agree on a version grammar. winget alone
//! yields `1.2.3`, `1.2.3.4`, `2024.05.01`, `1.0.0-beta.2`, `v3.1`, `21H2` and
//! the literal string `Unknown`. Comparing those as plain strings is what makes
//! naive updaters "upgrade" 1.10.0 down to 1.9.0, so we parse into a structured
//! form and compare segment-wise.
//!
//! The rules, in order:
//!   * a leading `v`/`V` is ignored
//!   * the release part is split on `.`, `-` and `_` into numeric or textual
//!     segments; numeric segments compare numerically, so 10 > 9
//!   * a shorter release compares as if zero-padded, so 1.2 == 1.2.0
//!   * a pre-release suffix (after the first `-`) makes a version *lower* than
//!     the same version without one, matching semver
//!   * anything we cannot parse becomes [`Version::Unknown`], which is never
//!     ordered against a real version — the policy layer refuses to act on it
//!     rather than guessing.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

/// One dot-separated piece of a version string.
///
/// Public only because it appears in [`Version`]'s serialised shape; callers
/// have no reason to construct one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Segment {
    Num(u64),
    Text(String),
}

impl Segment {
    fn parse(raw: &str) -> Self {
        // Leading zeros are common in date-like versions (2024.05.01); parsing
        // them as numbers is what we want, so `05` and `5` compare equal.
        match raw.parse::<u64>() {
            Ok(n) => Segment::Num(n),
            Err(_) => Segment::Text(raw.to_ascii_lowercase()),
        }
    }
}

impl Ord for Segment {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Segment::Num(a), Segment::Num(b)) => a.cmp(b),
            (Segment::Text(a), Segment::Text(b)) => a.cmp(b),
            // A numeric segment outranks a textual one: 1.2 > 1.rc.
            (Segment::Num(_), Segment::Text(_)) => Ordering::Greater,
            (Segment::Text(_), Segment::Num(_)) => Ordering::Less,
        }
    }
}

impl PartialOrd for Segment {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A parsed package version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Version {
    /// A version we understood well enough to order against others.
    Parsed {
        /// Original text, preserved so we hand the exact string back to the
        /// package manager when pinning.
        raw: String,
        release: Vec<Segment>,
        pre: Vec<Segment>,
    },
    /// A version we could not make sense of, including winget's `Unknown`.
    Unknown(String),
}

impl Version {
    /// Parse a version string. Never fails; unparseable input becomes
    /// [`Version::Unknown`].
    pub fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Version::Unknown(raw.to_string());
        }

        // winget and PowerShell both emit these for "we could not read it".
        let lowered = trimmed.to_ascii_lowercase();
        if matches!(
            lowered.as_str(),
            "unknown" | "n/a" | "none" | "<none>" | "-"
        ) {
            return Version::Unknown(raw.to_string());
        }

        // Vendors append free text to the version they register, e.g. winget
        // reports Visual Studio as `17.14.29 (March 2026)`. No real version
        // contains a space, so everything from the first one is commentary —
        // dropping it lets 17.14.29 order properly against 17.14.36 instead of
        // falling back to a textual segment comparison.
        let body = trimmed.split_whitespace().next().unwrap_or(trimmed);

        let body = body.strip_prefix(['v', 'V']).unwrap_or(body);

        // Build metadata after `+` carries no ordering weight in semver, and in
        // practice it is noise (git hashes, build numbers). Drop it.
        let body = body.split('+').next().unwrap_or(body);

        // Split release from pre-release at the first `-` that follows a digit,
        // so `1.0.0-beta` splits but `21H2-preview`-style names still parse.
        let (release_raw, pre_raw) = match body.find('-') {
            Some(i) if body[..i].chars().any(|c| c.is_ascii_digit()) => {
                (&body[..i], Some(&body[i + 1..]))
            }
            _ => (body, None),
        };

        let release: Vec<Segment> = release_raw
            .split(['.', '_'])
            .filter(|s| !s.is_empty())
            .map(Segment::parse)
            .collect();

        // Require at least one numeric segment. A purely textual "version" like
        // a channel name carries no ordering we can trust.
        if release.is_empty() || !release.iter().any(|s| matches!(s, Segment::Num(_))) {
            return Version::Unknown(raw.to_string());
        }

        let pre: Vec<Segment> = pre_raw
            .map(|p| {
                p.split(['.', '-', '_'])
                    .filter(|s| !s.is_empty())
                    .map(Segment::parse)
                    .collect()
            })
            .unwrap_or_default();

        Version::Parsed {
            raw: trimmed.to_string(),
            release,
            pre,
        }
    }

    /// The original string, for handing back to a package manager verbatim.
    pub fn raw(&self) -> &str {
        match self {
            Version::Parsed { raw, .. } => raw,
            Version::Unknown(raw) => raw,
        }
    }

    /// Whether this version can participate in ordering comparisons.
    pub fn is_known(&self) -> bool {
        matches!(self, Version::Parsed { .. })
    }

    /// True when this is a pre-release (beta, rc, alpha...).
    ///
    /// The policy layer uses this to keep users on stable channels unless they
    /// opt in explicitly.
    pub fn is_prerelease(&self) -> bool {
        match self {
            Version::Parsed { pre, .. } => {
                !pre.is_empty()
                    && pre.iter().any(|s| match s {
                        Segment::Text(t) => {
                            matches!(
                                t.as_str(),
                                "alpha"
                                    | "beta"
                                    | "rc"
                                    | "pre"
                                    | "preview"
                                    | "dev"
                                    | "nightly"
                                    | "canary"
                                    | "insider"
                                    | "snapshot"
                                    | "test"
                            )
                        }
                        Segment::Num(_) => false,
                    })
            }
            Version::Unknown(_) => false,
        }
    }

    /// Compare two versions, returning `None` when either side is unknown.
    ///
    /// Callers must treat `None` as "refuse to act", never as "equal".
    pub fn compare(&self, other: &Version) -> Option<Ordering> {
        let (a_rel, a_pre) = match self {
            Version::Parsed { release, pre, .. } => (release, pre),
            Version::Unknown(_) => return None,
        };
        let (b_rel, b_pre) = match other {
            Version::Parsed { release, pre, .. } => (release, pre),
            Version::Unknown(_) => return None,
        };

        // Zero-pad the shorter release so 1.2 == 1.2.0.
        let len = a_rel.len().max(b_rel.len());
        for i in 0..len {
            let a = a_rel.get(i).cloned().unwrap_or(Segment::Num(0));
            let b = b_rel.get(i).cloned().unwrap_or(Segment::Num(0));
            match a.cmp(&b) {
                Ordering::Equal => continue,
                non_eq => return Some(non_eq),
            }
        }

        // Equal releases: absence of a pre-release wins (1.0.0 > 1.0.0-rc1).
        match (a_pre.is_empty(), b_pre.is_empty()) {
            (true, true) => Some(Ordering::Equal),
            (true, false) => Some(Ordering::Greater),
            (false, true) => Some(Ordering::Less),
            (false, false) => {
                let len = a_pre.len().max(b_pre.len());
                for i in 0..len {
                    match (a_pre.get(i), b_pre.get(i)) {
                        (Some(a), Some(b)) => match a.cmp(b) {
                            Ordering::Equal => continue,
                            non_eq => return Some(non_eq),
                        },
                        // A longer pre-release chain is the later one:
                        // 1.0-rc.1 < 1.0-rc.1.2
                        (Some(_), None) => return Some(Ordering::Greater),
                        (None, Some(_)) => return Some(Ordering::Less),
                        (None, None) => break,
                    }
                }
                Some(Ordering::Equal)
            }
        }
    }

    /// True when `candidate` is strictly newer than `self`.
    ///
    /// Returns `false` — not an error — when either version is unknown, so the
    /// default answer to "should we upgrade?" is always no.
    pub fn is_upgrade_to(&self, candidate: &Version) -> bool {
        matches!(self.compare(candidate), Some(Ordering::Less))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Version::Parsed { raw, .. } => f.write_str(raw),
            Version::Unknown(_) => f.write_str("unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s)
    }

    #[test]
    fn numeric_segments_compare_numerically_not_lexically() {
        // The classic string-comparison bug: "1.9" > "1.10" as text.
        assert!(v("1.9.0").is_upgrade_to(&v("1.10.0")));
        assert!(!v("1.10.0").is_upgrade_to(&v("1.9.0")));
    }

    #[test]
    fn four_part_windows_versions_order_correctly() {
        assert!(v("1.2.3.4").is_upgrade_to(&v("1.2.3.5")));
        assert!(!v("1.2.3.5").is_upgrade_to(&v("1.2.3.4")));
    }

    #[test]
    fn missing_trailing_segments_are_zero() {
        assert_eq!(v("1.2").compare(&v("1.2.0")), Some(Ordering::Equal));
        assert_eq!(v("1.2").compare(&v("1.2.0.0")), Some(Ordering::Equal));
        assert!(v("1.2").is_upgrade_to(&v("1.2.1")));
    }

    #[test]
    fn leading_v_is_ignored() {
        assert_eq!(v("v1.2.3").compare(&v("1.2.3")), Some(Ordering::Equal));
    }

    #[test]
    fn build_metadata_is_ignored() {
        assert_eq!(
            v("1.2.3+abc123").compare(&v("1.2.3")),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn date_versions_with_leading_zeros_compare_numerically() {
        assert!(v("2024.05.01").is_upgrade_to(&v("2024.05.10")));
        assert_eq!(
            v("2024.05.01").compare(&v("2024.5.1")),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn prerelease_sorts_below_its_release() {
        assert!(v("1.0.0-rc1").is_upgrade_to(&v("1.0.0")));
        assert!(!v("1.0.0").is_upgrade_to(&v("1.0.0-rc1")));
    }

    #[test]
    fn prerelease_chain_orders_by_length_then_value() {
        assert!(v("1.0.0-rc.1").is_upgrade_to(&v("1.0.0-rc.2")));
        assert!(v("1.0.0-rc.1").is_upgrade_to(&v("1.0.0-rc.1.1")));
    }

    #[test]
    fn prereleases_are_detected() {
        assert!(v("1.0.0-beta.2").is_prerelease());
        assert!(v("2.1.0-nightly").is_prerelease());
        assert!(v("3.0.0-insider").is_prerelease());
        // A numeric-only suffix is a build revision, not a beta channel.
        assert!(!v("1.0.0-4").is_prerelease());
        assert!(!v("1.0.0").is_prerelease());
    }

    #[test]
    fn unknown_versions_never_compare() {
        assert!(!v("Unknown").is_known());
        assert!(!v("").is_known());
        assert!(!v("n/a").is_known());
        assert_eq!(v("Unknown").compare(&v("1.0.0")), None);
        assert_eq!(v("1.0.0").compare(&v("Unknown")), None);
    }

    #[test]
    fn unknown_version_is_never_an_upgrade_in_either_direction() {
        // This is the guard that stops the `--include-unknown` sidegrade class
        // of bug: no known-good comparison, no action.
        assert!(!v("Unknown").is_upgrade_to(&v("9.9.9")));
        assert!(!v("9.9.9").is_upgrade_to(&v("Unknown")));
    }

    #[test]
    fn purely_textual_versions_are_unknown() {
        assert!(!v("stable").is_known());
        assert!(!v("latest").is_known());
    }

    #[test]
    fn trailing_vendor_commentary_is_ignored_for_ordering() {
        // Real winget data for Visual Studio Community 2022.
        let installed = v("17.14.29 (March 2026)");
        assert!(installed.is_known());
        assert!(installed.is_upgrade_to(&v("17.14.36")));
        assert!(!installed.is_upgrade_to(&v("17.14.20")));
        // ...and it still compares equal to the bare number.
        assert_eq!(installed.compare(&v("17.14.29")), Some(Ordering::Equal));
    }

    #[test]
    fn commentary_is_kept_in_the_raw_string() {
        assert_eq!(v("17.14.29 (March 2026)").raw(), "17.14.29 (March 2026)");
    }

    #[test]
    fn raw_string_is_preserved_for_pinning() {
        // We must hand the package manager back exactly what it gave us.
        assert_eq!(v("v1.2.3").raw(), "v1.2.3");
        assert_eq!(v("  1.2.3  ").raw(), "1.2.3");
    }
}
