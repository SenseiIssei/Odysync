//! Disk cleanup: measure first, then remove only what was asked for.
//!
//! The rules and the model live in [`odysync_core::cleanup`], which is pure and
//! tested on every platform. This module is the part that touches the disk, and
//! it is deliberately dull: a declarative table of targets, a directory walk
//! that measures, and a delete that refuses anything the table did not name.
//!
//! ## The shape of the safety argument
//!
//! Every path removed here has to satisfy three separate checks, and they are
//! independent on purpose — a mistake in one is caught by the next:
//!
//! 1. The category's roots are built from environment variables, then filtered
//!    through [`is_safe_cleanup_root`]. An unset `%LOCALAPPDATA%` turning a
//!    root into `\Temp` is dropped here rather than acted on.
//! 2. The walk never follows a directory symlink, so a junction inside a cache
//!    cannot lead the delete somewhere else.
//! 3. Every individual path is re-checked with [`is_within`] immediately before
//!    removal, against the root it was found under.
//!
//! ## What is not here
//!
//! No registry writes, no memory optimiser, no TCP tuning — see `PLAN.md` §4.
//! `Windows.old` is measured and reported but never deleted: it is owned by
//! TrustedInstaller, and a half-working reimplementation of Disk Cleanup that
//! fails partway through is worse than sending the user to the tool that works.

use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::Duration;

use odysync_core::cleanup::{
    is_safe_cleanup_root, is_within, CleanupCategory, CleanupFinding, CleanupOutcome,
};
use odysync_core::error::Result;

/// DISM's component cleanup genuinely takes several minutes on a machine that
/// has been servicing itself for years.
#[cfg(windows)]
const DISM_TIMEOUT: Duration = Duration::from_secs(45 * 60);

/// How the contents of a target are identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Everything under the root.
    Contents,
    /// Only entries whose file name starts with one of these prefixes.
    ///
    /// The thumbnail cache shares `%LOCALAPPDATA%\Microsoft\Windows\Explorer`
    /// with other Explorer state, so clearing the whole directory would take
    /// more than was asked for.
    Prefixed(&'static [&'static str]),
}

/// One directory a category draws from.
#[derive(Debug, Clone)]
pub struct Target {
    pub category: CleanupCategory,
    pub root: PathBuf,
    pub strategy: Strategy,
}

