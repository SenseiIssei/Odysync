//! Windows security posture and indicator-of-compromise (IOC) auditor.
//!
//! # What this is — and what it is not
//!
//! This module is **not an antivirus engine**. It does not have signatures, it
//! does not unpack executables, it does not hook the kernel, and it cannot see
//! anything a sufficiently privileged rootkit chooses to hide from user-mode
//! WMI/registry queries. Where real malware detection is needed it delegates to
//! the AV engine that ships with the OS (Microsoft Defender) via the
//! `Defender` PowerShell module, and otherwise it audits the *places* attackers
//! persist and the *configuration* that lets them in.
//!
//! Consequences worth stating plainly, because a security tool that oversells
//! itself is worse than none:
//!
//!   * A clean report **does not prove the machine is clean**. It proves that
//!     the specific locations checked here look ordinary right now.
//!   * Most checks are heuristic. "Unsigned binary in a user-writable
//!     directory" describes plenty of legitimate software (Electron apps,
//!     game launchers, dev tools) as well as most stealers. Findings are
//!     leads to investigate, not verdicts.
//!   * Several checks (BitLocker, SMBv1 feature state, some services) return
//!     nothing useful without elevation. The scan degrades quietly rather than
//!     failing, so an unelevated report is *less* complete, not more clean.
//!   * Reading only detects what is on disk or in the registry. Fileless
//!     in-memory implants are out of scope apart from the WMI subscription
//!     check.
//!
//! # Structure
//!
//! Every section is an independent submodule exposing `pub async fn scan() ->
//! Result<Vec<Finding>>`. [`scan`] runs them concurrently and records each
//! one's outcome in a [`SectionResult`]; a section that fails (missing cmdlet,
//! access denied, timeout) is reported and the rest of the scan continues. A
//! partial answer is far more useful here than an aborted one.
//!
//! I/O and parsing are deliberately separated: the PowerShell call sites are
//! thin, and every classification decision lives in a pure function that takes
//! already-deserialized rows. That is what makes this testable on a Linux CI
//! box without spawning a single process.

pub mod defender;
pub mod integrity;
pub mod network;
pub mod persistence;
pub mod posture;
pub mod remediate;

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use odysync_core::error::Result;

/// How much a finding should worry the user.
///
/// The derived `Ord` follows declaration order, so `Critical < Info`; sorting a
/// `Vec<Finding>` ascending by severity puts the scariest item first, which is
/// what [`ScanReport`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl Severity {
    /// Stable lowercase label, handy for logging and UI class names.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
            Severity::Info => "info",
        }
    }

    /// Raise this severity to `other` when `other` is more severe.
    pub fn escalate(self, other: Severity) -> Severity {
        if other < self {
            other
        } else {
            self
        }
    }
}

/// One thing the scan noticed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Stable slug, e.g. `defender-threat:Trojan:Win32/X`. Stable across scans
    /// so a UI can diff two reports and so acknowledgements can be persisted.
    pub id: String,
    pub severity: Severity,
    /// One of `malware`, `persistence`, `network`, `posture`, `integrity`,
    /// `account`.
    pub category: String,
    pub title: String,
    /// What it means, in plain language — this is shown to a worried human.
    pub detail: String,
    /// Paths, registry keys, ports, hashes, matching lines.
    pub evidence: Vec<String>,
    pub remediation: Option<Remediation>,
}

impl Finding {
    pub fn new(
        id: impl Into<String>,
        severity: Severity,
        category: &str,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Finding {
            id: id.into(),
            severity,
            category: category.to_string(),
            title: title.into(),
            detail: detail.into(),
            evidence: Vec::new(),
            remediation: None,
        }
    }

    pub fn with_evidence(mut self, evidence: impl IntoIterator<Item = String>) -> Self {
        self.evidence.extend(evidence);
        self
    }

    pub fn with_remediation(mut self, r: Remediation) -> Self {
        self.remediation = Some(r);
        self
    }
}

