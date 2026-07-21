//! Applying fixes — the part that is allowed to change the machine.
//!
//! The guiding assumption is that this code will one day be handed a
//! [`Remediation`] built from attacker-controlled data: a registry value name, a
//! service name, a file path that came out of a scan of the very machine that
//! may be compromised. So nothing here trusts its input.
//!
//!   * **Deletion is never deletion.** Files are moved into a quarantine
//!     directory and renamed `.quarantined`, so a false positive is an
//!     inconvenience rather than data loss. Anything outside the user-writable
//!     directories — anywhere under `C:\Windows` or `Program Files` — is
//!     refused outright with [`Error::SecurityViolation`], regardless of what
//!     the finding claimed.
//!   * **Registry changes are backed up first.** The original value is written
//!     to a JSON restore file before it is removed, so "disable" is reversible.
//!   * **Elevation is checked up front.** An action that needs administrator
//!     rights fails with a sentence telling the user what to do, rather than
//!     with whatever PowerShell says when access is denied.
//!   * **Allowlists, not denylists**, wherever the set of legal values is
//!     known: only the standard `Run` keys can be edited, and a short list of
//!     services critical to Windows can never be stopped.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use odysync_core::error::{Error, Result};

use super::Remediation;

/// Registry keys whose values may be removed. Anything else is refused: this
/// function's whole job is to edit autostart entries, and a caller asking it to
/// touch some other key is either confused or hostile.
const ALLOWED_RUN_KEYS: &[&str] = &[
    r"hkey_current_user\software\microsoft\windows\currentversion\run",
    r"hkey_current_user\software\microsoft\windows\currentversion\runonce",
    r"hkey_current_user\software\wow6432node\microsoft\windows\currentversion\run",
    r"hkey_current_user\software\wow6432node\microsoft\windows\currentversion\runonce",
    r"hkey_current_user\software\microsoft\windows\currentversion\policies\explorer\run",
    r"hkey_local_machine\software\microsoft\windows\currentversion\run",
    r"hkey_local_machine\software\microsoft\windows\currentversion\runonce",
    r"hkey_local_machine\software\wow6432node\microsoft\windows\currentversion\run",
    r"hkey_local_machine\software\wow6432node\microsoft\windows\currentversion\runonce",
    r"hkey_local_machine\software\microsoft\windows\currentversion\policies\explorer\run",
];

/// Services that must never be stopped, whatever a finding says. Disabling any
/// of these leaves a machine that will not boot, will not network, or cannot be
/// managed — which is a far worse outcome than the malware.
const PROTECTED_SERVICES: &[&str] = &[
    "windefend",
    "wscsvc",
    "securityhealthservice",
    "sense",
    "mpssvc",
    "bfe",
    "eventlog",
    "rpcss",
    "dcomlaunch",
    "plugplay",
    "power",
    "profsvc",
    "schedule",
    "lsm",
    "dhcp",
    "dnscache",
    "nsi",
    "lanmanworkstation",
    "lanmanserver",
    "winmgmt",
    "cryptsvc",
    "trustedinstaller",
    "wuauserv",
    "samss",
    "netlogon",
    "termservice",
    "audiosrv",
];

/// The stock contents of `hosts`, written when the file is reset.
pub const DEFAULT_HOSTS: &str = "# Copyright (c) 1993-2009 Microsoft Corp.\r\n\
#\r\n\
# This is a sample HOSTS file used by Microsoft TCP/IP for Windows.\r\n\
#\r\n\
# This file contains the mappings of IP addresses to host names. Each\r\n\
# entry should be kept on an individual line. The IP address should\r\n\
# be placed in the first column followed by the corresponding host name.\r\n\
# The IP address and the host name should be separated by at least one\r\n\
# space.\r\n\
#\r\n\
# localhost name resolution is handled within DNS itself.\r\n\
#\t127.0.0.1       localhost\r\n\
#\t::1             localhost\r\n";

/// One saved registry value, so a disabled autostart entry can be put back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunKeyBackup {
    pub hive: String,
    pub name: String,
    pub value: String,
    pub removed_at: String,
}

// ---------------------------------------------------------------------------
// Validation (pure)
// ---------------------------------------------------------------------------