/// Read an environment variable into a path, or `None` when it is unset.
///
/// Returning `None` rather than an empty path matters: joining onto an empty
/// path silently produces a root-relative one, which is how `%LOCALAPPDATA%`
/// being unset turns a cache path into `\Temp`.
fn env_path(name: &str) -> Option<PathBuf> {
    let value = std::env::var(name).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// The Windows directory, from the environment, falling back to `C:\Windows`.
fn windows_dir() -> PathBuf {
    env_path("SystemRoot")
        .or_else(|| env_path("windir"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

/// Every target on this machine, already filtered for root safety.
///
/// A root that does not exist is kept — measuring reports zero for it, which
/// is correct and keeps the table declarative. A root that is *unsafe* is
/// dropped and logged, because that indicates the environment lied to us.
pub fn targets() -> Vec<Target> {
    let mut out: Vec<Target> = Vec::new();
    let mut push = |category, root: PathBuf, strategy| {
        if is_safe_cleanup_root(&root) {
            out.push(Target {
                category,
                root,
                strategy,
            });
        } else {
            tracing::warn!(
                category = category.id(),
                root = %root.display(),
                "refusing an unsafe cleanup root; the environment may be incomplete"
            );
        }
    };

    let windows = windows_dir();

    // Temporary files. `%TEMP%` and `%TMP%` are usually the same directory;
    // duplicates are collapsed after the table is built.
    for var in ["TEMP", "TMP"] {
        if let Some(dir) = env_path(var) {
            push(CleanupCategory::TempFiles, dir, Strategy::Contents);
        }
    }
    push(
        CleanupCategory::TempFiles,
        windows.join("Temp"),
        Strategy::Contents,
    );

    if let Some(local) = env_path("LOCALAPPDATA") {
        push(
            CleanupCategory::ThumbnailCache,
            local.join(r"Microsoft\Windows\Explorer"),
            Strategy::Prefixed(&["thumbcache_", "iconcache_"]),
        );
        push(
            CleanupCategory::ErrorReports,
            local.join(r"Microsoft\Windows\WER"),
            Strategy::Contents,
        );
        push(
            CleanupCategory::CrashDumps,
            local.join("CrashDumps"),
            Strategy::Contents,
        );
    }

    if let Some(program_data) = env_path("ProgramData") {
        push(
            CleanupCategory::ErrorReports,
            program_data.join(r"Microsoft\Windows\WER"),
            Strategy::Contents,
        );
    }

    push(
        CleanupCategory::CrashDumps,
        windows.join("Minidump"),
        Strategy::Contents,
    );
    push(
        CleanupCategory::WindowsUpdateCache,
        windows.join(r"SoftwareDistribution\Download"),
        Strategy::Contents,
    );
    push(
        CleanupCategory::DeliveryOptimization,
        windows.join(r"ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache"),
        Strategy::Contents,
    );

    // Collapse duplicate roots — `%TEMP%` and `%TMP%` normally point at the
    // same place, and measuring it twice would double the reported total.
    out.dedup_by(|a, b| a.category == b.category && a.root == b.root);
    out.sort_by(|a, b| a.root.cmp(&b.root));
    out.dedup_by(|a, b| a.category == b.category && a.root == b.root);
    out
}

/// Does this entry belong to the target, per its strategy?
fn matches_strategy(strategy: Strategy, path: &Path) -> bool {
    match strategy {
        Strategy::Contents => true,
        Strategy::Prefixed(prefixes) => {
            let Some(name) = path
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
            else {
                return false;
            };
            prefixes.iter().any(|p| name.starts_with(p))
        }
    }
}

/// Bytes and file count under `dir`, never following a directory symlink.
///
/// A junction inside a cache — and Windows uses them liberally — would
/// otherwise make the walk wander into a tree the target never named, both
/// inflating the reported size and, worse, offering it up for deletion.
fn measure(dir: &Path, strategy: Strategy) -> (u64, usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };

    let mut bytes = 0;
    let mut files = 0;

    for entry in entries.flatten() {
        let path = entry.path();
        if !matches_strategy(strategy, &path) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };

        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            // Sub-entries of a matched directory all belong to it.
            let (b, f) = measure(&path, Strategy::Contents);
            bytes += b;
            files += f;
        } else if meta.is_file() {
            bytes += meta.len();
            files += 1;
        }
    }

    (bytes, files)
}

/// Remove everything under `dir` that the strategy matches.
///
/// Returns what was removed and what refused to go. A file held open by a
/// running program is the normal case on Windows, not a failure worth
/// aborting for, so it is recorded and the walk continues.
fn remove_matching(root: &Path, dir: &Path, strategy: Strategy, outcome: &mut CleanupOutcome) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !matches_strategy(strategy, &path) {
            continue;
        }

        // The third, independent check. The table said this root was safe and
        // the walk stayed inside it — this catches the case where neither of
        // those was true after all.
        if !is_within(root, &path) {
            tracing::error!(
                root = %root.display(),
                path = %path.display(),
                "refusing to delete a path outside its cleanup root"
            );
            outcome
                .skipped
                .push(format!("{}: outside its cleanup root", path.display()));
            continue;
        }

        let Ok(meta) = entry.metadata() else {
            continue;
        };

        // A symlink or junction is removed as a link, never recursed into.
        if meta.is_symlink() {
            let removed = if meta.is_dir() {
                std::fs::remove_dir(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if let Err(e) = removed {
                outcome.skipped.push(format!("{}: {e}", path.display()));
            }
            continue;
        }

        if meta.is_dir() {
            let (bytes, files) = measure(&path, Strategy::Contents);
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {
                    outcome.removed_bytes += bytes;
                    outcome.removed_files += files;
                }
                Err(e) => {
                    // Partial success is the norm: one locked file inside a
                    // directory fails the whole `remove_dir_all`, but the rest
                    // may well be gone. Re-measure instead of guessing.
                    let (left, _) = measure(&path, Strategy::Contents);
                    outcome.removed_bytes += bytes.saturating_sub(left);
                    outcome.skipped.push(format!("{}: {e}", path.display()));
                }
            }
        } else {
            let size = meta.len();
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    outcome.removed_bytes += size;
                    outcome.removed_files += 1;
                }
                Err(e) => outcome.skipped.push(format!("{}: {e}", path.display())),
            }
        }
    }
}

