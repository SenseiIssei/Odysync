//! The model and the safety rules for disk cleanup.
//!
//! Cleanup is the part of a maintenance tool that can destroy a user's data,
//! so the rules live here — pure, unit-tested on every platform — rather than
//! inside the code that calls `remove_file`.
//!
//! Three invariants, in the order they matter:
//!
//! 1. **Scan, then delete what was ticked.** Never the other way round. A
//!    category reports its size and file count first; deleting happens only
//!    for categories the caller passed back in.
//! 2. **Nothing outside a declared root.** Every candidate path is checked
//!    against its category's root with [`is_within`], and the roots themselves
//!    are checked with [`is_safe_cleanup_root`] so a bug or a hostile
//!    environment variable cannot aim the cleaner at `C:\Windows`.
//! 3. **Say what cannot be undone.** [`Reversibility`] is part of the model,
//!    not a note in the UI, so a front-end cannot forget to show it.
//!
//! What is deliberately absent: anything that edits the registry to "clean" it,
//! any memory optimiser, and any TCP tuning. See `PLAN.md` §4.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How recoverable a category's contents are once removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reversibility {
    /// Regenerated automatically the next time it is needed. A thumbnail cache
    /// rebuilds itself; the only cost is being slower once.
    Regenerated,
    /// Gone, but it was never user data — a log, a crash dump, a stale
    /// installer.
    Discardable,
    /// Removes a capability the user may be relying on. `Windows.old` is the
    /// case that matters: deleting it reclaims tens of gigabytes and
    /// permanently removes the ability to roll back the Windows upgrade.
    LosesCapability,
}

impl Reversibility {
    /// Whether a front-end must obtain a separate, explicit confirmation.
    pub fn needs_explicit_confirmation(&self) -> bool {
        matches!(self, Reversibility::LosesCapability)
    }
}

/// One kind of reclaimable space.
///
/// Ids are stable and locale-independent; they appear in the CLI, the config
/// and the GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupCategory {
    /// `%TEMP%`, `%TMP%` and `C:\Windows\Temp`.
    TempFiles,
    /// The Recycle Bin.
    RecycleBin,
    /// Windows Update's downloaded payloads, once installed.
    WindowsUpdateCache,
    /// Delivery Optimization's peer-to-peer cache.
    DeliveryOptimization,
    /// The component store's superseded components (`DISM /StartComponentCleanup`).
    ComponentStore,
    /// Explorer's thumbnail and icon caches.
    ThumbnailCache,
    /// Windows Error Reporting queues and archives.
    ErrorReports,
    /// Kernel and application crash dumps.
    CrashDumps,
    /// The previous Windows installation.
    PreviousWindows,
    /// Browser caches, per profile. Never cookies, never saved passwords.
    BrowserCache,
    /// Per-user application caches: `~/.cache` on Linux, `~/Library/Caches`
    /// on macOS. By definition regenerable, but opt-in, because "every app
    /// on the machine is slow once" is not what anybody means by "clean".
    AppCaches,
    /// The list of recently opened documents — a privacy trace, not a space
    /// saver. Only the shortcuts; never the documents they point at.
    RecentItems,
    /// Compiled GPU shaders: DirectX's own cache and the NVIDIA, AMD and
    /// Intel driver caches. These grow without bound on a gaming machine —
    /// tens of gigabytes is common — and rebuild themselves.
    ShaderCache,
}

