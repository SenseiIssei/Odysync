//! Driver-store backup and restore.
//!
//! Updating a driver is the one routine operation that can leave a machine
//! unable to display anything, reach the network, or boot. `ROADMAP.md` is
//! honest that Odysync offers "prevention, not undo" — but for drivers
//! specifically Windows hands us a real undo for free, in `pnputil`, and not
//! using it was a choice worth reversing. This is the piece that makes the
//! hardware half of the tool defensible.
//!
//! ## Why nothing here parses a label
//!
//! `pnputil` output is fully localised. On a German machine `Published Name`
//! reads `Veröffentlichter Name`, and the console code page mangles the umlaut
//! on the way out. Keying on labels would reproduce exactly the defect the
//! winget table parser was rewritten to avoid.
//!
//! So [`parse_enum_drivers`] reads *values*, not labels: an `oem<n>.inf` token
//! is a published name in every locale, any other `*.inf` token in the same
//! record is the original name, and neither contains a character the code page
//! can damage. Export and restore go through exit codes alone.
//!
//! ## What restore can and cannot do
//!
//! Restore re-adds the saved packages to the driver store and asks Windows to
//! install them on matching devices. It is not a forced downgrade: Windows
//! ranks driver packages, and if the newer package is still in the store it may
//! keep winning. Removing the newer package is a separate, destructive step and
//! is deliberately not automatic — see [`restore`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use odysync_core::proc;

/// Enumerating the store is a local registry walk; exporting copies files.
const ENUM_TIMEOUT: Duration = Duration::from_secs(120);
/// A full driver-store export is hundreds of megabytes on a laptop with vendor
/// packages installed, so this is generous.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Restoring re-adds packages one at a time; each may prompt Windows to
/// reinstall a device.
const RESTORE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Name of the manifest written into each backup directory.
const MANIFEST_FILENAME: &str = "odysync-driver-backup.json";

/// `ERROR_NO_MORE_ITEMS`, which `pnputil /add-driver /install` returns when the
/// package was added but **no device needed it** — every matching device
/// already had a driver at least as good.
///
/// This is the normal outcome of restoring a package that is still current,
/// and it is also what happens in the case the module documents at length:
/// Windows ranks driver packages, so if the newer one is still in the store it
/// keeps winning and nothing is installed. Reporting either as a failure would
/// make a working restore look broken, and would train users to ignore the
/// error line that matters.
///
/// Found by running a real restore elevated: the first version of this code
/// reported "Re-added 0 package(s); 1 failed" for an operation that had done
/// exactly what it should.
const ERROR_NO_MORE_ITEMS: i32 = 259;

/// One third-party driver package in the Windows driver store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverPackage {
    /// The store's own name for it, e.g. `oem23.inf`. This is what
    /// `pnputil /export-driver` and `/delete-driver` accept.
    pub published_name: String,
    /// The vendor's name for it, e.g. `amdacpbus.inf`. Only for display.
    pub original_name: String,
}

/// One package as it was saved into a backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackedUpPackage {
    pub published_name: String,
    pub original_name: String,
    /// Directory name inside the backup, always the published name.
    pub directory: String,
    pub file_count: usize,
    pub size_bytes: u64,
}

/// The record written alongside a backup's files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Directory name and handle for `--to`, e.g. `2026-07-27T14-05-33Z`.
    pub id: String,
    pub created_at: String,
    /// Why the backup was taken, e.g. `"before applying 3 driver updates"`.
    pub reason: String,
    pub packages: Vec<BackedUpPackage>,
    /// Packages that could not be exported, with the reason.
    ///
    /// A partial backup is still worth keeping — one unexportable package
    /// should not discard the other eighty — but it must not be mistaken for a
    /// complete one.
    pub failures: Vec<String>,
}

impl BackupManifest {
    /// Whether every package in the store made it into the backup.
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }

    pub fn total_size_bytes(&self) -> u64 {
        self.packages.iter().map(|p| p.size_bytes).sum()
    }
}

/// What a restore actually managed to do.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RestoreReport {
    /// Packages that were re-added and installed onto a device.
    pub restored: Vec<String>,
    /// Packages that were re-added, but which no device needed.
    ///
    /// Separate from both success and failure because it is genuinely a third
    /// outcome: the package is back in the driver store, and Windows decided
    /// the driver already on the device was at least as good. Folding it into
    /// `failed` misreports a working restore; folding it into `restored`
    /// claims a device changed when none did.
    pub already_current: Vec<String>,
    pub failed: Vec<String>,
    /// Windows asked for a restart to finish installing at least one package.
    pub reboot_required: bool,
}