// ── Scan ────────────────────────────────────────────────────────────────────

/// Measure every category. Changes nothing.
pub async fn scan() -> Vec<CleanupFinding> {
    if !cfg!(windows) {
        return Vec::new();
    }

    tokio::task::spawn_blocking(scan_blocking)
        .await
        .unwrap_or_default()
}

fn scan_blocking() -> Vec<CleanupFinding> {
    let mut findings: Vec<CleanupFinding> = Vec::new();

    for category in CleanupCategory::ALL {
        let finding = match category {
            CleanupCategory::RecycleBin => measure_recycle_bin(),
            CleanupCategory::PreviousWindows => measure_previous_windows(),
            CleanupCategory::BrowserCache => measure_browser_caches(),
            // DISM does not report what it would reclaim without doing the
            // analysis, which itself takes minutes. Offering the action with
            // an honest "size unknown" beats blocking a scan on it.
            CleanupCategory::ComponentStore => CleanupFinding {
                note: Some(
                    "Size is only known once Windows analyses the component store, \
                     which takes several minutes."
                        .into(),
                ),
                ..CleanupFinding::empty(*category)
            },
            _ => {
                let mut bytes = 0;
                let mut files = 0;
                for target in targets().iter().filter(|t| t.category == *category) {
                    let (b, f) = measure(&target.root, target.strategy);
                    bytes += b;
                    files += f;
                }
                CleanupFinding {
                    category: *category,
                    bytes,
                    file_count: files,
                    note: None,
                    error: None,
                }
            }
        };
        findings.push(finding);
    }

    findings
}

/// Exact Recycle Bin size across every drive, via the shell's own accounting.
///
/// Walking `C:\$Recycle.Bin` would be wrong twice over: it is per-user and
/// permission-guarded, so the walk would under-report, and it is exactly the
/// kind of path the root-safety rule refuses.
#[cfg(windows)]
fn measure_recycle_bin() -> CleanupFinding {
    use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};

    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };

    // SAFETY: `info` is a correctly sized SHQUERYRBINFO; a null path means
    // "every drive", which is what the Recycle Bin means to a user.
    let hr = unsafe { SHQueryRecycleBinW(None, &mut info) };
    match hr {
        Ok(()) => CleanupFinding {
            category: CleanupCategory::RecycleBin,
            bytes: info.i64Size.max(0) as u64,
            file_count: info.i64NumItems.max(0) as usize,
            note: None,
            error: None,
        },
        Err(e) => CleanupFinding::failed(CleanupCategory::RecycleBin, e.to_string()),
    }
}

#[cfg(not(windows))]
fn measure_recycle_bin() -> CleanupFinding {
    CleanupFinding::empty(CleanupCategory::RecycleBin)
}