impl CleanupCategory {
    /// Every category, in the order a UI should present them: safest first.
    pub const ALL: &'static [CleanupCategory] = &[
        CleanupCategory::TempFiles,
        CleanupCategory::ThumbnailCache,
        CleanupCategory::RecentItems,
        CleanupCategory::RecycleBin,
        CleanupCategory::ErrorReports,
        CleanupCategory::CrashDumps,
        CleanupCategory::WindowsUpdateCache,
        CleanupCategory::DeliveryOptimization,
        CleanupCategory::BrowserCache,
        CleanupCategory::ShaderCache,
        CleanupCategory::AppCaches,
        CleanupCategory::ComponentStore,
        CleanupCategory::PreviousWindows,
    ];

    /// Whether this category means anything on the platform Odysync runs on.
    ///
    /// The Windows component store has no Linux counterpart; listing it there
    /// as "0 B" would read as "checked, nothing to do", which is untrue.
    pub fn applies_to_this_platform(&self) -> bool {
        match self {
            CleanupCategory::TempFiles => cfg!(any(windows, target_os = "macos")),
            CleanupCategory::RecycleBin | CleanupCategory::BrowserCache => true,
            CleanupCategory::ThumbnailCache => cfg!(any(windows, target_os = "linux")),
            CleanupCategory::ErrorReports => cfg!(any(windows, target_os = "macos")),
            CleanupCategory::RecentItems => cfg!(any(windows, target_os = "linux")),
            CleanupCategory::AppCaches => cfg!(unix),
            CleanupCategory::WindowsUpdateCache
            | CleanupCategory::DeliveryOptimization
            | CleanupCategory::ComponentStore
            | CleanupCategory::CrashDumps
            | CleanupCategory::ShaderCache
            | CleanupCategory::PreviousWindows => cfg!(windows),
        }
    }

    /// The categories that apply here, in presentation order.
    pub fn for_this_platform() -> Vec<CleanupCategory> {
        CleanupCategory::ALL
            .iter()
            .copied()
            .filter(CleanupCategory::applies_to_this_platform)
            .collect()
    }

    pub fn id(&self) -> &'static str {
        match self {
            CleanupCategory::TempFiles => "temp-files",
            CleanupCategory::RecycleBin => "recycle-bin",
            CleanupCategory::WindowsUpdateCache => "windows-update-cache",
            CleanupCategory::DeliveryOptimization => "delivery-optimization",
            CleanupCategory::ComponentStore => "component-store",
            CleanupCategory::ThumbnailCache => "thumbnail-cache",
            CleanupCategory::ErrorReports => "error-reports",
            CleanupCategory::CrashDumps => "crash-dumps",
            CleanupCategory::PreviousWindows => "previous-windows",
            CleanupCategory::BrowserCache => "browser-cache",
            CleanupCategory::AppCaches => "app-caches",
            CleanupCategory::RecentItems => "recent-items",
            CleanupCategory::ShaderCache => "shader-cache",
        }
    }

    pub fn from_id(id: &str) -> Option<CleanupCategory> {
        CleanupCategory::ALL
            .iter()
            .copied()
            .find(|c| c.id().eq_ignore_ascii_case(id.trim()))
    }

    pub fn label(&self) -> &'static str {
        match self {
            CleanupCategory::TempFiles => "Temporary files",
            CleanupCategory::RecycleBin => {
                if cfg!(windows) {
                    "Recycle Bin"
                } else {
                    "Trash"
                }
            }
            CleanupCategory::WindowsUpdateCache => "Windows Update downloads",
            CleanupCategory::DeliveryOptimization => "Delivery Optimization cache",
            CleanupCategory::ComponentStore => "Superseded Windows components",
            CleanupCategory::ThumbnailCache => "Thumbnail and icon caches",
            CleanupCategory::ErrorReports => "Windows error reports",
            CleanupCategory::CrashDumps => "Crash dumps",
            CleanupCategory::PreviousWindows => "Previous Windows installation",
            CleanupCategory::BrowserCache => "Browser caches",
            CleanupCategory::AppCaches => "Application caches",
            CleanupCategory::RecentItems => "Recently opened files list",
            CleanupCategory::ShaderCache => "GPU shader caches",
        }
    }

    /// What removing this actually costs, in one sentence.
    pub fn consequence(&self) -> &'static str {
        match self {
            CleanupCategory::TempFiles => {
                "Files applications left behind. A program still using one keeps it."
            }
            CleanupCategory::RecycleBin => {
                "Permanently deletes everything currently in the bin or trash."
            }
            CleanupCategory::WindowsUpdateCache => {
                "Already-installed update payloads. Windows re-downloads if it needs them again."
            }
            CleanupCategory::DeliveryOptimization => {
                "Peer-to-peer update cache. Rebuilds itself; other machines on \
                 the network may re-download from Microsoft instead."
            }
            CleanupCategory::ComponentStore => {
                "Superseded component versions. Windows updates installed before \
                 this point can no longer be uninstalled."
            }
            CleanupCategory::ThumbnailCache => {
                "Explorer rebuilds these on demand; folders browse slower once."
            }
            CleanupCategory::ErrorReports => "Queued crash reports that were never sent.",
            CleanupCategory::CrashDumps => {
                "Memory dumps from past crashes. Delete only if nobody is still diagnosing one."
            }
            CleanupCategory::PreviousWindows => {
                "Removes the ability to roll back to the previous Windows version, permanently."
            }
            CleanupCategory::BrowserCache => {
                "Cached page assets only — never cookies, logins or saved passwords. \
                 Sites load slower once."
            }
            CleanupCategory::AppCaches => {
                "Caches every application keeps for itself. Apps rebuild them, \
                 so each is slower the next time it starts."
            }
            CleanupCategory::RecentItems => {
                "Clears the recently-opened list in Explorer and file dialogs. \
                 The files themselves are not touched; pinned items stay."
            }
            CleanupCategory::ShaderCache => {
                "Compiled shaders for games and apps. They are rebuilt on demand, \
                 so games may stutter briefly the first time they run afterwards."
            }
        }
    }

    pub fn reversibility(&self) -> Reversibility {
        match self {
            CleanupCategory::TempFiles
            | CleanupCategory::ThumbnailCache
            | CleanupCategory::DeliveryOptimization
            | CleanupCategory::BrowserCache
            | CleanupCategory::AppCaches
            | CleanupCategory::ShaderCache
            | CleanupCategory::WindowsUpdateCache => Reversibility::Regenerated,

            CleanupCategory::ErrorReports
            | CleanupCategory::CrashDumps
            | CleanupCategory::RecentItems => Reversibility::Discardable,

            // The Recycle Bin is the user's own undo. Emptying it is exactly
            // the action they would otherwise take deliberately, and what it
            // destroys is their data, not a cache.
            CleanupCategory::RecycleBin
            | CleanupCategory::ComponentStore
            | CleanupCategory::PreviousWindows => Reversibility::LosesCapability,
        }
    }

    /// Whether Odysync measures this but refuses to remove it itself.
    ///
    /// `Windows.old` is owned by TrustedInstaller, so deleting it means taking
    /// ownership of tens of thousands of files and recursing through them.
    /// Windows already ships a tool that does this correctly — Disk Cleanup,
    /// and Settings → Storage — and a half-working reimplementation of it
    /// would fail partway through and leave the tree in a state neither tool
    /// understands. Reporting the size is the useful part; the deletion is
    /// somebody else's job.
    pub fn is_report_only(&self) -> bool {
        matches!(self, CleanupCategory::PreviousWindows)
    }

    /// Whether this category is selected when the user has expressed no
    /// preference.
    ///
    /// Only things that regenerate themselves. Anything that destroys data or
    /// removes a rollback path must be chosen deliberately, so a user who
    /// clicks the obvious button does not lose their Recycle Bin.
    pub fn selected_by_default(&self) -> bool {
        !self.is_report_only()
            && self.reversibility() == Reversibility::Regenerated
            && !matches!(
                self,
                CleanupCategory::BrowserCache
                    | CleanupCategory::AppCaches
                    | CleanupCategory::ShaderCache
            )
    }
}