impl RestoreReport {
    /// Whether every package was dealt with, one way or the other.
    pub fn is_success(&self) -> bool {
        self.failed.is_empty()
    }
}

// ── Parsing ─────────────────────────────────────────────────────────────────

/// Extract the driver packages from `pnputil /enum-drivers` output.
///
/// Records are separated by blank lines. Within a record the published name is
/// the `oem<n>.inf` token and the original name is the other `.inf` token —
/// both are values rather than labels, so this is unaffected by the display
/// language and by the console code page mangling non-ASCII characters.
pub fn parse_enum_drivers(stdout: &str) -> Vec<DriverPackage> {
    let mut out = Vec::new();

    for block in blocks(stdout) {
        let mut published: Option<String> = None;
        let mut original: Option<String> = None;

        for token in inf_tokens(&block) {
            if is_published_name(&token) {
                published.get_or_insert(token);
            } else {
                original.get_or_insert(token);
            }
        }

        if let Some(published_name) = published {
            let original_name = original.unwrap_or_else(|| published_name.clone());
            // The same package cannot legitimately appear twice, and a
            // duplicate would be exported twice into the same directory.
            if !out
                .iter()
                .any(|p: &DriverPackage| p.published_name == published_name)
            {
                out.push(DriverPackage {
                    published_name,
                    original_name,
                });
            }
        }
    }

    out
}

/// Split `pnputil` output into one block per driver package.
///
/// Blocks are separated by a blank line — but `pnputil` is a Windows console
/// program, so its blank line is `\r\n\r\n`. Splitting the raw text on `"\n\n"`
/// finds no match at all, collapsing the entire output into a single block and
/// yielding exactly **one** package. That is not a hypothetical: it read 1
/// package on a machine with 215, and the unit tests missed it because they
/// were written with `\n` line endings and so only ever tested their own
/// assumption. Accumulating lines and breaking on a blank one sidesteps the
/// line-ending question rather than trying to enumerate its forms.
fn blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    if !current.is_empty() {
        out.push(current);
    }

    out
}

/// Every `*.inf` token in `text`, lowercased, in order of appearance.
fn inf_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace())
        .map(|w| w.trim_matches(|c: char| c == '(' || c == ')' || c == ',' || c == ':'))
        .filter(|w| w.len() > 4 && w.to_ascii_lowercase().ends_with(".inf"))
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

/// Is this the driver store's own name for a package — `oem23.inf`?
fn is_published_name(name: &str) -> bool {
    let Some(digits) = name
        .strip_prefix("oem")
        .and_then(|rest| rest.strip_suffix(".inf"))
    else {
        return false;
    };
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

// ── Locations ───────────────────────────────────────────────────────────────

/// Root directory holding every backup, one subdirectory per backup.
pub fn backup_root() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
        .ok_or_else(|| anyhow::anyhow!("could not resolve the user data directory"))?;
    let dir = dirs.data_local_dir().join("driver-backup");
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Reject a backup id that is not a single plain directory name.
///
/// Ids reach us from the CLI and from the GUI, and they are joined onto the
/// backup root. `..\..\Windows\System32` must not turn a restore into an
/// arbitrary-directory read or a prune into an arbitrary-directory delete.
fn safe_backup_path(root: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() {
        anyhow::bail!("backup id is empty");
    }
    if id.contains('/') || id.contains('\\') || id.contains(':') {
        anyhow::bail!("backup id {id:?} contains a path separator");
    }
    let mut components = Path::new(id).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None) => {}
        _ => anyhow::bail!("backup id {id:?} is not a plain directory name"),
    }
    Ok(root.join(id))
}

/// A timestamp usable as a directory name on Windows, where `:` is illegal.
fn backup_id_for(now: chrono::DateTime<chrono::Utc>) -> String {
    now.format("%Y-%m-%dT%H-%M-%SZ").to_string()
}

// ── Enumerate ───────────────────────────────────────────────────────────────