/// Reject values that could break out of a PowerShell string literal's line, or
/// that are obviously not what they claim to be.
fn reject_control_chars(context: &'static str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::security(context, "value is empty"));
    }
    if value.contains(['\0', '\r', '\n']) {
        return Err(Error::security(
            context,
            "value contains a control character",
        ));
    }
    Ok(())
}

/// A registry key may only be one of the standard autostart locations.
pub fn validate_run_key_hive(hive: &str) -> Result<String> {
    reject_control_chars("registry key validation", hive)?;
    let normalized = hive
        .trim()
        .trim_start_matches("Registry::")
        .replace("HKCU:", "HKEY_CURRENT_USER")
        .replace("HKLM:", "HKEY_LOCAL_MACHINE")
        .replace('/', "\\")
        .trim_matches('\\')
        .to_ascii_lowercase();

    if !ALLOWED_RUN_KEYS.contains(&normalized.as_str()) {
        return Err(Error::security(
            "registry key validation",
            format!(
                "{hive} is not one of the standard autostart keys; this tool only \
                 edits Run and RunOnce"
            ),
        ));
    }
    Ok(normalized)
}

/// True when the key lives in `HKEY_LOCAL_MACHINE` and therefore needs
/// administrator rights.
pub fn run_key_needs_elevation(normalized_hive: &str) -> bool {
    normalized_hive.starts_with("hkey_local_machine")
}

/// A scheduled task must be a full path, and must not be one of Windows' own.
///
/// Disabling something under `\Microsoft\Windows\` breaks Windows Update,
/// defragmentation, or Defender's own scheduled scans — an outcome an attacker
/// would be delighted with, so it is refused even though such a task can
/// legitimately be hijacked. Those are reported for manual handling instead.
pub fn validate_task_path(task_path: &str) -> Result<(String, String)> {
    reject_control_chars("scheduled task validation", task_path)?;
    let full = task_path.trim();
    if !full.starts_with('\\') {
        return Err(Error::security(
            "scheduled task validation",
            format!("{full} is not a full task path (it must start with \\)"),
        ));
    }
    if full
        .to_ascii_lowercase()
        .starts_with("\\microsoft\\windows\\")
    {
        return Err(Error::security(
            "scheduled task validation",
            format!(
                "{full} is one of Windows' own scheduled tasks; disabling it \
                 automatically could break the system, so it must be reviewed by hand"
            ),
        ));
    }
    let idx = full.rfind('\\').expect("checked to start with a backslash");
    let (dir, name) = full.split_at(idx + 1);
    if name.is_empty() {
        return Err(Error::security(
            "scheduled task validation",
            format!("{full} names a folder, not a task"),
        ));
    }
    Ok((dir.to_string(), name.to_string()))
}

/// A service name must be a plain identifier and must not be critical to
/// Windows.
pub fn validate_service_name(name: &str) -> Result<()> {
    reject_control_chars("service validation", name)?;
    let trimmed = name.trim();
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ' '))
    {
        return Err(Error::security(
            "service validation",
            format!("{trimmed} is not a valid service name"),
        ));
    }
    if PROTECTED_SERVICES.contains(&trimmed.to_ascii_lowercase().as_str()) {
        return Err(Error::security(
            "service validation",
            format!(
                "{trimmed} is a service Windows needs to function; it will not be \
                 stopped automatically. If you believe it has been hijacked, \
                 investigate its binary path rather than disabling the service"
            ),
        ));
    }
    Ok(())
}

/// Decide whether a file may be quarantined.
///
/// `roots` are the user-writable directories, passed in rather than read from
/// the environment so the rule can be tested. The check is deliberately
/// two-sided: the path must be inside a known-safe root *and* must not be
/// inside a system directory, so a root that itself sits under `C:\Windows`
/// could not be used to smuggle a deletion through.
pub fn check_quarantinable(path: &str, roots: &[String]) -> Result<()> {
    reject_control_chars("file quarantine", path)?;

    let normalized = super::normalize_path(path);
    if normalized.contains("..") {
        return Err(Error::security(
            "file quarantine",
            format!("{path} contains a relative path segment"),
        ));
    }
    if super::is_system_dir(&normalized) {
        return Err(Error::security(
            "file quarantine",
            format!(
                "{path} is inside a protected system directory; this tool will never \
                 remove files from C:\\Windows or Program Files. If a system file is \
                 genuinely infected, let Windows Defender handle it"
            ),
        ));
    }
    let inside = roots.iter().any(|r| {
        let r = super::normalize_path(r);
        !r.is_empty() && (normalized == r || normalized.starts_with(&format!("{r}\\")))
    });
    if !inside {
        return Err(Error::security(
            "file quarantine",
            format!(
                "{path} is outside the directories this tool is allowed to touch \
                 (%TEMP%, %APPDATA%, %LOCALAPPDATA% and the Startup folders)"
            ),
        ));
    }
    Ok(())
}