/// An action the user can authorise to fix a finding.
///
/// Deliberately a closed enum rather than "a command to run": the remediation
/// layer must be able to validate every parameter before touching the system,
/// and a free-form command string would make that impossible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Remediation {
    /// Hand the threat back to Defender for removal.
    RemoveDefenderThreat { threat_id: String },
    /// Delete an autostart registry value, after backing it up.
    DisableRunKey { hive: String, name: String },
    /// `Disable-ScheduledTask`; never applied to `\Microsoft\Windows\` tasks.
    DisableScheduledTask { task_path: String },
    /// Quarantine (rename + move), never a hard delete, and only inside
    /// user-writable directories.
    DeleteFile { path: String },
    StopAndDisableService { name: String },
    /// Back up `hosts` and restore the stock Windows contents.
    ResetHostsFile,
    /// Nothing safe to automate; tell the user what to do themselves.
    Manual { instructions: String },
}

/// Outcome of one scan section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionResult {
    pub name: String,
    pub ok: bool,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// The full result of one sweep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    /// Sorted most-severe first.
    pub findings: Vec<Finding>,
    /// RFC 3339 UTC timestamp of when the scan started.
    pub scanned_at: String,
    pub sections: Vec<SectionResult>,
}

impl ScanReport {
    /// Number of findings at each severity, for a summary strip in the UI.
    pub fn counts(&self) -> HashMap<Severity, usize> {
        let mut m = HashMap::new();
        for f in &self.findings {
            *m.entry(f.severity).or_insert(0) += 1;
        }
        m
    }

    /// True when at least one section failed, so the UI can say "incomplete"
    /// instead of implying the machine is clean.
    pub fn incomplete(&self) -> bool {
        self.sections.iter().any(|s| !s.ok)
    }
}

/// Section names, in report order.
pub const SECTIONS: &[&str] = &[
    "defender",
    "persistence",
    "integrity",
    "network",
    "posture",
];

/// Run every section concurrently and collect the findings.
///
/// Never returns an error: a scan that half-worked is still worth showing, so
/// section failures land in [`ScanReport::sections`] instead of propagating.
/// The sections are independent and each is dominated by process-spawn latency,
/// so running them in sequence would make the sweep as slow as their sum.
pub async fn scan() -> ScanReport {
    let scanned_at = chrono::Utc::now().to_rfc3339();

    let (defender, persistence, integrity, network, posture) = futures::join!(
        run_section("defender", defender::scan()),
        run_section("persistence", persistence::scan()),
        run_section("integrity", integrity::scan()),
        run_section("network", network::scan()),
        run_section("posture", posture::scan()),
    );

    let mut sections = Vec::new();
    let mut findings = Vec::new();
    for (result, mut found) in [defender, persistence, integrity, network, posture] {
        sections.push(result);
        findings.append(&mut found);
    }

    // Stable sort: severity first, then id, so two scans of an unchanged
    // machine produce byte-identical reports and a diff means something.
    findings.sort_by(|a, b| a.severity.cmp(&b.severity).then_with(|| a.id.cmp(&b.id)));

    ScanReport {
        findings,
        scanned_at,
        sections,
    }
}

async fn run_section(
    name: &str,
    fut: impl std::future::Future<Output = Result<Vec<Finding>>>,
) -> (SectionResult, Vec<Finding>) {
    let started = Instant::now();
    let (ok, error, findings) = match fut.await {
        Ok(f) => (true, None, f),
        Err(e) => {
            tracing::warn!(section = name, error = %e, "security scan section failed");
            (false, Some(e.to_string()), Vec::new())
        }
    };
    (
        SectionResult {
            name: name.to_string(),
            ok,
            error,
            duration_ms: started.elapsed().as_millis() as u64,
        },
        findings,
    )
}

// ---------------------------------------------------------------------------
// PowerShell plumbing
// ---------------------------------------------------------------------------

/// Wrap `s` in a PowerShell single-quoted literal, escaping embedded quotes.
///
/// Every value interpolated into a script must go through this. Most of what
/// this module handles — registry values, file names, service names, scheduled
/// task command lines — is attacker-controllable, and a single-quoted literal
/// is the only PowerShell string form with no expansion at all: inside it, the
/// sole metacharacter is `'`, which doubles to escape itself.
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Build a PowerShell array literal from strings, each individually quoted.
pub fn ps_string_array(items: &[String]) -> String {
    if items.is_empty() {
        return "@()".to_string();
    }
    let inner: Vec<String> = items.iter().map(|s| ps_quote(s)).collect();
    format!("@({})", inner.join(","))
}