/// What a scan found for one category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupFinding {
    pub category: CleanupCategory,
    pub bytes: u64,
    pub file_count: usize,
    /// Extra context, e.g. which browser profiles contributed, or why the
    /// size is an estimate.
    pub note: Option<String>,
    /// Set when the category could not be measured, rather than being empty.
    ///
    /// "0 bytes" and "could not look" must never render the same way.
    pub error: Option<String>,
}

impl CleanupFinding {
    pub fn empty(category: CleanupCategory) -> Self {
        Self {
            category,
            bytes: 0,
            file_count: 0,
            note: None,
            error: None,
        }
    }

    pub fn failed(category: CleanupCategory, error: impl Into<String>) -> Self {
        Self {
            category,
            bytes: 0,
            file_count: 0,
            note: None,
            error: Some(error.into()),
        }
    }

    /// Whether this is worth offering to the user at all.
    pub fn is_actionable(&self) -> bool {
        self.error.is_none() && self.bytes > 0
    }
}

/// What removing one category actually did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CleanupOutcome {
    pub removed_bytes: u64,
    pub removed_files: usize,
    /// Paths that could not be removed, with the reason. A file held open by a
    /// running program is the normal case, not an error worth failing over.
    pub skipped: Vec<String>,
}

/// Total reclaimable space across findings that are safe by default.
pub fn default_reclaimable(findings: &[CleanupFinding]) -> u64 {
    findings
        .iter()
        .filter(|f| f.is_actionable() && f.category.selected_by_default())
        .map(|f| f.bytes)
        .sum()
}