/// Measure `Windows.old`, which is reported but never removed.
fn measure_previous_windows() -> CleanupFinding {
    let root = windows_dir()
        .parent()
        .map(|p| p.join("Windows.old"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows.old"));

    if !root.is_dir() {
        return CleanupFinding::empty(CleanupCategory::PreviousWindows);
    }

    let (bytes, files) = measure(&root, Strategy::Contents);
    CleanupFinding {
        category: CleanupCategory::PreviousWindows,
        bytes,
        file_count: files,
        note: Some(
            "Reported only. These files are owned by TrustedInstaller; remove them \
             with Settings → System → Storage → Temporary files, which is built to \
             take ownership correctly."
                .into(),
        ),
        error: None,
    }
}

// ── Clean ───────────────────────────────────────────────────────────────────

/// Remove the categories in `selected`, and nothing else.
///
/// Categories the caller did not name are not touched, and a report-only
/// category is refused even if it was named — the caller having asked is not
/// enough when the operation is one this module has decided it cannot perform
/// correctly.
pub async fn clean(selected: Vec<CleanupCategory>) -> Vec<(CleanupCategory, CleanupOutcome)> {
    if !cfg!(windows) {
        return Vec::new();
    }

    let mut results = Vec::new();

    for category in selected {
        if category.is_report_only() {
            results.push((
                category,
                CleanupOutcome {
                    skipped: vec![format!(
                        "{} is reported only; Odysync does not remove it",
                        category.label()
                    )],
                    ..CleanupOutcome::default()
                },
            ));
            continue;
        }

        let outcome = match category {
            CleanupCategory::RecycleBin => empty_recycle_bin().await,
            CleanupCategory::ComponentStore => run_component_cleanup().await,
            CleanupCategory::BrowserCache => clean_browser_caches().await,
            other => tokio::task::spawn_blocking(move || {
                let mut outcome = CleanupOutcome::default();
                for target in targets().iter().filter(|t| t.category == other) {
                    remove_matching(&target.root, &target.root, target.strategy, &mut outcome);
                }
                outcome
            })
            .await
            .unwrap_or_default(),
        };

        tracing::info!(
            category = category.id(),
            bytes = outcome.removed_bytes,
            files = outcome.removed_files,
            skipped = outcome.skipped.len(),
            "cleanup category finished"
        );
        results.push((category, outcome));
    }

    results
}

#[cfg(windows)]
async fn empty_recycle_bin() -> CleanupOutcome {
    use windows::Win32::UI::Shell::{SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI};

    // Measured first, because once it is empty there is nothing left to count.
    let before = measure_recycle_bin();

    let result = tokio::task::spawn_blocking(|| {
        // SAFETY: a null window and null path mean "no UI, every drive".
        unsafe { SHEmptyRecycleBinW(None, None, SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI) }
    })
    .await;

    match result {
        Ok(Ok(())) => CleanupOutcome {
            removed_bytes: before.bytes,
            removed_files: before.file_count,
            skipped: Vec::new(),
        },
        Ok(Err(e)) => CleanupOutcome {
            skipped: vec![format!("could not empty the Recycle Bin: {e}")],
            ..CleanupOutcome::default()
        },
        Err(e) => CleanupOutcome {
            skipped: vec![format!("the Recycle Bin task panicked: {e}")],
            ..CleanupOutcome::default()
        },
    }
}

#[cfg(not(windows))]
async fn empty_recycle_bin() -> CleanupOutcome {
    CleanupOutcome::default()
}

/// `DISM /Online /Cleanup-Image /StartComponentCleanup`.
///
/// Deliberately without `/ResetBase`: that additionally makes every installed
/// update permanently un-uninstallable, which is a different and much larger
/// promise than "reclaim superseded components".
#[cfg(windows)]
async fn run_component_cleanup() -> CleanupOutcome {
    if !odysync_core::platform::is_elevated() {
        return CleanupOutcome {
            skipped: vec!["component cleanup needs administrator rights".into()],
            ..CleanupOutcome::default()
        };
    }

    let result = odysync_core::proc::run(
        "dism.exe",
        &["/Online", "/Cleanup-Image", "/StartComponentCleanup"],
        DISM_TIMEOUT,
    )
    .await;

    match result {
        // DISM does not report the bytes it reclaimed, and inventing a number
        // would be worse than admitting it.
        Ok(out) if out.success() => CleanupOutcome::default(),
        Ok(out) => CleanupOutcome {
            skipped: vec![format!("DISM exited with code {}", out.code)],
            ..CleanupOutcome::default()
        },
        Err(e) => CleanupOutcome {
            skipped: vec![format!("could not run DISM: {e}")],
            ..CleanupOutcome::default()
        },
    }
}

#[cfg(not(windows))]
async fn run_component_cleanup() -> CleanupOutcome {
    CleanupOutcome::default()
}

// ── Browsers ────────────────────────────────────────────────────────────────

/// A browser whose cache Odysync knows how to find.
struct Browser {
    /// Display name.
    name: &'static str,
    /// Process image name, used to refuse cleaning while it is running.
    process: &'static str,
    /// Directory holding the browser's profiles.
    profiles_root: Option<PathBuf>,
    /// Cache directories relative to a profile directory.
    cache_dirs: &'static [&'static str],
    /// Whether profiles are the immediate subdirectories of `profiles_root`.
    profiles_are_subdirs: bool,
}

fn browsers() -> Vec<Browser> {
    let local = env_path("LOCALAPPDATA");
    let roaming = env_path("APPDATA");

    vec![
        Browser {
            name: "Google Chrome",
            process: "chrome.exe",
            profiles_root: local.as_ref().map(|p| p.join(r"Google\Chrome\User Data")),
            // Never `Network`, `Cookies` or `Login Data` — those are the
            // things a user would be furious to lose.
            cache_dirs: &["Cache", "Code Cache", "GPUCache"],
            profiles_are_subdirs: true,
        },
        Browser {
            name: "Microsoft Edge",
            process: "msedge.exe",
            profiles_root: local.as_ref().map(|p| p.join(r"Microsoft\Edge\User Data")),
            cache_dirs: &["Cache", "Code Cache", "GPUCache"],
            profiles_are_subdirs: true,
        },
        Browser {
            name: "Mozilla Firefox",
            process: "firefox.exe",
            // Firefox keeps its cache under Local and its profiles under
            // Roaming; the cache tree mirrors the profile names.
            profiles_root: local
                .as_ref()
                .map(|p| p.join(r"Mozilla\Firefox\Profiles"))
                .or_else(|| {
                    roaming
                        .as_ref()
                        .map(|p| p.join(r"Mozilla\Firefox\Profiles"))
                }),
            cache_dirs: &["cache2"],
            profiles_are_subdirs: true,
        },
    ]
}

/// Cache directories that exist right now, per browser.
fn browser_cache_dirs() -> Vec<(&'static str, &'static str, PathBuf)> {
    let mut out = Vec::new();

    for browser in browsers() {
        let Some(root) = browser.profiles_root.clone() else {
            continue;
        };
        if !root.is_dir() {
            continue;
        }

        let profile_dirs: Vec<PathBuf> = if browser.profiles_are_subdirs {
            std::fs::read_dir(&root)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        } else {
            vec![root.clone()]
        };

        for profile in profile_dirs {
            for cache in browser.cache_dirs {
                let dir = profile.join(cache);
                if dir.is_dir() && is_safe_cleanup_root(&dir) {
                    out.push((browser.name, browser.process, dir));
                }
            }
        }
    }

    out
}

/// Is a process with this image name running?
///
/// Clearing a live browser's cache corrupts its index, so this is a refusal
/// rather than a warning.
#[cfg(windows)]
async fn is_process_running(image: &str) -> bool {
    // `tasklist /FI` filters server-side and returns a fixed-format line; the
    // "no tasks" message is localised, so the check is for the image name in
    // the output rather than for the absence of that message.
    let filter = format!("IMAGENAME eq {image}");
    match odysync_core::proc::run(
        "tasklist.exe",
        &["/FI", &filter, "/NH", "/FO", "CSV"],
        Duration::from_secs(30),
    )
    .await
    {
        Ok(out) => out
            .stdout
            .to_ascii_lowercase()
            .contains(&image.to_ascii_lowercase()),
        // Unknown is treated as running: refusing to clean a cache costs the
        // user some disk space, corrupting a live profile costs them more.
        Err(e) => {
            tracing::warn!(image, error = %e, "could not determine whether the browser is running");
            true
        }
    }
}

#[cfg(not(windows))]
async fn is_process_running(_image: &str) -> bool {
    true
}

fn measure_browser_caches() -> CleanupFinding {
    let dirs = browser_cache_dirs();
    if dirs.is_empty() {
        return CleanupFinding::empty(CleanupCategory::BrowserCache);
    }

    let mut bytes = 0;
    let mut files = 0;
    let mut names: Vec<&str> = Vec::new();

    for (name, _, dir) in &dirs {
        let (b, f) = measure(dir, Strategy::Contents);
        bytes += b;
        files += f;
        if !names.contains(name) {
            names.push(name);
        }
    }

    CleanupFinding {
        category: CleanupCategory::BrowserCache,
        bytes,
        file_count: files,
        note: Some(format!(
            "{}. Cached page assets only — cookies, logins and saved passwords are \
             never touched. A browser that is running is skipped.",
            names.join(", ")
        )),
        error: None,
    }
}

async fn clean_browser_caches() -> CleanupOutcome {
    let mut outcome = CleanupOutcome::default();
    let dirs = browser_cache_dirs();

    // Checked once per browser rather than once per profile directory.
    let mut checked: Vec<(&str, bool)> = Vec::new();
    for (name, process, _) in &dirs {
        if !checked.iter().any(|(n, _)| n == name) {
            checked.push((name, is_process_running(process).await));
        }
    }

    for (name, _, dir) in dirs {
        let running = checked
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, r)| *r)
            .unwrap_or(true);

        if running {
            outcome.skipped.push(format!(
                "{name} is running; close it before clearing its cache"
            ));
            continue;
        }

        remove_matching(&dir, &dir, Strategy::Contents, &mut outcome);
    }

    // The same "still running" line once per profile directory is noise.
    outcome.skipped.dedup();
    outcome
}