/// Deserialize `ConvertTo-Json` output, which emits a bare object for a single
/// row and an array for several.
///
/// Returns an empty vector rather than an error for unparseable output: the
/// caller is a scan section that should degrade rather than abort, and PS
/// happily writes warnings to stdout that are not JSON.
pub fn parse_ps_json<T: serde::de::DeserializeOwned>(stdout: &str) -> Vec<T> {
    let stdout = stdout.trim();
    if stdout.is_empty() || stdout == "null" {
        return Vec::new();
    }
    if stdout.starts_with('[') {
        match serde_json::from_str::<Vec<T>>(stdout) {
            Ok(v) => v,
            Err(e) => {
                // Deliberately `warn`: a discarded payload silently empties a
                // whole scan section, and the user is told the section failed
                // for a reason that is usually wrong. Losing the actual serde
                // error to `debug` once cost hours of misdiagnosis.
                tracing::warn!(error = %e, "could not parse PowerShell JSON array");
                Vec::new()
            }
        }
    } else {
        match serde_json::from_str::<T>(stdout) {
            Ok(v) => vec![v],
            Err(e) => {
                tracing::warn!(error = %e, "could not parse PowerShell JSON object");
                Vec::new()
            }
        }
    }
}

/// Deserialize a single `ConvertTo-Json` object, or `None`.
pub fn parse_ps_object<T: serde::de::DeserializeOwned>(stdout: &str) -> Option<T> {
    parse_ps_json::<T>(stdout).into_iter().next()
}

/// Run a read-only PowerShell query and return stdout.
#[cfg(windows)]
/// Every shape `ConvertTo-Json` can produce for one logical collection.
///
/// PowerShell is remarkably inconsistent here, and each variant below was
/// observed in real output rather than guessed at.
#[derive(Deserialize)]
#[serde(untagged)]
enum PsCollection<T> {
    /// The normal case.
    List(Vec<T>),
    /// What `ConvertTo-Json` emits once a collection sits at the `-Depth`
    /// limit: `{"value":[...],"Count":2}` instead of a bare array.
    Wrapped { value: Vec<T> },
    /// A single element serialized unwrapped.
    One(T),
}

/// Deserialize a PowerShell collection into a `Vec`, whatever shape it arrived in.
///
/// Accepts `null`, a normal array, a `{"value":[…],"Count":n}` depth-limit
/// wrapper, and a bare single element. Anything unrecognised becomes empty
/// rather than an error.
///
/// This exists because the alternative is catastrophic rather than annoying: a
/// collection in an unexpected shape fails the *whole* payload, which fails the
/// whole scan section. In practice that meant one `"Resources":{"value":[…]}`
/// buried 1,600 characters into a 58 KB response reported the machine's
/// perfectly healthy Defender as "absent or superseded by a third-party AV".
///
/// `#[serde(default)]` does not help: it covers *missing* fields, not fields
/// that are present and null.
pub(crate) fn ps_collection<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(
        match Option::<PsCollection<T>>::deserialize(deserializer).unwrap_or(None) {
            Some(PsCollection::List(v)) => v,
            Some(PsCollection::Wrapped { value }) => value,
            Some(PsCollection::One(v)) => vec![v],
            None => Vec::new(),
        },
    )
}

pub(crate) async fn ps_query(script: &str, timeout: std::time::Duration) -> Result<String> {
    let out = odysync_core::proc::powershell(script, timeout).await?;
    Ok(out.stdout)
}

/// Run a mutating PowerShell script, failing loudly on a non-zero exit.
#[cfg(windows)]
pub(crate) async fn ps_mutate(
    what: &str,
    script: &str,
    timeout: std::time::Duration,
) -> Result<String> {
    let out = odysync_core::proc::powershell(script, timeout).await?;
    out.ok_stdout(what)
}

// ---------------------------------------------------------------------------
// Authenticode trust, shared by the persistence and network sections
// ---------------------------------------------------------------------------

/// What Authenticode says about one file on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTrust {
    pub path: String,
    pub exists: bool,
    /// `Valid`, `NotSigned`, `HashMismatch`, `UnknownError`, or `Missing`.
    pub status: String,
    /// Signer certificate subject, when there is one.
    pub signer: Option<String>,
}

impl FileTrust {
    /// True when the file exists but carries no valid Authenticode signature.
    ///
    /// A missing file is *not* "unsigned" — it is its own, separate finding.
    pub fn unsigned(&self) -> bool {
        self.exists && !self.status.eq_ignore_ascii_case("Valid")
    }

