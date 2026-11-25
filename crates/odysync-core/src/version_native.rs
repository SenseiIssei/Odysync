//! Version ordering the way each package manager defines it.
//!
//! The general algebra in [`crate::version`] is right for the versions that
//! software vendors write. It is wrong for the ones distributions write: a
//! Debian security update moves `1.1.1w-0+deb11u1` to `1.1.1w-0+deb11u2`,
//! which the general rules read as the same version with different build
//! metadata, so the update was shown as "not an upgrade" and never installed.
//! `1:9.2p1-2+deb12u3` was not readable at all.
//!
//! These are the published algorithms, reimplemented so that they run in tests
//! on every platform: `dpkg --compare-versions`, `rpmvercmp` over
//! epoch:version-release, pacman's `vercmp`, and PEP 440 for Python packages.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// Whose ordering rules a version follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scheme {
    /// Debian, Ubuntu and derivatives.
    Dpkg,
    /// Fedora, RHEL, openSUSE.
    Rpm,
    /// Arch Linux and derivatives.
    Pacman,
    /// Python packages (PEP 440).
    Pep440,
}

impl Scheme {
    /// Whether `raw` is a version this scheme can order.
    pub fn accepts(self, raw: &str) -> bool {
        let raw = raw.trim();
        if raw.is_empty() || raw.chars().any(char::is_whitespace) {
            return false;
        }
        match self {
            // The managers themselves require the upstream part to contain a
            // digit in practice; a bare word such as `stable` or `(none)` is
            // what they print when there is no version.
            Scheme::Dpkg | Scheme::Rpm | Scheme::Pacman => {
                raw.bytes().any(|b| b.is_ascii_digit())
                    && raw
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b".+-~:^_".contains(&b))
            }
            Scheme::Pep440 => Pep440::parse(raw).is_some(),
        }
    }

    /// Order two versions, or `None` when either cannot be read.
    pub fn compare(self, a: &str, b: &str) -> Option<Ordering> {
        let (a, b) = (a.trim(), b.trim());
        if !self.accepts(a) || !self.accepts(b) {
            return None;
        }
        match self {
            Scheme::Dpkg => Some(dpkg(a, b)),
            Scheme::Rpm => Some(evr(a, b, rpmvercmp)),
            Scheme::Pacman => Some(evr(a, b, alpm_vercmp)),
            Scheme::Pep440 => Some(Pep440::parse(a)?.cmp(&Pep440::parse(b)?)),
        }
    }

    /// Whether `raw` is a pre-release under this scheme.
    ///
    /// A distribution decides itself what is released, so what its stable
    /// repositories carry is stable even when upstream calls it `~rc1`. The
    /// one place the marker means "not released" is a Python package.
    pub fn is_prerelease(self, raw: &str) -> bool {
        match self {
            Scheme::Pep440 => Pep440::parse(raw.trim()).is_some_and(|v| v.is_prerelease()),
            Scheme::Dpkg | Scheme::Rpm | Scheme::Pacman => false,
        }
    }
}

/// Split `epoch:rest`. A missing epoch is zero.
fn split_epoch(v: &str) -> (u64, &str) {
    match v.split_once(':') {
        Some((e, rest)) => match e.parse() {
            Ok(n) => (n, rest),
            Err(_) => (0, v),
        },
        None => (0, v),
    }
}

// ── dpkg ────────────────────────────────────────────────────────────────────

fn dpkg(a: &str, b: &str) -> Ordering {
    let (ea, a) = split_epoch(a);
    let (eb, b) = split_epoch(b);
    if ea != eb {
        return ea.cmp(&eb);
    }
    // The revision is what follows the *last* hyphen; none counts as "0".
    let split = |v: &str| match v.rsplit_once('-') {
        Some((u, r)) => (u.to_string(), r.to_string()),
        None => (v.to_string(), String::new()),
    };
    let (ua, ra) = split(a);
    let (ub, rb) = split(b);
    verrevcmp(&ua, &ub).then_with(|| verrevcmp(&ra, &rb))
}

/// dpkg's `order()`: `~` before everything, even the end; letters before the
/// remaining punctuation; digits end a non-digit run.
fn dpkg_order(c: Option<u8>) -> i32 {
    match c {
        None => 0,
        Some(c) if c.is_ascii_digit() => 0,
        Some(c) if c.is_ascii_alphabetic() => i32::from(c),
        Some(b'~') => -1,
        Some(c) => i32::from(c) + 256,
    }
}