/// Every third-party driver package currently in the store.
///
/// Works unelevated: enumerating and exporting the driver store does not need
/// administrator rights, only installing does.
pub async fn list_packages() -> Result<Vec<DriverPackage>> {
    if !cfg!(windows) {
        return Ok(Vec::new());
    }
    let out = proc::run("pnputil", &["/enum-drivers"], ENUM_TIMEOUT).await?;
    Ok(parse_enum_drivers(&out.stdout))
}

// ── Backup ──────────────────────────────────────────────────────────────────

/// Export every third-party driver package into a fresh backup directory.
///
/// Each package gets its own subdirectory named after its published name.
/// `pnputil /export-driver` writes a single package's files *flat* into the
/// target directory, so exporting several into one directory would let two
/// packages overwrite each other's files.
pub async fn create_backup(reason: &str) -> Result<BackupManifest> {
    let packages = list_packages().await?;
    create_backup_of(reason, &packages).await
}

/// [`create_backup`] over a caller-supplied package list.
pub async fn create_backup_of(reason: &str, packages: &[DriverPackage]) -> Result<BackupManifest> {
    let root = backup_root()?;
    let id = backup_id_for(chrono::Utc::now());
    let dir = safe_backup_path(&root, &id)?;
    std::fs::create_dir_all(&dir)?;

    let mut saved = Vec::new();
    let mut failures = Vec::new();

    for package in packages {
        // The published name is `oem<n>.inf` and nothing else — `is_published_name`
        // enforced that — so it cannot escape the backup directory.
        let target = dir.join(&package.published_name);
        if let Err(e) = std::fs::create_dir_all(&target) {
            failures.push(format!("{}: {e}", package.published_name));
            continue;
        }

        let result = proc::run(
            "pnputil",
            &[
                "/export-driver",
                &package.published_name,
                &target.display().to_string(),
            ],
            EXPORT_TIMEOUT,
        )
        .await;

        match result {
            Ok(out) if out.success() => {
                let (file_count, size_bytes) = measure_dir(&target);
                // An export that reports success but wrote nothing is a failed
                // export wearing a zero exit code. Counting the files is the
                // only way to tell, and a backup entry that restores nothing is
                // worse than an honest failure.
                if file_count == 0 {
                    failures.push(format!("{} exported no files", package.published_name));
                    let _ = std::fs::remove_dir_all(&target);
                    continue;
                }
                saved.push(BackedUpPackage {
                    published_name: package.published_name.clone(),
                    original_name: package.original_name.clone(),
                    directory: package.published_name.clone(),
                    file_count,
                    size_bytes,
                });
            }
            Ok(out) => {
                failures.push(format!(
                    "{} failed with exit code {}",
                    package.published_name, out.code
                ));
                let _ = std::fs::remove_dir_all(&target);
            }
            Err(e) => {
                failures.push(format!("{}: {e}", package.published_name));
                let _ = std::fs::remove_dir_all(&target);
            }
        }
    }

    let manifest = BackupManifest {
        id: id.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        reason: reason.to_string(),
        packages: saved,
        failures,
    };

    std::fs::write(
        dir.join(MANIFEST_FILENAME),
        serde_json::to_string_pretty(&manifest)?,
    )?;

    tracing::info!(
        id = %manifest.id,
        packages = manifest.packages.len(),
        failed = manifest.failures.len(),
        size = manifest.total_size_bytes(),
        "driver backup written"
    );

    Ok(manifest)
}

/// File count and total size under `dir`, recursively. Never fails.
fn measure_dir(dir: &Path) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };

    let mut count = 0;
    let mut size = 0;
    for entry in entries.flatten() {
        match entry.metadata() {
            Ok(m) if m.is_file() => {
                count += 1;
                size += m.len();
            }
            Ok(m) if m.is_dir() => {
                let (c, s) = measure_dir(&entry.path());
                count += c;
                size += s;
            }
            _ => {}
        }
    }
    (count, size)
}

// ── List, restore, prune ────────────────────────────────────────────────────

