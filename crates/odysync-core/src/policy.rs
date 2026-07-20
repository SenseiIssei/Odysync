//! The safety policy: decides which offered updates are allowed to run.
//!
//! This module is the reason the rewrite exists. Every rule here maps to a way
//! the previous Python version could damage an installation:
//!
//!   * it upgraded packages whose installed version was `Unknown`, which
//!     regularly sidegraded or downgraded them
//!   * it compared versions as strings, so 1.10 looked older than 1.9
//!   * on a failed upgrade it fell back to `winget install`, reinstalling the
//!     package from scratch over a working copy and wiping its state
//!
//! The engine is pure: it takes candidates and configuration, and returns
//! decisions. No I/O, no process spawning — which is what makes it testable.

use serde::{Deserialize, Serialize};

use crate::model::{PlannedUpdate, SkipReason, UpdateCandidate};

/// User-tunable safety settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Policy {
    /// Only install final releases; never beta/rc/nightly builds.
    pub stable_only: bool,
    /// Refuse to act when either version is unparseable.
    ///
    /// Turning this off is what `winget --include-unknown` effectively does,
    /// so it stays on unless the user knowingly opts out per package.
    pub require_known_versions: bool,
    /// Packages never to touch, by `backend:id` or bare id.
    pub exclude: Vec<String>,
    /// Packages pinned to a specific version, or held at their current one.
    pub holds: Vec<Hold>,
    /// Whether the running process currently has admin/root rights.
    #[serde(skip)]
    pub elevated: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            stable_only: true,
            require_known_versions: true,
            exclude: Vec::new(),
            holds: Vec::new(),
            elevated: false,
        }
    }
}

/// A package the user has pinned or frozen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Hold {
    /// `backend:id` or a bare native id.
    pub package: String,
    /// When set, only this exact version may be installed. When `None`, the
    /// package is frozen at whatever is installed.
    pub pin: Option<String>,
    /// Free-text reason, shown back to the user when the hold blocks an update.
    pub note: Option<String>,
}

impl Policy {
    /// Does `pattern` refer to this candidate? Matches either the fully
    /// qualified `backend:id` or the bare native id, case-insensitively.
    fn matches(pattern: &str, candidate: &UpdateCandidate) -> bool {
        let pattern = pattern.trim();
        pattern.eq_ignore_ascii_case(&candidate.id.to_string())
            || pattern.eq_ignore_ascii_case(&candidate.id.native)
    }

    /// Run one candidate through every rule, returning the first that blocks
    /// it. `None` means the update may proceed.
    pub fn evaluate(&self, candidate: &UpdateCandidate) -> Option<SkipReason> {
        // 1. Explicit exclusions win over everything.
        if self.exclude.iter().any(|p| Self::matches(p, candidate)) {
            return Some(SkipReason::Excluded);
        }

        // 2. Holds and pins.
        if let Some(hold) = self
            .holds
            .iter()
            .find(|h| Self::matches(&h.package, candidate))
        {
            match &hold.pin {
                // Pinned to exactly this version: allow only that one through.
                Some(pin) if pin.trim() == candidate.available.raw().trim() => {}
                _ => {
                    return Some(SkipReason::Held {
                        note: hold.note.clone(),
                    })
                }
            }
        }

        // 3. Elevation constraints. Store apps silently corrupt their install
        //    state when driven from an elevated process, so this is a hard no.
        if candidate.id.backend.forbids_elevation() && self.elevated {
            return Some(SkipReason::RequiresUnelevated);
        }
        if candidate.id.backend.requires_elevation() && !self.elevated {
            return Some(SkipReason::RequiresElevation);
        }

        // 4. Version sanity. This is the core guard.
        if self.require_known_versions {
            if !candidate.installed.is_known() {
                return Some(SkipReason::UnknownInstalledVersion);
            }
            if !candidate.available.is_known() {
                return Some(SkipReason::UnknownAvailableVersion);
            }
        }

        // 5. Never move sideways or backwards. `is_upgrade_to` returns false
        //    for unknown versions, so this also catches the case where the
        //    user disabled `require_known_versions`.
        if !candidate.installed.is_upgrade_to(&candidate.available) {
            return Some(SkipReason::NotAnUpgrade);
        }

        // 6. Stable channel enforcement.
        if self.stable_only && candidate.available.is_prerelease() {
            return Some(SkipReason::PrereleaseBlocked {
                version: candidate.available.raw().to_string(),
            });
        }

        None
    }