/// The name a quarantined file is stored under.
pub fn quarantine_name(original: &str, stamp: &str) -> String {
    let file = original.rsplit(['\\', '/']).next().unwrap_or(original);
    format!("{stamp}_{file}.quarantined")
}

// ---------------------------------------------------------------------------
// Application directories
// ---------------------------------------------------------------------------

fn app_dir(sub: &str) -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync")
        .ok_or_else(|| Error::Config("could not resolve the application data directory".into()))?;
    let dir = dirs.data_dir().join(sub);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Where undo information is written.
pub fn restore_dir() -> Result<PathBuf> {
    app_dir("security-restore")
}

/// Where quarantined files are moved.
pub fn quarantine_dir() -> Result<PathBuf> {
    app_dir("quarantine")
}

/// The user-writable roots that files may be quarantined from.
#[cfg(windows)]
fn quarantine_roots() -> Vec<String> {
    ["TEMP", "TMP", "APPDATA", "LOCALAPPDATA"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .filter(|v| !v.trim().is_empty())
        .chain(
            std::env::var("ProgramData")
                .ok()
                .map(|p| format!(r"{p}\Microsoft\Windows\Start Menu\Programs\StartUp")),
        )
        .collect()
}

/// Fail with an actionable message rather than an access-denied stack trace.
#[cfg(windows)]
fn require_elevation(action: &str) -> Result<()> {
    if odysync_core::platform::is_elevated() {
        return Ok(());
    }
    Err(Error::health_check_failed(
        "administrator rights",
        format!(
            "{action} requires administrator rights. Close Odysync, right-click it and \
             choose \"Run as administrator\", then apply this fix again."
        ),
    ))
}

// ---------------------------------------------------------------------------
// apply
// ---------------------------------------------------------------------------

/// Carry out a remediation, returning a description of what was done.
///
/// The returned string is written for the user, not for a log: it says what
/// changed and, where relevant, where the undo information went.
#[cfg(windows)]
pub async fn apply(remediation: &Remediation) -> Result<String> {
    match remediation {
        Remediation::Manual { instructions } => Ok(instructions.clone()),
        Remediation::RemoveDefenderThreat { threat_id } => {
            super::defender::remove_threat(threat_id).await
        }
        Remediation::DisableRunKey { hive, name } => disable_run_key(hive, name).await,
        Remediation::DisableScheduledTask { task_path } => disable_scheduled_task(task_path).await,
        Remediation::DeleteFile { path } => quarantine_file(path).await,
        Remediation::StopAndDisableService { name } => stop_and_disable_service(name).await,
        Remediation::ResetHostsFile => reset_hosts_file().await,
    }
}

/// Non-Windows stub: every remediation here manipulates Windows-specific state.
#[cfg(not(windows))]
pub async fn apply(remediation: &Remediation) -> Result<String> {
    match remediation {
        Remediation::Manual { instructions } => Ok(instructions.clone()),
        _ => Err(Error::unavailable(
            "security remediation",
            "automatic remediation is only implemented on Windows",
        )),
    }
}

/// Back up an autostart value, then remove it.
#[cfg(windows)]
async fn disable_run_key(hive: &str, name: &str) -> Result<String> {
    use std::time::Duration;

    let normalized = validate_run_key_hive(hive)?;
    reject_control_chars("registry value validation", name)?;
    if run_key_needs_elevation(&normalized) {
        require_elevation("Removing a machine-wide autostart entry")?;
    }

    let key = format!("Registry::{}", normalized.to_ascii_uppercase());
    let quoted_key = super::ps_quote(&key);
    let quoted_name = super::ps_quote(name);

    // Read first: a backup written after the deletion would be worthless.
    let read = format!(
        "$ErrorActionPreference='SilentlyContinue'
$k = Get-Item -LiteralPath {quoted_key}
if ($k) {{ [string]$k.GetValue({quoted_name}) }}"
    );
    let value = super::ps_query(&read, Duration::from_secs(60))
        .await?
        .trim()
        .to_string();

    let backup = RunKeyBackup {
        hive: normalized.clone(),
        name: name.to_string(),
        value: value.clone(),
        removed_at: chrono::Utc::now().to_rfc3339(),
    };
    let backup_path = append_run_key_backup(&backup)?;

    let remove = format!(
        "$ErrorActionPreference='Stop'
Remove-ItemProperty -LiteralPath {quoted_key} -Name {quoted_name} -Force
'ok'"
    );
    super::ps_mutate("Remove-ItemProperty", &remove, Duration::from_secs(60)).await?;

    Ok(format!(
        "Removed the autostart entry \"{name}\". Its original value was saved to {} so \
         it can be restored.",
        backup_path.display()
    ))
}

/// Append one backup record to the restore file.
#[cfg(windows)]
fn append_run_key_backup(backup: &RunKeyBackup) -> Result<PathBuf> {
    let path = restore_dir()?.join("run-keys.json");
    let mut all: Vec<RunKeyBackup> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    all.push(backup.clone());
    std::fs::write(&path, serde_json::to_string_pretty(&all)?)?;
    Ok(path)
}

/// Every autostart value this tool has removed, most recent last.
pub fn run_key_backups() -> Result<Vec<RunKeyBackup>> {
    let path = restore_dir()?.join("run-keys.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
        Err(_) => Ok(Vec::new()),
    }
}

#[cfg(windows)]
async fn disable_scheduled_task(task_path: &str) -> Result<String> {
    use std::time::Duration;

    let (dir, name) = validate_task_path(task_path)?;
    require_elevation("Disabling a scheduled task")?;

    let script = format!(
        "$ErrorActionPreference='Stop'
Disable-ScheduledTask -TaskPath {} -TaskName {} | Out-Null
'ok'",
        super::ps_quote(&dir),
        super::ps_quote(&name)
    );
    super::ps_mutate("Disable-ScheduledTask", &script, Duration::from_secs(90)).await?;

    Ok(format!(
        "Disabled the scheduled task {task_path}. It still exists — re-enable it with \
         Enable-ScheduledTask if this turns out to be wrong."
    ))
}

/// Move a file into quarantine rather than deleting it.
#[cfg(windows)]
async fn quarantine_file(path: &str) -> Result<String> {
    let roots = quarantine_roots();
    check_quarantinable(path, &roots)?;

    let source = PathBuf::from(path);
    if !source.is_file() {
        return Err(Error::parse(
            "quarantine",
            format!("{path} no longer exists; nothing to do"),
        ));
    }

    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let dest = quarantine_dir()?.join(quarantine_name(path, &stamp));

    let moved = move_file(&source, &dest)?;
    Ok(format!(
        "Moved {path} to quarantine at {}. Nothing was deleted: if this was a mistake, \
         rename the file and move it back.{}",
        dest.display(),
        if moved {
            ""
        } else {
            " (copied, then the original was removed)"
        }
    ))
}

/// Rename where possible, copy-then-remove across volumes. Returns whether the
/// cheap path was taken.
#[cfg(windows)]
fn move_file(source: &std::path::Path, dest: &std::path::Path) -> Result<bool> {
    match std::fs::rename(source, dest) {
        Ok(()) => Ok(true),
        Err(_) => {
            // Different volume, or the file is locked by a running process.
            std::fs::copy(source, dest)?;
            std::fs::remove_file(source)?;
            Ok(false)
        }
    }
}

#[cfg(windows)]
async fn stop_and_disable_service(name: &str) -> Result<String> {
    use std::time::Duration;

    validate_service_name(name)?;
    require_elevation("Stopping and disabling a service")?;

    let quoted = super::ps_quote(name.trim());
    let script = format!(
        "$ErrorActionPreference='Stop'
Stop-Service -Name {quoted} -Force -ErrorAction SilentlyContinue
Set-Service -Name {quoted} -StartupType Disabled
'ok'"
    );
    super::ps_mutate("Set-Service", &script, Duration::from_secs(120)).await?;

    Ok(format!(
        "Stopped the {name} service and set it to Disabled. The service and its files \
         are still present; re-enable it with Set-Service -StartupType Automatic if \
         needed."
    ))
}

#[cfg(windows)]
async fn reset_hosts_file() -> Result<String> {
    require_elevation("Resetting the hosts file")?;

    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let hosts = root.join(r"System32\drivers\etc\hosts");

    let backup = restore_dir()?.join(format!(
        "hosts-{}.bak",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    ));
    if hosts.is_file() {
        std::fs::copy(&hosts, &backup)?;
    }
    std::fs::write(&hosts, DEFAULT_HOSTS)?;

    Ok(format!(
        "Reset the hosts file to the Windows default. The previous contents were saved \
         to {}. Any ad-blocking or development entries you had are in that backup.",
        backup.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_standard_run_keys_may_be_edited() {
        assert_eq!(
            validate_run_key_hive(
                r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run"
            )
            .unwrap(),
            r"hkey_current_user\software\microsoft\windows\currentversion\run"
        );
        // The PowerShell drive spelling and a Registry:: prefix both normalise.
        assert!(
            validate_run_key_hive(r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Run").is_ok()
        );
        assert!(validate_run_key_hive(
            r"Registry::HKEY_LOCAL_MACHINE\Software\Wow6432Node\Microsoft\Windows\CurrentVersion\RunOnce"
        )
        .is_ok());
    }

    #[test]
    fn arbitrary_registry_keys_are_refused() {
        for bad in [
            r"HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\WinDefend",
            r"HKEY_CURRENT_USER\Software\Anything",
            r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon",
            "",
        ] {
            let err = validate_run_key_hive(bad).unwrap_err();
            assert!(
                matches!(err, Error::SecurityViolation { .. }),
                "{bad} should be a security violation, got {err:?}"
            );
        }
    }

    #[test]
    fn a_registry_key_with_an_injected_statement_is_refused() {
        // Even though every value is quoted before interpolation, a key that is
        // not on the allowlist never reaches PowerShell at all.
        let err = validate_run_key_hive(
            r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run'; Remove-Item C:\ -Recurse; '",
        )
        .unwrap_err();
        assert!(matches!(err, Error::SecurityViolation { .. }));
    }

    #[test]
    fn control_characters_are_refused_everywhere() {
        assert!(validate_run_key_hive("HKCU:\\Software\r\nEvil").is_err());
        assert!(validate_service_name("svc\nStop-Computer").is_err());
        assert!(validate_task_path("\\task\nEvil").is_err());
        assert!(check_quarantinable("C:\\a\0b", &["c:\\".into()]).is_err());
    }

    #[test]
    fn machine_wide_run_keys_are_identified_as_needing_elevation() {
        let hklm =
            validate_run_key_hive(r"HKLM:\Software\Microsoft\Windows\CurrentVersion\Run").unwrap();
        assert!(run_key_needs_elevation(&hklm));
        let hkcu =
            validate_run_key_hive(r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Run").unwrap();
        assert!(!run_key_needs_elevation(&hkcu));
    }

    #[test]
    fn task_paths_split_into_folder_and_name() {
        let (dir, name) = validate_task_path(r"\MyApp\Updater").unwrap();
        assert_eq!(dir, r"\MyApp\");
        assert_eq!(name, "Updater");

        let (dir, name) = validate_task_path(r"\TopLevelTask").unwrap();
        assert_eq!(dir, "\\");
        assert_eq!(name, "TopLevelTask");
    }

    #[test]
    fn windows_own_tasks_are_never_disabled_automatically() {
        let err =
            validate_task_path(r"\Microsoft\Windows\UpdateOrchestrator\Schedule Scan").unwrap_err();
        assert!(matches!(err, Error::SecurityViolation { .. }));
        // A relative name is not a task path.
        assert!(validate_task_path("Updater").is_err());
        // A folder, not a task.
        assert!(validate_task_path(r"\MyApp\").is_err());
    }

    #[test]
    fn critical_services_are_never_stopped() {
        for svc in ["WinDefend", "windefend", "EventLog", "RpcSs", "Winmgmt"] {
            let err = validate_service_name(svc).unwrap_err();
            assert!(
                matches!(err, Error::SecurityViolation { .. }),
                "{svc} must be protected"
            );
        }
        assert!(validate_service_name("SomeVendorUpdater").is_ok());
        assert!(validate_service_name("My Service 2").is_ok());
        // Anything that is not a plain identifier is refused.
        assert!(validate_service_name("svc; Stop-Computer").is_err());
        assert!(validate_service_name("svc$(whoami)").is_err());
    }

    #[test]
    fn quarantine_refuses_system_directories() {
        let roots = vec![
            r"C:\Users\bob\AppData\Local\Temp".to_string(),
            r"C:\Users\bob\AppData\Roaming".to_string(),
        ];
        for bad in [
            r"C:\Windows\System32\svchost.exe",
            r"C:\Windows\Temp\x.exe",
            r"C:\Program Files\App\app.exe",
            r"C:\Program Files (x86)\App\app.exe",
        ] {
            let err = check_quarantinable(bad, &roots).unwrap_err();
            assert!(
                matches!(err, Error::SecurityViolation { .. }),
                "{bad} must be refused"
            );
        }
    }

    #[test]
    fn quarantine_refuses_paths_outside_the_allowed_roots() {
        let roots = vec![r"C:\Users\bob\AppData\Local\Temp".to_string()];
        let err = check_quarantinable(r"C:\Users\bob\Documents\thesis.docx", &roots).unwrap_err();
        assert!(matches!(err, Error::SecurityViolation { .. }));
        // A path that merely starts with the same characters is not inside.
        let err =
            check_quarantinable(r"C:\Users\bob\AppData\Local\TempEvil\x.exe", &roots).unwrap_err();
        assert!(matches!(err, Error::SecurityViolation { .. }));
    }

    #[test]
    fn quarantine_refuses_traversal() {
        let roots = vec![r"C:\Users\bob\AppData\Local\Temp".to_string()];
        let err = check_quarantinable(
            r"C:\Users\bob\AppData\Local\Temp\..\..\..\..\Windows\System32\drivers\etc\hosts",
            &roots,
        )
        .unwrap_err();
        assert!(matches!(err, Error::SecurityViolation { .. }));
    }

    #[test]
    fn quarantine_accepts_files_in_user_writable_roots() {
        let roots = vec![
            r"C:\Users\bob\AppData\Local\Temp".to_string(),
            r"C:\Users\bob\AppData\Roaming".to_string(),
        ];
        assert!(
            check_quarantinable(r"C:\Users\bob\AppData\Local\Temp\dropper.exe", &roots).is_ok()
        );
        // Case and slash direction must not matter.
        assert!(check_quarantinable(r"c:/users/bob/appdata/roaming/x.exe", &roots).is_ok());
    }

    #[test]
    fn quarantine_names_are_unique_and_marked() {
        assert_eq!(
            quarantine_name(r"C:\Users\bob\AppData\Local\Temp\a.exe", "20260721T101500Z"),
            "20260721T101500Z_a.exe.quarantined"
        );
        assert_eq!(quarantine_name("plain.exe", "S"), "S_plain.exe.quarantined");
    }

    #[test]
    fn the_default_hosts_file_has_no_active_entries() {
        let entries = crate::security::integrity::parse_hosts(DEFAULT_HOSTS);
        assert!(entries.is_empty(), "the default hosts file must be inert");
        // And the scanner must consider what we write to be clean.
        assert!(crate::security::integrity::analyze_hosts(DEFAULT_HOSTS).is_empty());
    }

    #[test]
    fn run_key_backups_round_trip_as_json() {
        let backup = RunKeyBackup {
            hive: r"hkey_current_user\software\microsoft\windows\currentversion\run".into(),
            name: "Updater".into(),
            value: r"C:\Users\bob\AppData\Roaming\x.exe -silent".into(),
            removed_at: "2026-07-21T10:15:00Z".into(),
        };
        let json = serde_json::to_string(&vec![backup.clone()]).unwrap();
        let back: Vec<RunKeyBackup> = serde_json::from_str(&json).unwrap();
        assert_eq!(back[0], backup);
    }

    #[tokio::test]
    async fn manual_remediation_needs_no_platform_support() {
        let out = apply(&Remediation::Manual {
            instructions: "do the thing".into(),
        })
        .await
        .unwrap();
        assert_eq!(out, "do the thing");
    }
}