    /// Short human phrase for evidence lines.
    pub fn describe(&self) -> String {
        if !self.exists {
            return "target file does not exist".to_string();
        }
        match self.signer.as_deref() {
            Some(s) if self.status.eq_ignore_ascii_case("Valid") => {
                format!("signed ({})", first_cn(s))
            }
            _ => format!("signature: {}", self.status),
        }
    }
}

/// Pull the CN out of an X.500 subject, falling back to the whole string.
fn first_cn(subject: &str) -> String {
    for part in subject.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("CN=") {
            return rest.trim_matches('"').to_string();
        }
    }
    subject.to_string()
}

/// Lower-cased path -> trust. Windows paths are case-insensitive, and the
/// casing PowerShell returns rarely matches the casing in a registry value.
pub type TrustMap = HashMap<String, FileTrust>;

/// Case-insensitive lookup into a [`TrustMap`].
pub fn trust_of<'a>(map: &'a TrustMap, path: &str) -> Option<&'a FileTrust> {
    map.get(&path.to_ascii_lowercase())
}

/// Build a [`TrustMap`] from rows, keyed for [`trust_of`].
pub fn trust_map(rows: Vec<FileTrust>) -> TrustMap {
    rows.into_iter()
        .map(|r| (r.path.to_ascii_lowercase(), r))
        .collect()
}

/// Ask Windows whether each path exists and whether it is validly signed.
///
/// Batched in chunks because `Get-AuthenticodeSignature` costs tens of
/// milliseconds per file but a PowerShell spawn costs hundreds, and because a
/// single script with thousands of interpolated literals gets unwieldy.
#[cfg(windows)]
pub(crate) async fn query_file_trust(paths: &[String]) -> Result<TrustMap> {
    use std::time::Duration;

    let mut unique: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for p in paths {
        let key = p.to_ascii_lowercase();
        if !p.trim().is_empty() && seen.insert(key) {
            unique.push(p.clone());
        }
    }

    let mut map = TrustMap::new();
    for chunk in unique.chunks(120) {
        let script = format!(
            r#"$ErrorActionPreference='SilentlyContinue'
$paths = {}
$out = foreach ($p in $paths) {{
  $e = Test-Path -LiteralPath $p -PathType Leaf
  $st = 'Missing'; $sg = $null
  if ($e) {{
    $s = Get-AuthenticodeSignature -LiteralPath $p
    if ($s) {{
      $st = [string]$s.Status
      if ($s.SignerCertificate) {{ $sg = [string]$s.SignerCertificate.Subject }}
    }} else {{ $st = 'UnknownError' }}
  }}
  [pscustomobject]@{{ path = $p; exists = [bool]$e; status = $st; signer = $sg }}
}}
@($out) | ConvertTo-Json -Depth 3 -Compress"#,
            ps_string_array(chunk)
        );

        let stdout = ps_query(&script, Duration::from_secs(120)).await?;
        for row in parse_ps_json::<FileTrust>(&stdout) {
            map.insert(row.path.to_ascii_lowercase(), row);
        }
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Path classification, shared across sections
// ---------------------------------------------------------------------------

/// Normalise a Windows path for comparison: lowercase, forward slashes folded
/// to backslashes, surrounding quotes and whitespace stripped, trailing
/// separator removed.
/// Machine-wide environment variables are expanded too: scheduled tasks and
/// service image paths routinely store `%windir%\system32\...` unexpanded, and
/// without this every Microsoft-shipped task would look like it runs from a
/// non-standard location.
pub fn normalize_path(path: &str) -> String {
    let p = path.trim().trim_matches('"').trim();
    let p = p.replace('/', "\\");
    let p = p.trim_end_matches('\\').to_ascii_lowercase();
    expand_system_vars(&p)
}

/// Expand the environment variables whose values are the same on every Windows
/// install. Per-user ones (`%appdata%`, `%temp%`) deliberately are not
/// expanded — their value depends on who is logged in, and [`is_user_writable_dir`]
/// already recognises them by name.
fn expand_system_vars(lower_path: &str) -> String {
    const SUBS: &[(&str, &str)] = &[
        ("%systemroot%", "c:\\windows"),
        ("%windir%", "c:\\windows"),
        ("%programfiles(x86)%", "c:\\program files (x86)"),
        ("%programfiles%", "c:\\program files"),
        ("%programdata%", "c:\\programdata"),
        ("%systemdrive%", "c:"),
        ("\\systemroot", "c:\\windows"),
        ("\\??\\c:", "c:"),
    ];
    let mut out = lower_path.to_string();
    for (from, to) in SUBS {
        if out.starts_with(from) {
            out = format!("{to}{}", &out[from.len()..]);
            break;
        }
    }
    out
}

/// True when `path` sits inside a directory only an administrator can write:
/// `C:\Windows` or either `Program Files` tree.
///
/// Used in two directions. A binary *outside* these trees is mildly
/// interesting; a remediation that wants to delete something *inside* them is
/// refused outright.
pub fn is_system_dir(path: &str) -> bool {
    let p = normalize_path(path);
    const ROOTS: &[&str] = &[
        "\\windows",
        "\\program files (x86)",
        "\\program files",
        "\\programdata\\microsoft\\windows defender",
    ];
    // Strip an optional drive letter so `c:\windows` and `\windows` both match.
    let tail = strip_drive(&p);
    ROOTS
        .iter()
        .any(|r| tail == *r || tail.starts_with(&format!("{r}\\")))
}

/// Remove a leading `x:` drive designator, if present.
pub fn strip_drive(path: &str) -> &str {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        &path[2..]
    } else {
        path
    }
}

/// True when `path` lives somewhere an unprivileged process can write: a user
/// profile, `AppData`, `ProgramData`, or a temp directory.
///
/// This is the "malware can drop here without admin" test, and the same test
/// gates what remediation is allowed to quarantine.
///
/// `C:\Windows\Temp` is intentionally *excluded* even though it is writable by
/// everyone: the remediation layer keys off this function, and nothing this
/// tool does should ever be able to delete a file under `C:\Windows`.
/// Expand `%VAR%` references in a Windows path.
///
/// Windows stores autostart and scheduled-task commands with variables intact
/// (`%SystemRoot%\system32\foo.exe`), and two thirds of the scheduled tasks on
/// a normal machine use them. Checking such a path for existence, or asking
/// Authenticode about it, always fails — which the persistence checks then
/// reported as "the program it runs no longer exists" for 65 perfectly healthy
/// Windows tasks.
///
/// Unknown variables are left as-is: a path we cannot resolve must stay
/// obviously unresolved rather than silently becoming a different path.
pub fn expand_env_vars(path: &str) -> String {
    if !path.contains('%') {
        return path.to_string();
    }

    let mut out = String::with_capacity(path.len());
    let mut rest = path;

    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];

        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) if !name.is_empty() => out.push_str(&value),
                    // Unknown or empty (`%%`): keep the original text verbatim.
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                // Unterminated '%' — nothing more to expand.
                out.push('%');
                out.push_str(after);
                return out;
            }
        }
    }

    out.push_str(rest);
    out
}