// ── Disk optimisation ───────────────────────────────────────────────────────

/// One volume and what optimising it would actually mean.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VolumeOptimization {
    pub drive: String,
    /// `SSD`, `HDD` or whatever Windows reported.
    pub media_type: String,
    /// The operation appropriate to the media: `Retrim` or `Defrag`.
    pub operation: String,
}

/// Which volumes could be optimised, and how.
///
/// The media type decides the operation and there is no user override:
/// defragmenting an SSD writes the whole drive for no benefit and spends its
/// erase cycles, so "defrag everything" is not offered.
#[cfg(windows)]
pub async fn plan_disk_optimization() -> Result<Vec<VolumeOptimization>> {
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$out = @()
foreach ($p in (Get-PhysicalDisk)) {
  foreach ($d in ($p | Get-Disk | Get-Partition | Get-Volume)) {
    if ($d.DriveLetter) {
      $out += [pscustomobject]@{
        Drive = "$($d.DriveLetter):"
        MediaType = "$($p.MediaType)"
      }
    }
  }
}
$out | ConvertTo-Json -Compress
"#;

    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "Drive")]
        drive: String,
        #[serde(rename = "MediaType")]
        media_type: String,
    }

    let out = odysync_core::proc::powershell(script, Duration::from_secs(120)).await?;
    let rows: Vec<Row> = odysync_backends_parse_json(&out.stdout);

    Ok(rows
        .into_iter()
        .map(|r| {
            let operation = if r.media_type.eq_ignore_ascii_case("SSD") {
                // TRIM tells the drive which blocks are free. Defragmenting it
                // would be pure wear for no gain.
                "Retrim"
            } else {
                "Defrag"
            };
            VolumeOptimization {
                drive: r.drive,
                media_type: r.media_type,
                operation: operation.to_string(),
            }
        })
        .collect())
}