    /// Evaluate a whole batch, preserving input order.
    pub fn plan(&self, candidates: Vec<UpdateCandidate>) -> Vec<PlannedUpdate> {
        candidates
            .into_iter()
            .map(|candidate| {
                let blocked_by = self.evaluate(&candidate);
                PlannedUpdate {
                    candidate,
                    blocked_by,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BackendKind, PackageId};
    use crate::version::Version;

    fn candidate(installed: &str, available: &str) -> UpdateCandidate {
        UpdateCandidate {
            id: PackageId::new(BackendKind::Winget, "Mozilla.Firefox"),
            name: "Firefox".into(),
            installed: Version::parse(installed),
            available: Version::parse(available),
            size_bytes: None,
            expected_sha256: None,
        }
    }

    fn policy() -> Policy {
        Policy::default()
    }

    #[test]
    fn a_genuine_upgrade_is_allowed() {
        assert_eq!(policy().evaluate(&candidate("1.0.0", "1.1.0")), None);
    }

    #[test]
    fn unknown_installed_version_is_refused() {
        // The exact case that corrupted installs before: winget reports
        // "Unknown" and the old code upgraded anyway.
        assert_eq!(
            policy().evaluate(&candidate("Unknown", "1.1.0")),
            Some(SkipReason::UnknownInstalledVersion)
        );
    }

    #[test]
    fn unknown_available_version_is_refused() {
        assert_eq!(
            policy().evaluate(&candidate("1.0.0", "")),
            Some(SkipReason::UnknownAvailableVersion)
        );
    }

    #[test]
    fn downgrades_are_refused() {
        assert_eq!(
            policy().evaluate(&candidate("2.0.0", "1.9.0")),
            Some(SkipReason::NotAnUpgrade)
        );
    }

    #[test]
    fn same_version_is_refused() {
        assert_eq!(
            policy().evaluate(&candidate("1.2.3", "1.2.3")),
            Some(SkipReason::NotAnUpgrade)
        );
        // ...including when written with different padding.
        assert_eq!(
            policy().evaluate(&candidate("1.2", "1.2.0")),
            Some(SkipReason::NotAnUpgrade)
        );
    }

    #[test]
    fn lexical_downgrade_trap_is_refused() {
        // 1.10.0 -> 1.9.0 looks like an upgrade to a string comparison.
        assert_eq!(
            policy().evaluate(&candidate("1.10.0", "1.9.0")),
            Some(SkipReason::NotAnUpgrade)
        );
    }

    #[test]
    fn prereleases_are_blocked_by_default_and_allowed_when_opted_in() {
        let c = candidate("1.0.0", "2.0.0-beta.1");
        assert_eq!(
            policy().evaluate(&c),
            Some(SkipReason::PrereleaseBlocked {
                version: "2.0.0-beta.1".into()
            })
        );

        let opted_in = Policy {
            stable_only: false,
            ..Policy::default()
        };
        assert_eq!(opted_in.evaluate(&c), None);
    }

    #[test]
    fn disabling_known_version_checks_still_refuses_unorderable_pairs() {
        // Even with the guard off, an unknown version can never be *proven*
        // newer, so it must not install.
        let loose = Policy {
            require_known_versions: false,
            ..Policy::default()
        };
        assert_eq!(
            loose.evaluate(&candidate("Unknown", "1.1.0")),
            Some(SkipReason::NotAnUpgrade)
        );
    }

    #[test]
    fn exclusions_match_qualified_and_bare_ids() {
        let by_bare = Policy {
            exclude: vec!["Mozilla.Firefox".into()],
            ..Policy::default()
        };
        assert_eq!(
            by_bare.evaluate(&candidate("1.0.0", "1.1.0")),
            Some(SkipReason::Excluded)
        );

        let by_qualified = Policy {
            exclude: vec!["winget:Mozilla.Firefox".into()],
            ..Policy::default()
        };
        assert_eq!(
            by_qualified.evaluate(&candidate("1.0.0", "1.1.0")),
            Some(SkipReason::Excluded)
        );
    }

    #[test]
    fn exclusion_matching_is_case_insensitive() {
        let p = Policy {
            exclude: vec!["mozilla.firefox".into()],
            ..Policy::default()
        };
        assert_eq!(
            p.evaluate(&candidate("1.0.0", "1.1.0")),
            Some(SkipReason::Excluded)
        );
    }

    #[test]
    fn a_hold_without_a_pin_freezes_the_package() {
        let p = Policy {
            holds: vec![Hold {
                package: "Mozilla.Firefox".into(),
                pin: None,
                note: Some("breaks my extension".into()),
            }],
            ..Policy::default()
        };
        assert_eq!(
            p.evaluate(&candidate("1.0.0", "1.1.0")),
            Some(SkipReason::Held {
                note: Some("breaks my extension".into())
            })
        );
    }

    #[test]
    fn a_pin_admits_only_the_pinned_version() {
        let p = Policy {
            holds: vec![Hold {
                package: "Mozilla.Firefox".into(),
                pin: Some("1.1.0".into()),
                note: None,
            }],
            ..Policy::default()
        };
        assert_eq!(p.evaluate(&candidate("1.0.0", "1.1.0")), None);
        assert_eq!(
            p.evaluate(&candidate("1.0.0", "1.2.0")),
            Some(SkipReason::Held { note: None })
        );
    }

    #[test]
    fn store_apps_are_refused_while_elevated() {
        let mut c = candidate("1.0.0", "1.1.0");
        c.id = PackageId::new(BackendKind::MsStore, "9NBLGGH4NNS1");
        let p = Policy {
            elevated: true,
            ..Policy::default()
        };
        assert_eq!(p.evaluate(&c), Some(SkipReason::RequiresUnelevated));

        let unelevated = Policy::default();
        assert_eq!(unelevated.evaluate(&c), None);
    }

    #[test]
    fn driver_updates_are_refused_without_elevation() {
        let mut c = candidate("1.0.0", "1.1.0");
        c.id = PackageId::new(BackendKind::WindowsDrivers, "KB5001234");
        assert_eq!(policy().evaluate(&c), Some(SkipReason::RequiresElevation));

        let elevated = Policy {
            elevated: true,
            ..Policy::default()
        };
        assert_eq!(elevated.evaluate(&c), None);
    }

    #[test]
    fn exclusion_takes_precedence_over_every_other_rule() {
        let p = Policy {
            exclude: vec!["Mozilla.Firefox".into()],
            holds: vec![Hold {
                package: "Mozilla.Firefox".into(),
                pin: None,
                note: None,
            }],
            ..Policy::default()
        };
        assert_eq!(
            p.evaluate(&candidate("Unknown", "bad")),
            Some(SkipReason::Excluded)
        );
    }

    #[test]
    fn plan_preserves_order_and_marks_each_entry() {
        let plan = policy().plan(vec![
            candidate("1.0.0", "1.1.0"),
            candidate("Unknown", "1.1.0"),
            candidate("2.0.0", "1.0.0"),
        ]);
        assert_eq!(plan.len(), 3);
        assert!(plan[0].is_actionable());
        assert!(!plan[1].is_actionable());
        assert!(!plan[2].is_actionable());
    }
}