pub fn is_user_writable_dir(path: &str) -> bool {
    let p = normalize_path(path);
    let tail = strip_drive(&p);
    if is_system_dir(&p) {
        return false;
    }
    // Unexpanded per-user variables count: they can only ever resolve inside a
    // user profile.
    const USER_VARS: &[&str] = &[
        "%appdata%",
        "%localappdata%",
        "%temp%",
        "%tmp%",
        "%userprofile%",
        "%public%",
    ];
    if USER_VARS.iter().any(|v| p.starts_with(v)) {
        return true;
    }
    const ROOTS: &[&str] = &[
        "\\users\\",
        "\\programdata\\",
        "\\temp\\",
        "\\$recycle.bin\\",
        "\\perflogs\\",
    ];
    ROOTS.iter().any(|r| tail.starts_with(r) || tail.contains(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two thirds of a normal machine's scheduled tasks store their command
    /// with `%SystemRoot%` intact. Without expansion every one of them looked
    /// like a task pointing at a deleted file: 72 findings on a machine that
    /// had 7 genuinely missing targets.
    #[test]
    fn env_vars_in_paths_are_expanded() {
        std::env::set_var("ODYSYNC_TEST_ROOT", r"C:\Windows");

        assert_eq!(
            expand_env_vars(r"%ODYSYNC_TEST_ROOT%\system32\foo.exe"),
            r"C:\Windows\system32\foo.exe"
        );
        // Several in one path.
        assert_eq!(
            expand_env_vars(r"%ODYSYNC_TEST_ROOT%\a\%ODYSYNC_TEST_ROOT%"),
            r"C:\Windows\a\C:\Windows"
        );
        // Nothing to do.
        assert_eq!(expand_env_vars(r"C:\plain\path.exe"), r"C:\plain\path.exe");

        // An unknown variable must stay visibly unresolved rather than quietly
        // collapsing into a different, possibly existing, path.
        assert_eq!(
            expand_env_vars(r"%ODYSYNC_NOT_SET_ANYWHERE%\x.exe"),
            r"%ODYSYNC_NOT_SET_ANYWHERE%\x.exe"
        );
        // Malformed input must not panic or truncate.
        assert_eq!(expand_env_vars("%unterminated"), "%unterminated");
        assert_eq!(expand_env_vars("100%"), "100%");
        assert_eq!(expand_env_vars("a%%b"), "a%%b");

        std::env::remove_var("ODYSYNC_TEST_ROOT");
    }

    #[test]
    fn ps_quote_neutralizes_statement_injection() {
        assert_eq!(ps_quote("plain"), "'plain'");
        assert_eq!(ps_quote("it's"), "'it''s'");

        // The attack: a registry value or file name that closes the literal and
        // appends a second statement. After quoting there is exactly one
        // unescaped quote at each end, so the payload stays inert data.
        let evil = "x'; Remove-Item C:\\ -Recurse -Force; '";
        let quoted = ps_quote(evil);
        assert_eq!(quoted, "'x''; Remove-Item C:\\ -Recurse -Force; '''");

        // Every interior quote is doubled, so the literal cannot terminate early.
        let interior = &quoted[1..quoted.len() - 1];
        let mut chars = interior.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                assert_eq!(chars.next(), Some('\''), "unpaired quote inside literal");
            }
        }
    }

    #[test]
    fn ps_quote_handles_the_other_shapes_registry_values_take() {
        // Backticks and $ are not special inside a single-quoted literal.
        assert_eq!(ps_quote("$(whoami)"), "'$(whoami)'");
        assert_eq!(ps_quote("a`nb"), "'a`nb'");
        assert_eq!(ps_quote(""), "''");
        assert_eq!(ps_quote("'"), "''''");
    }

    #[test]
    fn ps_string_array_quotes_every_element() {
        assert_eq!(ps_string_array(&[]), "@()");
        assert_eq!(
            ps_string_array(&["a".into(), "b'c".into()]),
            "@('a','b''c')"
        );
    }

    #[test]
    fn parse_ps_json_accepts_a_bare_object_and_an_array() {
        #[derive(serde::Deserialize, PartialEq, Debug)]
        struct Row {
            a: u32,
        }
        assert_eq!(parse_ps_json::<Row>(r#"{"a":1}"#), vec![Row { a: 1 }]);
        assert_eq!(
            parse_ps_json::<Row>(r#"[{"a":1},{"a":2}]"#),
            vec![Row { a: 1 }, Row { a: 2 }]
        );
        assert_eq!(parse_ps_json::<Row>("   "), Vec::<Row>::new());
        assert_eq!(parse_ps_json::<Row>("null"), Vec::<Row>::new());
        // Non-JSON noise on stdout must not blow up a scan section.
        assert_eq!(parse_ps_json::<Row>("WARNING: something"), Vec::<Row>::new());
    }

    #[test]
    fn severity_orders_worst_first_and_escalates() {
        let mut v = vec![Severity::Low, Severity::Critical, Severity::Medium];
        v.sort();
        assert_eq!(v, vec![Severity::Critical, Severity::Medium, Severity::Low]);
        assert_eq!(Severity::Medium.escalate(Severity::Critical), Severity::Critical);
        assert_eq!(Severity::Critical.escalate(Severity::Low), Severity::Critical);
    }

    #[test]
    fn severity_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&Severity::Critical).unwrap(),
            "\"critical\""
        );
    }

    #[test]
    fn remediation_serializes_with_a_kind_tag() {
        let json = serde_json::to_value(Remediation::DisableRunKey {
            hive: "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run".into(),
            name: "Updater".into(),
        })
        .unwrap();
        assert_eq!(json["kind"], "disable-run-key");
        assert_eq!(json["name"], "Updater");

        let json = serde_json::to_value(Remediation::ResetHostsFile).unwrap();
        assert_eq!(json["kind"], "reset-hosts-file");
    }

    #[test]
    fn system_dirs_are_recognised_regardless_of_case_or_slash() {
        assert!(is_system_dir(r"C:\Windows\System32\svchost.exe"));
        assert!(is_system_dir(r"c:/windows/system32/cmd.exe"));
        assert!(is_system_dir(r"C:\Program Files\Foo\foo.exe"));
        assert!(is_system_dir(r"C:\Program Files (x86)\Foo\foo.exe"));
        assert!(!is_system_dir(r"C:\Users\bob\AppData\Local\Temp\a.exe"));
        // Near-misses must not be treated as trusted.
        assert!(!is_system_dir(r"C:\Windows2\evil.exe"));
        assert!(!is_system_dir(r"C:\Program Files Evil\evil.exe"));
    }

    #[test]
    fn unexpanded_system_variables_still_resolve_to_system_dirs() {
        // Scheduled tasks and services store these verbatim; without expansion
        // every Microsoft-shipped task looks like it runs from nowhere special.
        assert!(is_system_dir(r"%windir%\system32\usoclient.exe"));
        assert!(is_system_dir(r"%SystemRoot%\System32\svchost.exe"));
        assert!(is_system_dir(r"%ProgramFiles%\Windows Defender\MsMpEng.exe"));
        assert!(is_system_dir(r"\SystemRoot\System32\drivers\http.sys"));
        assert!(!is_system_dir(r"%APPDATA%\evil.exe"));
    }

    #[test]
    fn unexpanded_user_variables_count_as_user_writable() {
        assert!(is_user_writable_dir(r"%APPDATA%\thing\thing.exe"));
        assert!(is_user_writable_dir(r"%TEMP%\a.exe"));
        assert!(!is_user_writable_dir(r"%windir%\system32\a.exe"));
    }

    #[test]
    fn user_writable_dirs_cover_the_places_droppers_land() {
        assert!(is_user_writable_dir(
            r"C:\Users\bob\AppData\Local\Temp\stealer.exe"
        ));
        assert!(is_user_writable_dir(r"C:\ProgramData\x\y.exe"));
        assert!(is_user_writable_dir(r"C:\Users\bob\Downloads\a.exe"));
        assert!(!is_user_writable_dir(r"C:\Windows\System32\svchost.exe"));
        assert!(!is_user_writable_dir(r"C:\Program Files\App\app.exe"));
    }

    #[test]
    fn file_trust_distinguishes_missing_from_unsigned() {
        let missing = FileTrust {
            path: "x".into(),
            exists: false,
            status: "Missing".into(),
            signer: None,
        };
        assert!(!missing.unsigned());
        assert_eq!(missing.describe(), "target file does not exist");

        let unsigned = FileTrust {
            path: "x".into(),
            exists: true,
            status: "NotSigned".into(),
            signer: None,
        };
        assert!(unsigned.unsigned());

        let signed = FileTrust {
            path: "x".into(),
            exists: true,
            status: "Valid".into(),
            signer: Some("CN=Microsoft Windows, O=Microsoft Corporation, C=US".into()),
        };
        assert!(!signed.unsigned());
        assert_eq!(signed.describe(), "signed (Microsoft Windows)");
    }

    #[test]
    fn trust_lookup_is_case_insensitive() {
        let map = trust_map(vec![FileTrust {
            path: r"C:\Users\Bob\a.exe".into(),
            exists: true,
            status: "NotSigned".into(),
            signer: None,
        }]);
        assert!(trust_of(&map, r"c:\users\bob\A.EXE").is_some());
        assert!(trust_of(&map, r"c:\other.exe").is_none());
    }

    #[test]
    fn a_report_reports_its_own_incompleteness() {
        let report = ScanReport {
            findings: vec![
                Finding::new("b", Severity::Low, "posture", "t", "d"),
                Finding::new("a", Severity::Critical, "malware", "t", "d"),
            ],
            scanned_at: "2026-07-21T00:00:00Z".into(),
            sections: vec![SectionResult {
                name: "defender".into(),
                ok: false,
                error: Some("access denied".into()),
                duration_ms: 5,
            }],
        };
        assert!(report.incomplete());
        assert_eq!(report.counts()[&Severity::Critical], 1);
    }
}