#[cfg(not(windows))]
pub async fn plan_disk_optimization() -> Result<Vec<VolumeOptimization>> {
    Ok(Vec::new())
}

/// Run the optimisation each volume's media type calls for.
///
/// Needs elevation. `Optimize-Volume` chooses `-ReTrim` for an SSD and
/// `-Defrag` for spinning rust; the caller does not get to pick, because
/// defragmenting an SSD writes the whole drive for no benefit.
#[cfg(windows)]
pub async fn run_disk_optimization(plan: &[VolumeOptimization]) -> Vec<String> {
    let mut lines = Vec::new();

    if !odysync_core::platform::is_elevated() {
        return vec!["disk optimisation needs administrator rights".into()];
    }

    for volume in plan {
        let Some(letter) = volume.drive.chars().next() else {
            continue;
        };
        // The drive letter is the only interpolated value and it is a single
        // character taken from Windows' own output, so it cannot carry a
        // quote or a statement separator.
        if !letter.is_ascii_alphabetic() {
            lines.push(format!("{}: not a drive letter, skipped", volume.drive));
            continue;
        }

        let flag = if volume.operation.eq_ignore_ascii_case("Retrim") {
            "-ReTrim"
        } else {
            "-Defrag"
        };
        let script = format!("Optimize-Volume -DriveLetter {letter} {flag} -Verbose");

        match odysync_core::proc::powershell(&script, Duration::from_secs(4 * 60 * 60)).await {
            Ok(out) if out.success() => {
                lines.push(format!("{} {}: done", volume.drive, volume.operation))
            }
            Ok(out) => lines.push(format!(
                "{} {}: exited with code {}",
                volume.drive, volume.operation, out.code
            )),
            Err(e) => lines.push(format!("{} {}: {e}", volume.drive, volume.operation)),
        }
    }

    lines
}