/// Every backup on disk, newest first.
///
/// A directory without a readable manifest is skipped rather than erroring:
/// one corrupt backup must not hide the others, which are the ones the user
/// needs when something has already gone wrong.
pub fn list_backups() -> Result<Vec<BackupManifest>> {
    let root = backup_root()?;
    let mut out = Vec::new();

    for entry in std::fs::read_dir(&root)?.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let manifest_path = entry.path().join(MANIFEST_FILENAME);
        match std::fs::read_to_string(&manifest_path) {
            Ok(text) => match serde_json::from_str::<BackupManifest>(&text) {
                Ok(m) => out.push(m),
                Err(e) => tracing::warn!(path = %manifest_path.display(), error = %e,
                    "skipping a driver backup with an unreadable manifest"),
            },
            Err(e) => tracing::warn!(path = %manifest_path.display(), error = %e,
                "skipping a driver backup with no manifest"),
        }
    }

    // Ids are `%Y-%m-%dT%H-%M-%SZ`, which sorts lexicographically by time.
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}

/// Re-add the packages from backup `id` and install them on matching devices.
///
/// **This is not a forced downgrade.** `pnputil /add-driver /install` puts the
/// saved package back into the store and asks Windows to install it where it
/// applies, but Windows ranks driver packages, and a newer package still in the
/// store can keep winning. When that happens the newer package has to be
/// deleted, which is destructive and is left to the user through Device
/// Manager rather than done automatically here — deleting the wrong driver
/// package is exactly the failure this module exists to protect against.
///
/// Requires elevation: adding to the driver store is an administrator
/// operation, unlike exporting from it.
pub async fn restore(id: &str) -> Result<RestoreReport> {
    let root = backup_root()?;
    let dir = safe_backup_path(&root, id)?;

    let text = std::fs::read_to_string(dir.join(MANIFEST_FILENAME))
        .map_err(|e| anyhow::anyhow!("no driver backup {id:?}: {e}"))?;
    let manifest: BackupManifest = serde_json::from_str(&text)?;

    if !odysync_core::platform::is_elevated() {
        anyhow::bail!(
            "restoring drivers writes to the driver store, which needs administrator rights. \
             Exporting a backup does not — only putting one back."
        );
    }

    let mut report = RestoreReport::default();

    for package in &manifest.packages {
        // `directory` came out of a JSON file on disk, so it is untrusted for
        // the same reason the offline cache's filenames are.
        let package_dir = match safe_backup_path(&dir, &package.directory) {
            Ok(p) => p,
            Err(e) => {
                report
                    .failed
                    .push(format!("{}: {e}", package.published_name));
                continue;
            }
        };

        let inf_glob = package_dir.join("*.inf");
        let result = proc::run(
            "pnputil",
            &[
                "/add-driver",
                &inf_glob.display().to_string(),
                "/subdirs",
                "/install",
            ],
            RESTORE_TIMEOUT,
        )
        .await;

        match result {
            Ok(out) if out.success() => report.restored.push(package.published_name.clone()),
            // Added to the store, but no device wanted it. A third outcome,
            // not a failure — see ERROR_NO_MORE_ITEMS.
            Ok(out) if out.code == ERROR_NO_MORE_ITEMS => {
                report.already_current.push(package.published_name.clone())
            }
            Ok(out) => report.failed.push(format!(
                "{} failed with exit code {}",
                package.published_name, out.code
            )),
            Err(e) => report
                .failed
                .push(format!("{}: {e}", package.published_name)),
        }
    }

    // Ask the OS rather than parsing pnputil's localised "a restart is
    // required" line.
    report.reboot_required = odysync_core::platform::reboot_pending();

    tracing::info!(
        id,
        restored = report.restored.len(),
        already_current = report.already_current.len(),
        failed = report.failed.len(),
        "driver restore finished"
    );

    Ok(report)
}