// ── Path safety ─────────────────────────────────────────────────────────────

/// Resolve `.` and `..` lexically, without touching the filesystem.
///
/// Deliberately not `canonicalize`: that requires the path to exist, which is
/// false for a candidate we are about to check and reject, and it would make
/// the safety rule untestable on a platform where the path does not exist.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Popping past the root leaves the root, matching how the OS
                // resolves `C:\..`.
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir) | Some(Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Is `candidate` inside `root`, after resolving `..`?
///
/// Case-insensitive on Windows, because `C:\Users\Jakob` and `c:\users\jakob`
/// are the same directory there and a case-sensitive check would be a way past
/// the guard.
///
/// A path equal to the root counts as inside it: emptying a cache directory is
/// an operation on the directory itself.
pub fn is_within(root: &Path, candidate: &Path) -> bool {
    let root = normalize(root);
    let candidate = normalize(candidate);

    if cfg!(windows) {
        let root = root.to_string_lossy().to_ascii_lowercase();
        let candidate = candidate.to_string_lossy().to_ascii_lowercase();
        let root = root.trim_end_matches('\\');
        // Compare component-wise via the separator so `C:\Foo` does not
        // contain `C:\Foobar`.
        candidate == root || candidate.starts_with(&format!("{root}\\"))
    } else {
        candidate == root || candidate.starts_with(&root)
    }
}

/// Would it ever be acceptable to delete things under `root`?
///
/// A cleanup root is derived from environment variables, so a stripped or
/// hostile environment can make one collapse to something catastrophic —
/// `%LOCALAPPDATA%` unset turns `%LOCALAPPDATA%\Temp` into `\Temp`, and a
/// naive join produces `C:\Temp` or worse. This refuses the shapes that are
/// never a cache:
///
///   * a filesystem root or a bare drive
///   * anything with fewer than two path components below the drive
///   * the Windows, System32 or Program Files trees
///   * a user's profile root, Documents, Desktop, Pictures or Downloads
pub fn is_safe_cleanup_root(root: &Path) -> bool {
    if cfg!(windows) {
        is_safe_windows_root(root)
    } else {
        is_safe_unix_root(root)
    }
}

/// The Unix rules: never a system tree, never a home directory itself, never
/// a user's own documents.
///
/// `~/.cache` and `~/Library/Caches` are three levels down, `/tmp` is one —
/// and `/tmp` is shared between users and holds live sockets, so a cleaner
/// has no business there even though it is called "temp".
fn is_safe_unix_root(root: &Path) -> bool {
    let normalized = normalize(root);
    if !normalized.has_root() {
        return false;
    }
    let parts: Vec<String> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.len() < 3 {
        // `/`, `/tmp`, `/home/jakob`, `/Users/jakob`, `/var/tmp`.
        return false;
    }

    const SYSTEM_TREES: &[&str] = &[
        "bin",
        "boot",
        "dev",
        "etc",
        "lib",
        "lib32",
        "lib64",
        "opt",
        "proc",
        "root",
        "run",
        "sbin",
        "srv",
        "sys",
        "usr",
        "System",
        "Library",
        "Applications",
        "Volumes",
        "private",
        "snap",
        "nix",
    ];
    // `/var/folders/...` is where macOS puts each user's `$TMPDIR`; the rest
    // of `/var` is system state.
    if parts[0] == "var" {
        return parts.get(1).map(String::as_str) == Some("folders");
    }
    if SYSTEM_TREES.contains(&parts[0].as_str()) {
        return false;
    }

    const FORBIDDEN_LEAVES: &[&str] = &[
        "documents",
        "desktop",
        "downloads",
        "pictures",
        "videos",
        "movies",
        "music",
        "public",
        "templates",
    ];
    let leaf = parts[parts.len() - 1].to_ascii_lowercase();
    !FORBIDDEN_LEAVES.contains(&leaf.as_str())
}

fn is_safe_windows_root(root: &Path) -> bool {
    let normalized = normalize(root);

    // Depth: a real cache lives at least two levels down, e.g.
    // `C:\Users\x\AppData\Local\Temp`. `C:\Temp` is one, and refusing it costs
    // nothing because Windows does not put a cache there.
    let depth = normalized
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    if depth < 2 {
        return false;
    }

    let lowered = normalized.to_string_lossy().to_ascii_lowercase();
    let lowered = lowered.replace('/', "\\");

    // Trees that are never a cleanup target, whatever the environment says.
    const FORBIDDEN_TREES: &[&str] = &[
        "\\windows\\system32",
        "\\windows\\syswow64",
        "\\program files",
        "\\program files (x86)",
        "\\programdata\\microsoft\\windows\\start menu",
        "\\system volume information",
        "\\$recycle.bin",
    ];
    for tree in FORBIDDEN_TREES {
        if lowered.contains(tree) {
            return false;
        }
    }

    // A user's own documents are not junk, however much of the disk they use.
    const FORBIDDEN_LEAVES: &[&str] = &[
        "documents",
        "desktop",
        "downloads",
        "pictures",
        "videos",
        "music",
        "onedrive",
    ];
    if let Some(last) = normalized
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
    {
        if FORBIDDEN_LEAVES.contains(&last.as_str()) {
            return false;
        }
    }

    // `C:\Windows` itself is refused, but `C:\Windows\Temp` is a legitimate
    // target, so this checks for the tree ending there rather than containing
    // it.
    if lowered.ends_with("\\windows") {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    // ── Category metadata ───────────────────────────────────────────────────

    #[test]
    fn every_category_has_a_unique_id_that_round_trips() {
        let mut ids: Vec<&str> = CleanupCategory::ALL.iter().map(|c| c.id()).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "two categories share an id");

        for c in CleanupCategory::ALL {
            assert_eq!(CleanupCategory::from_id(c.id()), Some(*c));
            assert!(!c.label().is_empty());
            assert!(
                c.consequence().len() > 20,
                "{} has a stub consequence",
                c.id()
            );
        }
        assert_eq!(CleanupCategory::from_id("not-a-category"), None);
        assert_eq!(
            CleanupCategory::from_id("  Temp-Files "),
            Some(CleanupCategory::TempFiles)
        );
    }

    /// The default selection is the whole safety story of a one-click button.
    #[test]
    fn nothing_that_destroys_data_is_selected_by_default() {
        for c in CleanupCategory::ALL {
            if c.reversibility() != Reversibility::Regenerated {
                assert!(
                    !c.selected_by_default(),
                    "{} destroys something and is on by default",
                    c.id()
                );
            }
        }
        // Specifically, the three that would hurt.
        assert!(!CleanupCategory::RecycleBin.selected_by_default());
        assert!(!CleanupCategory::PreviousWindows.selected_by_default());
        assert!(!CleanupCategory::ComponentStore.selected_by_default());
    }

    #[test]
    fn browser_caches_are_opt_in_even_though_they_regenerate() {
        // Clearing a cache logs nobody out, but it does make every site slow
        // for a while, and the user did not ask for that by clicking "clean".
        assert_eq!(
            CleanupCategory::BrowserCache.reversibility(),
            Reversibility::Regenerated
        );
        assert!(!CleanupCategory::BrowserCache.selected_by_default());
    }

    /// A report-only category must never end up in a delete request, and the
    /// default selection is the most likely way one would.
    #[test]
    fn a_report_only_category_is_never_selected_by_default() {
        assert!(CleanupCategory::PreviousWindows.is_report_only());
        assert!(!CleanupCategory::PreviousWindows.selected_by_default());

        for c in CleanupCategory::ALL {
            if c.is_report_only() {
                assert!(
                    !c.selected_by_default(),
                    "{} is report-only yet default-on",
                    c.id()
                );
            }
        }
    }

    #[test]
    fn losing_a_capability_demands_its_own_confirmation() {
        assert!(Reversibility::LosesCapability.needs_explicit_confirmation());
        assert!(!Reversibility::Regenerated.needs_explicit_confirmation());
        assert!(!Reversibility::Discardable.needs_explicit_confirmation());

        assert!(CleanupCategory::PreviousWindows
            .reversibility()
            .needs_explicit_confirmation());
    }

    // ── Findings ────────────────────────────────────────────────────────────

    #[test]
    fn a_failed_measurement_is_not_an_empty_one() {
        let empty = CleanupFinding::empty(CleanupCategory::TempFiles);
        let failed = CleanupFinding::failed(CleanupCategory::TempFiles, "access denied");

        assert!(!empty.is_actionable());
        assert!(!failed.is_actionable());
        // The distinction a UI must be able to make: nothing to do, versus we
        // could not look.
        assert!(empty.error.is_none());
        assert!(failed.error.is_some());
    }

    #[test]
    fn default_reclaimable_counts_only_safe_actionable_categories() {
        let findings = vec![
            CleanupFinding {
                category: CleanupCategory::TempFiles,
                bytes: 1_000,
                file_count: 5,
                note: None,
                error: None,
            },
            // Not selected by default: must not be counted.
            CleanupFinding {
                category: CleanupCategory::RecycleBin,
                bytes: 9_000,
                file_count: 3,
                note: None,
                error: None,
            },
            // Failed to measure: must not be counted.
            CleanupFinding {
                bytes: 5_000,
                ..CleanupFinding::failed(CleanupCategory::ThumbnailCache, "denied")
            },
        ];
        assert_eq!(default_reclaimable(&findings), 1_000);
    }

    // ── Path normalisation ──────────────────────────────────────────────────

    #[cfg(windows)]
    #[test]
    fn normalize_resolves_dot_and_dotdot() {
        assert_eq!(normalize(&p(r"C:\a\b\..\c")), p(r"C:\a\c"));
        assert_eq!(normalize(&p(r"C:\a\.\b")), p(r"C:\a\b"));
        assert_eq!(normalize(&p(r"C:\a\b\..\..")), p(r"C:\"));
    }

    #[cfg(windows)]
    #[test]
    fn normalize_cannot_climb_above_the_root() {
        // `C:\..` is `C:\` to the OS, and must be here too, or the containment
        // check could be fooled into thinking an escape stayed inside.
        assert_eq!(normalize(&p(r"C:\..\..\..")), p(r"C:\"));
    }

    // ── Containment ─────────────────────────────────────────────────────────

    #[cfg(windows)]
    #[test]
    fn a_path_inside_the_root_is_within_it() {
        assert!(is_within(&p(r"C:\cache"), &p(r"C:\cache\a\b.tmp")));
        assert!(is_within(&p(r"C:\cache"), &p(r"C:\cache")));
    }

    #[cfg(windows)]
    #[test]
    fn traversal_out_of_the_root_is_refused() {
        assert!(!is_within(&p(r"C:\cache"), &p(r"C:\cache\..\..\Windows")));
        assert!(!is_within(&p(r"C:\cache"), &p(r"C:\Windows\System32")));
    }

    /// The classic prefix bug: `C:\Foo` must not appear to contain `C:\Foobar`.
    #[cfg(windows)]
    #[test]
    fn a_sibling_with_a_shared_prefix_is_not_inside() {
        assert!(!is_within(&p(r"C:\cache"), &p(r"C:\cache-old\a.tmp")));
        assert!(!is_within(&p(r"C:\cache"), &p(r"C:\cacheable")));
    }

    #[cfg(windows)]
    #[test]
    fn containment_ignores_case_on_windows() {
        // A case-sensitive check would be a way straight past the guard, since
        // the OS treats these as the same directory.
        assert!(is_within(&p(r"C:\Cache"), &p(r"c:\cache\a.tmp")));
        assert!(is_within(&p(r"c:\cache"), &p(r"C:\CACHE\A.TMP")));
    }

    // ── Root safety ─────────────────────────────────────────────────────────

    #[cfg(windows)]
    #[test]
    fn a_legitimate_cache_root_is_accepted() {
        assert!(is_safe_cleanup_root(&p(
            r"C:\Users\jakob\AppData\Local\Temp"
        )));
        assert!(is_safe_cleanup_root(&p(r"C:\Windows\Temp")));
        assert!(is_safe_cleanup_root(&p(
            r"C:\Windows\SoftwareDistribution\Download"
        )));
    }

    /// The failure this exists for: an unset environment variable collapsing a
    /// root into something catastrophic.
    #[cfg(windows)]
    #[test]
    fn a_root_that_is_too_shallow_is_refused() {
        for root in [r"C:\", r"C:\Temp", r"\Temp", r"\", "C:"] {
            assert!(
                !is_safe_cleanup_root(&p(root)),
                "accepted a shallow root: {root}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn system_trees_are_never_cleanup_roots() {
        for root in [
            r"C:\Windows",
            r"C:\Windows\System32",
            r"C:\Windows\System32\config",
            r"C:\Windows\SysWOW64\drivers",
            r"C:\Program Files\Odysync",
            r"C:\Program Files (x86)\Something",
            r"C:\System Volume Information\x",
        ] {
            assert!(!is_safe_cleanup_root(&p(root)), "accepted {root}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_users_own_files_are_never_junk() {
        for root in [
            r"C:\Users\jakob\Documents",
            r"C:\Users\jakob\Desktop",
            r"C:\Users\jakob\Downloads",
            r"C:\Users\jakob\Pictures",
            r"C:\Users\jakob\OneDrive",
        ] {
            assert!(!is_safe_cleanup_root(&p(root)), "accepted {root}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn traversal_inside_a_root_is_resolved_before_it_is_judged() {
        // `...\Local\Temp\..\..\..\..\Windows\System32` must be seen for what
        // it resolves to, not for how it is spelled.
        assert!(!is_safe_cleanup_root(&p(
            r"C:\Users\jakob\AppData\Local\Temp\..\..\..\..\..\Windows\System32"
        )));
    }

    // ── Unix ────────────────────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn unix_paths_normalise_and_contain_like_windows_ones() {
        assert_eq!(normalize(&p("/home/a/.cache/../x")), p("/home/a/x"));
        assert_eq!(normalize(&p("/../../etc")), p("/etc"));
        assert!(is_within(&p("/home/a/.cache"), &p("/home/a/.cache/x/y")));
        assert!(!is_within(&p("/home/a/.cache"), &p("/home/a/.cache-old/x")));
        assert!(!is_within(
            &p("/home/a/.cache"),
            &p("/home/a/.cache/../.ssh/id")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn per_user_caches_and_trash_are_legitimate_roots_on_unix() {
        for root in [
            "/home/jakob/.cache",
            "/home/jakob/.cache/thumbnails",
            "/home/jakob/.local/share/Trash/files",
            "/Users/jakob/Library/Caches",
            "/Users/jakob/.Trash",
            "/var/folders/xy/abc123/T",
        ] {
            assert!(is_safe_cleanup_root(&p(root)), "refused {root}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn system_trees_homes_and_documents_are_refused_on_unix() {
        for root in [
            "/",
            "/tmp",
            "/var/tmp",
            "/home/jakob",
            "/Users/jakob",
            "/usr/lib/x",
            "/etc/apt/cache",
            "/Library/Caches/com.apple",
            "/System/Library/Caches",
            "/var/log/journal",
            "/home/jakob/Documents",
            "/Users/jakob/Downloads",
            "relative/path/cache",
            "/home/jakob/.cache/../../../etc",
        ] {
            assert!(!is_safe_cleanup_root(&p(root)), "accepted {root}");
        }
    }

    #[test]
    fn every_platform_has_cleanup_categories_and_hides_the_rest() {
        let here = CleanupCategory::for_this_platform();
        assert!(here.contains(&CleanupCategory::RecycleBin));
        assert!(here.contains(&CleanupCategory::BrowserCache));
        if cfg!(windows) {
            assert!(here.contains(&CleanupCategory::ComponentStore));
            assert!(!here.contains(&CleanupCategory::AppCaches));
        } else {
            assert!(!here.contains(&CleanupCategory::ComponentStore));
            assert!(here.contains(&CleanupCategory::AppCaches));
        }
    }

    #[test]
    fn privacy_and_app_caches_are_opt_in() {
        assert!(!CleanupCategory::RecentItems.selected_by_default());
        assert!(!CleanupCategory::AppCaches.selected_by_default());
        // Regenerates, but costs stutter in games the user did not expect.
        assert!(!CleanupCategory::ShaderCache.selected_by_default());
    }
}