#[cfg(not(windows))]
pub async fn run_disk_optimization(_plan: &[VolumeOptimization]) -> Vec<String> {
    Vec::new()
}

/// Reuse the security module's tolerant PowerShell JSON parsing, which handles
/// a single object being emitted where an array was expected.
#[cfg(windows)]
fn odysync_backends_parse_json<T: serde::de::DeserializeOwned>(stdout: &str) -> Vec<T> {
    crate::security::parse_ps_json(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_variable_yields_no_path_rather_than_an_empty_one() {
        // The failure this prevents: joining onto an empty path produces a
        // root-relative one, so `%LOCALAPPDATA%\Temp` becomes `\Temp`.
        assert_eq!(env_path("ODYSYNC_DEFINITELY_UNSET_VARIABLE"), None);
    }

    #[test]
    fn every_target_root_passes_the_safety_rule() {
        // `targets()` filters, so this asserts the filter is actually applied
        // rather than that the table happens to be safe.
        for target in targets() {
            assert!(
                is_safe_cleanup_root(&target.root),
                "unsafe root survived: {}",
                target.root.display()
            );
        }
    }

    #[test]
    fn no_target_root_is_duplicated() {
        // %TEMP% and %TMP% are normally the same directory; counting it twice
        // would double the reported reclaimable space.
        let targets = targets();
        for (i, a) in targets.iter().enumerate() {
            for b in targets.iter().skip(i + 1) {
                assert!(
                    !(a.category == b.category && a.root == b.root),
                    "duplicate target: {} {}",
                    a.category.id(),
                    a.root.display()
                );
            }
        }
    }

    #[test]
    fn the_prefix_strategy_matches_only_the_named_files() {
        let strategy = Strategy::Prefixed(&["thumbcache_", "iconcache_"]);
        // Built with `join` so the file name is the last component on every
        // platform; a `C:\x\...` literal is a single file name on Unix.
        let dir = Path::new("x");
        assert!(matches_strategy(strategy, &dir.join("thumbcache_256.db")));
        assert!(matches_strategy(strategy, &dir.join("iconcache_32.db")));
        // The Explorer directory holds more than caches, and clearing all of
        // it would take state the user did not offer up.
        assert!(!matches_strategy(strategy, &dir.join("Settings.dat")));
        assert!(!matches_strategy(strategy, &dir.join("NotAThumbcache.db")));
    }

    #[test]
    fn the_contents_strategy_matches_everything() {
        assert!(matches_strategy(
            Strategy::Contents,
            Path::new(r"C:\x\anything.at.all")
        ));
    }

    #[test]
    fn measuring_a_missing_directory_reports_zero_rather_than_failing() {
        let (bytes, files) = measure(
            Path::new(r"C:\odysync-definitely-not-a-directory-xyz"),
            Strategy::Contents,
        );
        assert_eq!((bytes, files), (0, 0));
    }

    #[test]
    fn browser_cache_dirs_never_include_a_credential_store() {
        // The single most important property of the browser cleaner: it must
        // never be pointed at cookies or saved passwords.
        for (_, _, dir) in browser_cache_dirs() {
            let lowered = dir.to_string_lossy().to_ascii_lowercase();
            for forbidden in ["cookies", "login data", "network", "logins.json", "key4.db"] {
                assert!(
                    !lowered.ends_with(forbidden),
                    "browser cleaner would target {}",
                    dir.display()
                );
            }
        }
    }

    #[test]
    fn every_declared_browser_cache_dir_is_a_cache() {
        for browser in browsers() {
            for dir in browser.cache_dirs {
                let lowered = dir.to_ascii_lowercase();
                assert!(
                    lowered.contains("cache"),
                    "{} declares a non-cache directory: {dir}",
                    browser.name
                );
            }
        }
    }
}