fn verrevcmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    let digit = |s: &[u8], k: usize| s.get(k).is_some_and(u8::is_ascii_digit);

    while i < a.len() || j < b.len() {
        while (i < a.len() && !digit(a, i)) || (j < b.len() && !digit(b, j)) {
            let (oa, ob) = (dpkg_order(a.get(i).copied()), dpkg_order(b.get(j).copied()));
            if oa != ob {
                return oa.cmp(&ob);
            }
            i += 1;
            j += 1;
        }
        let (na, next_i) = digits(a, i);
        let (nb, next_j) = digits(b, j);
        match cmp_digits(na, nb) {
            Ordering::Equal => {}
            other => return other,
        }
        i = next_i;
        j = next_j;
    }
    Ordering::Equal
}

/// The run of digits at `i`, without leading zeros, and where it ends.
fn digits(s: &[u8], i: usize) -> (&[u8], usize) {
    let start = i.min(s.len());
    let end = s[start..]
        .iter()
        .position(|c| !c.is_ascii_digit())
        .map_or(s.len(), |n| start + n);
    let run = &s[start..end];
    let zeros = run.iter().take_while(|c| **c == b'0').count();
    (&run[zeros..], end)
}

/// Compare digit runs of any length without overflowing.
fn cmp_digits(a: &[u8], b: &[u8]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

// ── rpm and pacman ──────────────────────────────────────────────────────────

/// Epoch, then version, then release, the last only when both sides have one.
fn evr(a: &str, b: &str, cmp: fn(&str, &str) -> Ordering) -> Ordering {
    let (ea, a) = split_epoch(a);
    let (eb, b) = split_epoch(b);
    if ea != eb {
        return ea.cmp(&eb);
    }
    let split = |v: &str| match v.rsplit_once('-') {
        Some((v, r)) => (v.to_string(), Some(r.to_string())),
        None => (v.to_string(), None),
    };
    let (va, ra) = split(a);
    let (vb, rb) = split(b);
    cmp(&va, &vb).then_with(|| match (ra, rb) {
        (Some(ra), Some(rb)) => cmp(&ra, &rb),
        _ => Ordering::Equal,
    })
}

/// The next run of only digits or only letters at `i`.
fn run_at(s: &[u8], i: usize) -> (&[u8], bool, usize) {
    let numeric = s[i].is_ascii_digit();
    let end = s[i..]
        .iter()
        .position(|c| {
            if numeric {
                !c.is_ascii_digit()
            } else {
                !c.is_ascii_alphabetic()
            }
        })
        .map_or(s.len(), |n| i + n);
    (&s[i..end], numeric, end)
}

fn cmp_runs(a: &[u8], b: &[u8], numeric: bool) -> Ordering {
    if numeric {
        cmp_digits(digits(a, 0).0, digits(b, 0).0)
    } else {
        a.cmp(b)
    }
}

/// rpm's `rpmvercmp`, including `~` (sorts before anything) and `^` (sorts
/// after the same version, before anything longer).
fn rpmvercmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    let skip = |s: &[u8], mut k: usize| {
        while k < s.len() && !s[k].is_ascii_alphanumeric() && s[k] != b'~' && s[k] != b'^' {
            k += 1;
        }
        k
    };

    while i < a.len() || j < b.len() {
        i = skip(a, i);
        j = skip(b, j);
        let (ca, cb) = (a.get(i).copied(), b.get(j).copied());

        if ca == Some(b'~') || cb == Some(b'~') {
            if ca != Some(b'~') {
                return Ordering::Greater;
            }
            if cb != Some(b'~') {
                return Ordering::Less;
            }
            i += 1;
            j += 1;
            continue;
        }
        if ca == Some(b'^') || cb == Some(b'^') {
            match (ca, cb) {
                (None, _) => return Ordering::Less,
                (_, None) => return Ordering::Greater,
                (Some(b'^'), Some(b'^')) => {
                    i += 1;
                    j += 1;
                    continue;
                }
                (Some(b'^'), _) => return Ordering::Less,
                _ => return Ordering::Greater,
            }
        }
        if ca.is_none() || cb.is_none() {
            break;
        }

        let (ra, na, ei) = run_at(a, i);
        let (rb, nb, ej) = run_at(b, j);
        if na != nb {
            // A numeric segment is newer than an alphabetic one.
            return if na {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
        match cmp_runs(ra, rb, na) {
            Ordering::Equal => {}
            other => return other,
        }
        i = ei;
        j = ej;
    }

    // Whichever still has something left is newer.
    (i < a.len()).cmp(&(j < b.len()))
}

/// pacman's `vercmp`, which differs from rpm in two ways: a remaining
/// alphabetic part loses (`1.0rc` < `1.0`), and there is no `~` or `^`.
fn alpm_vercmp(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    let skip = |s: &[u8], mut k: usize| {
        while k < s.len() && !s[k].is_ascii_alphanumeric() {
            k += 1;
        }
        k
    };

    while i < a.len() && j < b.len() {
        let (si, sj) = (skip(a, i), skip(b, j));
        let (gap_a, gap_b) = (si - i, sj - j);
        i = si;
        j = sj;
        if i >= a.len() || j >= b.len() {
            break;
        }
        if gap_a != gap_b {
            return gap_a.cmp(&gap_b);
        }
        let (ra, na, ei) = run_at(a, i);
        let (rb, nb, ej) = run_at(b, j);
        if na != nb {
            return if na {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
        match cmp_runs(ra, rb, na) {
            Ordering::Equal => {}
            other => return other,
        }
        i = ei;
        j = ej;
    }

    let alpha = |c: Option<&u8>| c.is_some_and(u8::is_ascii_alphabetic);
    match (a.get(i), b.get(j)) {
        (None, None) => Ordering::Equal,
        (x, y) => {
            if (x.is_none() && !alpha(y)) || alpha(x) {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
    }
}

// ── PEP 440 ─────────────────────────────────────────────────────────────────

/// A Python version, normalised enough to order.
#[derive(Debug, PartialEq, Eq)]
struct Pep440 {
    epoch: u64,
    release: Vec<u64>,
    /// 0 = alpha, 1 = beta, 2 = candidate.
    pre: Option<(u8, u64)>,
    post: Option<u64>,
    dev: Option<u64>,
    local: Vec<LocalPart>,
}

/// Alphabetic local parts sort before numeric ones.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LocalPart {
    Text(String),
    Num(u64),
}

impl Pep440 {
    fn parse(raw: &str) -> Option<Self> {
        let lower = raw.trim().to_ascii_lowercase();
        let body = lower.strip_prefix('v').unwrap_or(&lower);
        let (body, local) = match body.split_once('+') {
            Some((b, l)) => (b, Some(l)),
            None => (body, None),
        };
        let (epoch, body) = match body.split_once('!') {
            Some((e, b)) => (e.parse().ok()?, b),
            None => (0, body),
        };

        // Release: digits separated by dots, as far as they go.
        let mut rest = body;
        let mut release = Vec::new();
        loop {
            let n = rest.bytes().take_while(u8::is_ascii_digit).count();
            if n == 0 {
                break;
            }
            release.push(rest[..n].parse().ok()?);
            rest = &rest[n..];
            match rest.strip_prefix('.') {
                Some(r) if r.starts_with(|c: char| c.is_ascii_digit()) => rest = r,
                _ => break,
            }
        }
        if release.is_empty() {
            return None;
        }

        let mut v = Pep440 {
            epoch,
            release,
            pre: None,
            post: None,
            dev: None,
            local: local
                .map(|l| {
                    l.split(['.', '-', '_'])
                        .map(|p| match p.parse() {
                            Ok(n) => LocalPart::Num(n),
                            Err(_) => LocalPart::Text(p.to_string()),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };

        while !rest.is_empty() {
            rest = rest.trim_start_matches(['.', '-', '_']);
            let n = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
            let (word, tail) = rest.split_at(n);
            let tail = tail
                .strip_prefix(['.', '-', '_'])
                .filter(|t| t.starts_with(|c: char| c.is_ascii_digit()))
                .unwrap_or(tail);
            let m = tail.bytes().take_while(u8::is_ascii_digit).count();
            let number: u64 = if m == 0 { 0 } else { tail[..m].parse().ok()? };
            rest = &tail[m..];
            match word {
                "a" | "alpha" => v.pre = Some((0, number)),
                "b" | "beta" => v.pre = Some((1, number)),
                "c" | "rc" | "pre" | "preview" => v.pre = Some((2, number)),
                "post" | "rev" | "r" => v.post = Some(number),
                "dev" => v.dev = Some(number),
                // `1.0-1` is the old spelling of `1.0.post1`.
                "" if m > 0 && v.post.is_none() => v.post = Some(number),
                _ => return None,
            }
        }
        Some(v)
    }

    fn is_prerelease(&self) -> bool {
        self.pre.is_some() || self.dev.is_some()
    }

    /// PEP 440's sort keys for the optional parts: a bare dev release sorts
    /// before any pre-release, a final release after all of them.
    fn pre_key(&self) -> (u8, u8, u64) {
        match (self.pre, self.post, self.dev) {
            (Some((kind, n)), _, _) => (1, kind, n),
            (None, None, Some(_)) => (0, 0, 0),
            _ => (2, 0, 0),
        }
    }
}

impl Ord for Pep440 {
    fn cmp(&self, other: &Self) -> Ordering {
        let len = self.release.len().max(other.release.len());
        let seg = |v: &Pep440, i: usize| v.release.get(i).copied().unwrap_or(0);
        self.epoch
            .cmp(&other.epoch)
            .then_with(|| {
                (0..len)
                    .map(|i| seg(self, i).cmp(&seg(other, i)))
                    .find(|o| o.is_ne())
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| self.pre_key().cmp(&other.pre_key()))
            // No post sorts before any post; no dev sorts after any dev.
            .then_with(|| self.post.cmp(&other.post))
            .then_with(|| {
                self.dev
                    .unwrap_or(u64::MAX)
                    .cmp(&other.dev.unwrap_or(u64::MAX))
            })
            .then_with(|| self.local.cmp(&other.local))
    }
}

impl PartialOrd for Pep440 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assert `older < newer` in both directions.
    fn ordered(scheme: Scheme, older: &str, newer: &str) {
        assert_eq!(
            scheme.compare(older, newer),
            Some(Ordering::Less),
            "{scheme:?}: {older} should be older than {newer}"
        );
        assert_eq!(scheme.compare(newer, older), Some(Ordering::Greater));
    }

    fn same(scheme: Scheme, a: &str, b: &str) {
        assert_eq!(scheme.compare(a, b), Some(Ordering::Equal), "{a} == {b}");
    }

    /// The case this module exists for: these are security updates.
    #[test]
    fn debian_security_updates_are_upgrades() {
        let d = Scheme::Dpkg;
        ordered(d, "1.1.1w-0+deb11u1", "1.1.1w-0+deb11u2");
        ordered(d, "7.88.1-10+deb12u5", "7.88.1-10+deb12u14");
        ordered(d, "1.9.13p3-1", "1.9.13p3-1+deb12u1");
        ordered(d, "1:9.2p1-2+deb12u2", "1:9.2p1-2+deb12u3");
        ordered(d, "2024a-0+deb12u1", "2025b-0+deb12u1");
        ordered(d, "2.39-0ubuntu8.9", "2.39-0ubuntu8.10");
        ordered(d, "1.2-1", "1.2-1ubuntu1");
    }

    /// Cases from dpkg's own `t-version.c`.
    #[test]
    fn dpkg_matches_the_reference_ordering() {
        let d = Scheme::Dpkg;
        ordered(d, "1.0~rc1-1", "1.0-1");
        ordered(d, "1.0~~", "1.0~");
        ordered(d, "1.0~", "1.0");
        ordered(d, "9", "1:0");
        ordered(d, "1.2.3", "1.2.10");
        ordered(d, "1.0", "1.0a");
        ordered(d, "1.0a", "1.0+");
        ordered(d, "2:9.1", "2:10.0");
        same(d, "0:1.0", "1.0");
        same(d, "1.0-0", "1.0");
        same(d, "1.00", "1.0");
        ordered(
            d,
            "20240101000000000000000001",
            "20240101000000000000000002",
        );
    }

    /// Cases from rpm's own `rpmvercmp.at`.
    #[test]
    fn rpm_matches_the_reference_ordering() {
        let r = Scheme::Rpm;
        ordered(r, "1.0", "2.0");
        ordered(r, "2.0", "2.0.1");
        ordered(r, "2.0.1", "2.0.1a");
        ordered(r, "5.5p1", "5.5p2");
        ordered(r, "5.5p1", "5.5p10");
        ordered(r, "10xyz", "10.1xyz");
        ordered(r, "xyz10a", "xyz10.1a");
        ordered(r, "1.0~rc1", "1.0");
        ordered(r, "1.0~rc1", "1.0~rc2");
        ordered(r, "1.0~rc1~git123", "1.0~rc1");
        ordered(r, "1.0", "1.0^");
        ordered(r, "1.0^git1", "1.0^git2");
        ordered(r, "1.0^git1", "1.01");
        ordered(r, "1.0~rc1", "1.0~rc1^git1");
        ordered(r, "1.0^git1~pre", "1.0^git1");
        same(r, "2_0", "2.0");
        same(r, "1.05", "1.5");
    }

    #[test]
    fn rpm_compares_epoch_version_and_release() {
        let r = Scheme::Rpm;
        ordered(r, "2.3-4.fc43", "1:1.0-1.fc43");
        ordered(r, "1:9.0-1", "1:10.0-1");
        ordered(r, "6.11.4-301.fc41", "6.11.10-200.fc41");
        ordered(r, "1.2-4.fc9", "1.2-4.fc10");
        ordered(r, "1.2-4.fc43", "1.2-10.fc43");
        // A query without a release must not look like an older one.
        same(r, "1.2", "1.2-4.fc43");
    }

    /// Cases from pacman's own `vercmptest.sh`.
    #[test]
    fn pacman_matches_the_reference_ordering() {
        let p = Scheme::Pacman;
        ordered(p, "1.5.0", "1.5.1");
        ordered(p, "1.5", "1.5.1");
        ordered(p, "1.5.0-1", "1.5.0-2");
        ordered(p, "1.0a", "1.0alpha");
        ordered(p, "1.0alpha", "1.0b");
        ordered(p, "1.0beta", "1.0p");
        ordered(p, "1.0pre", "1.0rc");
        ordered(p, "1.0rc", "1.0");
        ordered(p, "1.5.b", "1.5.1");
        ordered(p, "1.5b", "1.5");
        ordered(p, "1.5b", "1.5.1");
        ordered(p, "1.0", "1.0.a");
        ordered(p, "1.0.a", "1.0.1");
        ordered(p, "2:9.0-1", "2:10.0-1");
        ordered(p, "9.9-1", "1:0.1-1");
        same(p, "0:1.0", "1.0");
        same(p, "1.5", "1.5-1");
    }

    #[test]
    fn python_versions_follow_pep_440() {
        let p = Scheme::Pep440;
        for pair in [
            "1.0.dev456",
            "1.0a1",
            "1.0a2.dev456",
            "1.0a12.dev456",
            "1.0a12",
            "1.0b1.dev456",
            "1.0b2",
            "1.0b2.post345.dev456",
            "1.0b2.post345",
            "1.0rc1.dev456",
            "1.0rc1",
            "1.0",
            "1.0+abc.5",
            "1.0+abc.7",
            "1.0+5",
            "1.0.post456.dev34",
            "1.0.post456",
            "1.1.dev1",
            "1!0.1",
        ]
        .windows(2)
        {
            ordered(p, pair[0], pair[1]);
        }
        same(p, "1.0", "1.0.0");
        same(p, "1.0-rc.1", "1.0rc1");
        same(p, "1.0-1", "1.0.post1");
        ordered(p, "1.0rc9", "1.0rc10");
        assert!(p.is_prerelease("2.0.0rc1"));
        assert!(p.is_prerelease("2.0.0.dev3"));
        assert!(!p.is_prerelease("2.0.0.post1"));
        // A CUDA build must not be "upgraded" to the plain one of the same
        // version.
        ordered(p, "2.1.0", "2.1.0+cu118");
    }

    #[test]
    fn unreadable_versions_are_refused_rather_than_ordered() {
        for scheme in [Scheme::Dpkg, Scheme::Rpm, Scheme::Pacman, Scheme::Pep440] {
            for bad in ["", "  ", "stable", "(none)", "1.0 beta", "n/a"] {
                assert!(!scheme.accepts(bad), "{scheme:?} accepted {bad:?}");
                assert_eq!(scheme.compare(bad, "1.0"), None);
                assert_eq!(scheme.compare("1.0", bad), None);
            }
        }
        assert_eq!(Scheme::Pep440.compare("1.0", "1.0.foo"), None);
    }
}
