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
//!   * a segment that mixes digits and letters is read as its number followed
//!     by a suffix: `0rc1` is a pre-release of `0`, `1w` a later patch of `1`
//!   * anything we cannot parse becomes [`Version::Unknown`], which is never
//!     ordered against a real version; nor are two versions whose difference
//!     has no agreed meaning. The policy layer refuses to act on both rather
//!     than guessing.
//!
//! Distribution packages and Python packages follow published rules of their
//! own; [`Version::parse_as`] uses those, see [`crate::version_native`].

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::version_native::Scheme;

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

/// Words that mark a version as not yet released.
const PRERELEASE_WORDS: &[&str] = &[
    "alpha",
    "beta",
    "rc",
    "cr",
    "pre",
    "prerelease",
    "preview",
    "dev",
    "nightly",
    "canary",
    "insider",
    "insiders",
    "snapshot",
    "test",
    "testing",
    "unstable",
    "ea",
    "next",
    "milestone",
];

/// Words that mark a later build of the same version.
const PATCH_WORDS: &[&str] = &["post", "p", "pl", "patch", "sp", "r", "rev", "u", "update"];

/// What a suffix such as the `rc1` in `0rc1` says about its version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SuffixKind {
    Prerelease,
    None,
    Patch,
}

/// Split `0rc1` into `("0", "rc1")` and `rc1` into `("", "rc1")`.
fn split_leading_digits(text: &str) -> (&str, &str) {
    let n = text.bytes().take_while(u8::is_ascii_digit).count();
    text.split_at(n)
}

/// Classify a lower-case suffix. `None` means we do not know what it means.
fn classify_suffix(suffix: &str) -> Option<SuffixKind> {
    if suffix.is_empty() {
        return Some(SuffixKind::None);
    }
    let n = suffix.bytes().take_while(u8::is_ascii_alphabetic).count();
    let (word, number) = suffix.split_at(n);
    if !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if PRERELEASE_WORDS.contains(&word) {
        return Some(SuffixKind::Prerelease);
    }
    if PATCH_WORDS.contains(&word) {
        return Some(SuffixKind::Patch);
    }
    match (word.len(), number.is_empty()) {
        // `1.0a2`, `1.0b1`: Python's and many others' alpha and beta.
        (1, false) if word == "a" || word == "b" => Some(SuffixKind::Prerelease),
        // `1.1.1w`, `9e`: a patch letter.
        (1, true) => Some(SuffixKind::Patch),
        _ => None,
    }
}

/// Order text so that runs of digits compare as numbers: `rc9` before `rc10`.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    while !a.is_empty() && !b.is_empty() {
        let (da, ra) = split_leading_digits(a);
        let (db, rb) = split_leading_digits(b);
        if !da.is_empty() && !db.is_empty() {
            let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
            match ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)) {
                Ordering::Equal => {}
                other => return other,
            }
            (a, b) = (ra, rb);
            continue;
        }
        let (mut ca, mut cb) = (a.chars(), b.chars());
        match ca.next().cmp(&cb.next()) {
            Ordering::Equal => {}
            other => return other,
        }
        (a, b) = (ca.as_str(), cb.as_str());
    }
    a.len().cmp(&b.len())
}

impl Segment {
    /// The segment as the number it starts with and the rest.
    fn parts(&self) -> (Option<u64>, &str) {
        match self {
            Segment::Num(n) => (Some(*n), ""),
            Segment::Text(t) => {
                let (digits, suffix) = split_leading_digits(t);
                (digits.parse().ok(), suffix)
            }
        }
    }

    /// Order two release segments, or `None` when their order has no agreed
    /// meaning.
    fn compare(&self, other: &Segment) -> Option<Ordering> {
        if self == other {
            return Some(Ordering::Equal);
        }
        match (self.parts(), other.parts()) {
            ((Some(a), sa), (Some(b), sb)) => {
                if a != b {
                    return Some(a.cmp(&b));
                }
                let (ka, kb) = (classify_suffix(sa)?, classify_suffix(sb)?);
                Some(ka.cmp(&kb).then_with(|| natural_cmp(sa, sb)))
            }
            // `1.0.beta` and the like: a word where a number is expected. It
            // only has a meaning when it is a known one.
            ((Some(_), _), (None, word)) => match classify_suffix(word)? {
                SuffixKind::Prerelease => Some(Ordering::Greater),
                _ => None,
            },
            ((None, word), (Some(_), _)) => match classify_suffix(word)? {
                SuffixKind::Prerelease => Some(Ordering::Less),
                _ => None,
            },
            ((None, a), (None, b)) => {
                let (ka, kb) = (classify_suffix(a)?, classify_suffix(b)?);
                Some(ka.cmp(&kb).then_with(|| natural_cmp(a, b)))
            }
        }
    }