/// Delete all but the `keep` newest backups. Returns how many were removed.
///
/// A full driver-store export runs to hundreds of megabytes, so keeping every
/// one would quietly fill the disk of the machine we are meant to be looking
/// after.
pub fn prune(keep: usize) -> Result<usize> {
    let root = backup_root()?;
    let backups = list_backups()?;

    let mut removed = 0;
    for manifest in backups.into_iter().skip(keep) {
        let dir = safe_backup_path(&root, &manifest.id)?;
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!(id = %manifest.id, error = %e,
                "could not remove an old driver backup"),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `pnputil /enum-drivers` output from a German Windows 11 machine,
    /// including the code-page damage the console does to the umlauts. The
    /// point of the test is that none of that matters.
    const GERMAN_OUTPUT: &str = "Microsoft-PnP-Hilfsprogramm\n\n\
         Ver\u{fffd}ffentlichter Name:     oem23.inf\n\
         Originalname:      amdacpbus.inf\n\
         Anbietername:      AMD\n\
         Klassenname:         System\n\
         Klassen-GUID:         {4d36e97d-e325-11ce-bfc1-08002be10318}\n\
         Treiberversion:     05/09/2023 6.0.0.71\n\n\
         Ver\u{fffd}ffentlichter Name:     oem51.inf\n\
         Originalname:      amdacpbusext.inf\n\
         Anbietername:      AMD\n\
         Klassenname:         Extension\n\n\
         Ver\u{fffd}ffentlichter Name:     oem11.inf\n\
         Originalname:      amdafd.inf\n\
         Anbietername:      AMD\n";

    const ENGLISH_OUTPUT: &str = "Microsoft PnP Utility\n\n\
         Published Name:     oem23.inf\n\
         Original Name:      amdacpbus.inf\n\
         Provider Name:      AMD\n\
         Class Name:         System\n\n\
         Published Name:     oem51.inf\n\
         Original Name:      amdacpbusext.inf\n\
         Provider Name:      AMD\n";

    /// `pnputil` is a Windows console program: its blank line is `\r\n\r\n`.
    /// Every parser test runs against both line endings, because the version
    /// of this parser that split on `"\n\n"` passed the LF-only tests and then
    /// read 1 package on a machine with 215.
    fn crlf(text: &str) -> String {
        text.replace('\n', "\r\n")
    }

    #[test]
    fn crlf_output_parses_the_same_as_lf() {
        let lf = parse_enum_drivers(ENGLISH_OUTPUT);
        let crlf_parsed = parse_enum_drivers(&crlf(ENGLISH_OUTPUT));
        assert_eq!(lf.len(), 2);
        assert_eq!(crlf_parsed, lf, "CRLF output parsed differently");
    }

    #[test]
    fn every_record_is_found_not_just_the_first() {
        // The collapse-to-one-block failure mode yields exactly one package
        // regardless of how many are present, so a count is the assertion that
        // catches it.
        for text in [GERMAN_OUTPUT.to_string(), crlf(GERMAN_OUTPUT)] {
            assert_eq!(parse_enum_drivers(&text).len(), 3, "records were merged");
        }
    }

    #[test]
    fn parses_localised_output_identically_to_english() {
        let de = parse_enum_drivers(GERMAN_OUTPUT);
        let en = parse_enum_drivers(ENGLISH_OUTPUT);

        assert_eq!(de.len(), 3);
        assert_eq!(en.len(), 2);
        // The first two records are the same packages in both languages.
        assert_eq!(de[0], en[0]);
        assert_eq!(de[1], en[1]);
    }

    #[test]
    fn reads_the_published_and_original_names() {
        let packages = parse_enum_drivers(ENGLISH_OUTPUT);
        assert_eq!(packages[0].published_name, "oem23.inf");
        assert_eq!(packages[0].original_name, "amdacpbus.inf");
    }

    #[test]
    fn the_banner_line_does_not_become_a_package() {
        // "Microsoft PnP Utility" is its own block before the first record and
        // contains no .inf token, so it must produce nothing.
        assert!(parse_enum_drivers("Microsoft PnP Utility\n\n").is_empty());
        assert!(parse_enum_drivers("").is_empty());
    }

    #[test]
    fn a_record_without_a_published_name_is_skipped() {
        // Not every block pnputil emits is a driver package; one without an
        // oem<n>.inf token cannot be exported and must not be listed.
        let text = "Published Name:\nOriginal Name:      loose.inf\n";
        assert!(parse_enum_drivers(text).is_empty());
    }

    #[test]
    fn duplicate_packages_are_collapsed() {
        for text in [
            format!("{ENGLISH_OUTPUT}\n{ENGLISH_OUTPUT}"),
            crlf(&format!("{ENGLISH_OUTPUT}\n{ENGLISH_OUTPUT}")),
        ] {
            let packages = parse_enum_drivers(&text);
            assert_eq!(packages.len(), 2, "a duplicate would be exported twice");
        }
    }

    #[test]
    fn only_oem_numbered_names_count_as_published() {
        assert!(is_published_name("oem23.inf"));
        assert!(is_published_name("oem0.inf"));
        assert!(!is_published_name("oem.inf"));
        assert!(!is_published_name("oemabc.inf"));
        assert!(!is_published_name("amdacpbus.inf"));
        assert!(!is_published_name("myoem23.inf"));
    }

    #[test]
    fn published_names_cannot_escape_the_backup_directory() {
        // Directory names come from `published_name`, so the shape check is
        // what keeps a hostile enum output from writing outside the backup.
        for hostile in [
            "../oem1.inf",
            "..\\oem1.inf",
            "oem1.inf/../..",
            "C:\\oem1.inf",
        ] {
            assert!(!is_published_name(hostile), "accepted {hostile:?}");
        }
    }

    #[test]
    fn a_backup_id_must_be_a_plain_directory_name() {
        let root = Path::new("C:\\backups");
        assert!(safe_backup_path(root, "2026-07-27T14-05-33Z").is_ok());
        for hostile in ["..", "../etc", "..\\Windows", "C:\\Windows", "a/b", ""] {
            assert!(
                safe_backup_path(root, hostile).is_err(),
                "accepted {hostile:?}"
            );
        }
    }

    #[test]
    fn the_backup_id_is_a_legal_windows_directory_name() {
        let ts = chrono::DateTime::parse_from_rfc3339("2026-07-27T14:05:33Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let id = backup_id_for(ts);
        assert_eq!(id, "2026-07-27T14-05-33Z");
        // `:` is illegal in a Windows path component, which is the whole
        // reason this is not just an RFC 3339 string.
        assert!(!id.contains(':'));
    }

    #[test]
    fn backup_ids_sort_newest_first_lexicographically() {
        let mut ids = [
            "2026-07-27T14-05-33Z",
            "2026-07-27T09-00-00Z",
            "2025-12-31T23-59-59Z",
        ];
        ids.sort_by(|a, b| b.cmp(a));
        assert_eq!(ids[0], "2026-07-27T14-05-33Z");
        assert_eq!(ids[2], "2025-12-31T23-59-59Z");
    }

    /// Found by a real elevated restore, which reported
    /// "Re-added 0 package(s); 1 failed" for an operation that had done
    /// exactly what it should: the package went back into the store and no
    /// device needed it, because the driver already installed was current.
    #[test]
    fn a_package_no_device_needed_is_not_a_failure() {
        let mut report = RestoreReport::default();
        report.already_current.push("oem23.inf".into());

        assert!(report.is_success(), "259 must not read as a failed restore");
        assert!(report.failed.is_empty());
        // Nor may it be claimed as a device that changed.
        assert!(report.restored.is_empty());
    }

    #[test]
    fn a_genuine_failure_still_fails() {
        let mut report = RestoreReport::default();
        report
            .failed
            .push("oem9.inf failed with exit code 5".into());
        assert!(!report.is_success());
    }

    #[test]
    fn the_no_more_items_code_is_the_documented_one() {
        // ERROR_NO_MORE_ITEMS. Named rather than inlined so the next person
        // does not have to look up what 259 meant.
        assert_eq!(ERROR_NO_MORE_ITEMS, 259);
    }

    #[test]
    fn a_manifest_with_failures_is_not_complete() {
        let mut m = BackupManifest {
            id: "x".into(),
            created_at: "x".into(),
            reason: "test".into(),
            packages: Vec::new(),
            failures: Vec::new(),
        };
        assert!(m.is_complete());
        m.failures.push("oem1.inf: denied".into());
        assert!(!m.is_complete());
    }

    #[test]
    fn a_manifest_round_trips_through_json() {
        let m = BackupManifest {
            id: "2026-07-27T14-05-33Z".into(),
            created_at: "2026-07-27T14:05:33Z".into(),
            reason: "before applying 3 driver updates".into(),
            packages: vec![BackedUpPackage {
                published_name: "oem23.inf".into(),
                original_name: "amdacpbus.inf".into(),
                directory: "oem23.inf".into(),
                file_count: 10,
                size_bytes: 4096,
            }],
            failures: vec!["oem9.inf: denied".into()],
        };
        let text = serde_json::to_string(&m).unwrap();
        let back: BackupManifest = serde_json::from_str(&text).unwrap();
        assert_eq!(back.id, m.id);
        assert_eq!(back.packages.len(), 1);
        assert_eq!(back.total_size_bytes(), 4096);
        assert!(!back.is_complete());
    }
}