    /// Order two pre-release segments the way semver does, except that digits
    /// inside a word count as numbers (`rc9` before `rc10`).
    fn compare_pre(&self, other: &Segment) -> Ordering {
        match (self, other) {
            (Segment::Num(a), Segment::Num(b)) => a.cmp(b),
            (Segment::Text(a), Segment::Text(b)) => natural_cmp(a, b),
            (Segment::Num(_), Segment::Text(_)) => Ordering::Less,
            (Segment::Text(_), Segment::Num(_)) => Ordering::Greater,
        }
    }

    /// Whether this segment says "not released yet": `rc`, `beta2`, `0rc1`.
    fn marks_prerelease(&self) -> bool {
        match self {
            Segment::Num(_) => false,
            Segment::Text(t) => {
                let (_, suffix) = split_leading_digits(t);
                classify_suffix(suffix) == Some(SuffixKind::Prerelease)
            }
        }
    }
}

/// The pre-release marker in a vendor's commentary, if there is one: `Beta 2`
/// in `5.0 Beta 2`, nothing in `17.14.29 (March 2026)`.
fn prerelease_in_commentary(rest: &str) -> Vec<Segment> {
    let words: Vec<String> = rest
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let Some(at) = words
        .iter()
        .position(|w| Segment::parse(w).marks_prerelease())
    else {
        return Vec::new();
    };
    let mut out = vec![Segment::parse(&words[at])];
    if let Some(next) = words.get(at + 1)
        && next.bytes().all(|b| b.is_ascii_digit())
    {
        out.push(Segment::parse(next));
    }
    out
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
    /// A version ordered by its package manager's own published rules.
    Native { raw: String, scheme: Scheme },
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
        // The exception is a pre-release marker: `5.0 Beta 2` is not 5.0.
        let body = trimmed.split_whitespace().next().unwrap_or(trimmed);
        let commentary = prerelease_in_commentary(&trimmed[body.len()..]);

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

        let mut pre: Vec<Segment> = pre_raw
            .map(|p| {
                p.split(['.', '-', '_'])
                    .filter(|s| !s.is_empty())
                    .map(Segment::parse)
                    .collect()
            })
            .unwrap_or_default();
        pre.extend(commentary);

        Version::Parsed {
            raw: trimmed.to_string(),
            release,
            pre,
        }
    }

    /// Parse a version that follows `scheme` rather than the general rules.
    pub fn parse_as(raw: &str, scheme: Scheme) -> Self {
        let trimmed = raw.trim();
        if scheme.accepts(trimmed) {
            Version::Native {
                raw: trimmed.to_string(),
                scheme,
            }
        } else {
            Version::Unknown(raw.to_string())
        }
    }

    /// Parse `raw` by the same rules as this version, so that a value read
    /// back from the same package manager can be compared with it.
    pub fn parse_like(&self, raw: &str) -> Self {
        match self {
            Version::Native { scheme, .. } => Version::parse_as(raw, *scheme),
            _ => Version::parse(raw),
        }
    }

    /// The original string, for handing back to a package manager verbatim.
    pub fn raw(&self) -> &str {
        match self {
            Version::Parsed { raw, .. } | Version::Native { raw, .. } => raw,
            Version::Unknown(raw) => raw,
        }
    }

    /// Whether this version can participate in ordering comparisons.
    pub fn is_known(&self) -> bool {
        !matches!(self, Version::Unknown(_))
    }

    /// True when this is a pre-release (beta, rc, alpha...).
    ///
    /// The policy layer uses this to keep users on stable channels unless they
    /// opt in explicitly.
    pub fn is_prerelease(&self) -> bool {
        match self {
            Version::Parsed { release, pre, .. } => {
                pre.iter().any(Segment::marks_prerelease)
                    || release.iter().any(Segment::marks_prerelease)
            }
            Version::Native { raw, scheme } => scheme.is_prerelease(raw),
            Version::Unknown(_) => false,
        }
    }

    /// Compare two versions, returning `None` when either side is unknown.
    ///
    /// Callers must treat `None` as "refuse to act", never as "equal".
    pub fn compare(&self, other: &Version) -> Option<Ordering> {
        let (a_rel, a_pre, b_rel, b_pre) = match (self, other) {
            (
                Version::Parsed {
                    release: ar,
                    pre: ap,
                    ..
                },
                Version::Parsed {
                    release: br,
                    pre: bp,
                    ..
                },
            ) => (ar, ap, br, bp),
            (Version::Native { raw: a, scheme: sa }, Version::Native { raw: b, scheme: sb })
                if sa == sb =>
            {
                return sa.compare(a, b);
            }
            // Unknown, or two different sets of rules.
            _ => return None,
        };

        // Zero-pad the shorter release so 1.2 == 1.2.0.
        let len = a_rel.len().max(b_rel.len());
        let zero = Segment::Num(0);
        for i in 0..len {
            let a = a_rel.get(i).unwrap_or(&zero);
            let b = b_rel.get(i).unwrap_or(&zero);
            match a.compare(b)? {
                Ordering::Equal => continue,
                non_eq => return Some(non_eq),
            }
        }

        // Equal releases: absence of a pre-release wins (1.0.0 > 1.0.0-rc1).
        // A suffix of only numbers is a pre-release to semver and a build
        // revision to almost everyone else, so against a bare version its
        // order is not knowable.
        let only_numbers = |pre: &[Segment]| pre.iter().all(|s| matches!(s, Segment::Num(_)));
        match (a_pre.is_empty(), b_pre.is_empty()) {
            (true, true) => Some(Ordering::Equal),
            (true, false) if only_numbers(b_pre) => None,
            (false, true) if only_numbers(a_pre) => None,
            (true, false) => Some(Ordering::Greater),
            (false, true) => Some(Ordering::Less),
            (false, false) => {
                let len = a_pre.len().max(b_pre.len());
                for i in 0..len {
                    match (a_pre.get(i), b_pre.get(i)) {
                        (Some(a), Some(b)) => match a.compare_pre(b) {
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
            Version::Parsed { raw, .. } | Version::Native { raw, .. } => f.write_str(raw),
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

    /// Every one of these was offered as a stable release before: only a
    /// segment that was exactly `rc` or `beta` counted as a marker.
    #[test]
    fn prerelease_markers_are_found_wherever_vendors_put_them() {
        for raw in [
            "1.0.0-rc1",
            "1.0.0-beta2",
            "1.0.0-alpha",
            "1.0.0rc1",
            "2.0a1",
            "2.0b3",
            "1.0.beta2",
            "1.0.dev0",
            "5.0 Beta 2",
            "17.15.0 Preview 3",
            "2.0 RC1",
            "3.0 (beta)",
            "24.0-ea",
            "1.2.3-SNAPSHOT",
        ] {
            assert!(v(raw).is_prerelease(), "{raw} is a pre-release");
        }
        for raw in [
            "1.1.1w",
            "9e",
            "17.14.29 (March 2026)",
            "2.39.2.windows.1",
            "1.0.0-4",
            "1.9.13p3",
            "10.0 (x64 en-US)",
            "21H2",
            "8u402",
        ] {
            assert!(!v(raw).is_prerelease(), "{raw} is a release");
        }
    }

    #[test]
    fn numbers_inside_words_compare_as_numbers() {
        assert!(v("1.0.0-rc9").is_upgrade_to(&v("1.0.0-rc10")));
        assert!(v("1.0.0rc9").is_upgrade_to(&v("1.0.0rc10")));
        assert!(v("1.0.8u99").is_upgrade_to(&v("1.0.8u402")));
        assert!(v("1.9.13p3").is_upgrade_to(&v("1.9.13p10")));
    }

    /// `3.5a` to `3.5` used to count as an upgrade, which is a downgrade.
    #[test]
    fn a_patch_letter_is_later_than_the_bare_version() {
        assert!(v("3.5").is_upgrade_to(&v("3.5a")));
        assert!(!v("3.5a").is_upgrade_to(&v("3.5")));
        assert!(v("1.1.1v").is_upgrade_to(&v("1.1.1w")));
        assert!(v("1.1.1w").is_upgrade_to(&v("1.1.2")));
    }

    #[test]
    fn a_prerelease_inside_a_segment_is_earlier_than_the_release() {
        assert!(v("1.0.0rc1").is_upgrade_to(&v("1.0.0")));
        assert!(!v("1.0.0").is_upgrade_to(&v("1.0.0rc1")));
        assert!(v("5.0 Beta 2").is_upgrade_to(&v("5.0")));
        assert!(!v("5.0").is_upgrade_to(&v("5.0 Beta 2")));
        assert!(v("1.0.beta2").is_upgrade_to(&v("1.0.0")));
    }

    /// When two versions differ in a way that has no agreed meaning, the
    /// answer is "do nothing", in both directions.
    #[test]
    fn differences_without_a_known_meaning_are_not_ordered() {
        for (a, b) in [
            ("1.0.0", "1.0.0-4"),
            ("1.0", "1.0git1234"),
            ("1.0.x", "1.0.1"),
            ("2.1.0foo", "2.1.0bar"),
        ] {
            assert_eq!(v(a).compare(&v(b)), None, "{a} against {b}");
            assert!(!v(a).is_upgrade_to(&v(b)));
            assert!(!v(b).is_upgrade_to(&v(a)));
        }
        // Two revisions of the same thing are fine.
        assert!(v("1.0.0-4").is_upgrade_to(&v("1.0.0-5")));
    }

    #[test]
    fn versions_under_different_rules_are_not_ordered() {
        let deb = Version::parse_as("1.0-1", Scheme::Dpkg);
        assert!(deb.is_known());
        assert_eq!(deb.compare(&v("1.1")), None);
        assert_eq!(deb.compare(&Version::parse_as("1.0-1", Scheme::Rpm)), None);
        assert!(deb.is_upgrade_to(&deb.parse_like("1.0-1+deb12u1")));
        assert!(!deb.parse_like("(none)").is_known());
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
