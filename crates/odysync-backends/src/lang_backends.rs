//! Language runtime package managers: pip, cargo, npm, go, dotnet, vscode, powershell.

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, InstalledPackage, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

const SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const INSTALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

// ── Pip ──────────────────────────────────────────────────────────────────────

pub struct PipBackend;

impl PipBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PipBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for PipBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Pip
    }
    fn display_name(&self) -> &str {
        "Python pip"
    }

    async fn is_available(&self) -> bool {
        proc::exists("pip", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run(
            "pip",
            &["list", "--outdated", "--format=json"],
            SCAN_TIMEOUT,
        )
        .await?;
        let packages: Vec<PipOutdated> = serde_json::from_str(&out.stdout)
            .map_err(|e| Error::parse("pip", format!("JSON parse: {e}")))?;
        let kind = self.kind();
        Ok(packages
            .into_iter()
            .map(|p| {
                let name = p.name.clone();
                UpdateCandidate {
                    id: PackageId::new(kind, p.name),
                    name,
                    installed: Version::parse(&p.version),
                    available: Version::parse(&p.latest_version),
                    size_bytes: None,
                    expected_sha256: None,
                }
            })
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }
        // The version must be attached to the name as `name==version` in a
        // single argument. Passing it as a separate arg made pip read it as a
        // second package to install (literally named "1.5.1"), so every pip
        // upgrade failed — the cause of a machine-full of "failed" history
        // entries.
        let spec = pip_install_spec(&candidate.id.native, candidate.available.raw());
        let out = proc::run("pip", &["install", "--upgrade", &spec], INSTALL_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("pip install --upgrade {spec}"),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let out = proc::run("pip", &["list", "--format=json"], SCAN_TIMEOUT).await?;
        parse_pip_list(&out.stdout)
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = proc::run("pip", &["show", &candidate.id.native], SCAN_TIMEOUT).await?;
        for line in out.stdout.lines() {
            if let Some(v) = line.strip_prefix("Version:") {
                return Ok(Some(v.trim().to_string()));
            }
        }
        Ok(None)
    }
}

#[derive(serde::Deserialize)]
struct PipOutdated {
    name: String,
    version: String,
    latest_version: String,
}

#[derive(serde::Deserialize)]
struct PipInstalled {
    name: String,
    version: String,
}

/// Build the pinned requirement pip expects: `name==version`, one argument.
fn pip_install_spec(name: &str, version: &str) -> String {
    format!("{name}=={version}")
}

/// Parse `pip list --format=json`: `[{"name": "...", "version": "..."}, …]`.
fn parse_pip_list(stdout: &str) -> Result<Vec<InstalledPackage>> {
    let packages: Vec<PipInstalled> = serde_json::from_str(stdout)
        .map_err(|e| Error::parse("pip", format!("JSON parse: {e}")))?;
    Ok(packages
        .into_iter()
        .map(|p| InstalledPackage {
            id: PackageId::new(BackendKind::Pip, &p.name),
            name: p.name,
            version: p.version,
        })
        .collect())
}

// ── Cargo ────────────────────────────────────────────────────────────────────

pub struct CargoBackend;

impl CargoBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CargoBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for CargoBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Cargo
    }
    fn display_name(&self) -> &str {
        "Rust cargo"
    }

    async fn is_available(&self) -> bool {
        proc::exists("cargo", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // `cargo install --list` reports only what is installed — cargo has no
        // notion of a "latest" version offline, so upgrade detection would need
        // a crates.io lookup that is not wired up. Rather than spawn cargo and
        // throw the output away, report no candidates and let the installed
        // inventory come from `list_installed`.
        Ok(Vec::new())
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let out = proc::run("cargo", &["install", "--list"], SCAN_TIMEOUT).await?;
        Ok(parse_cargo_install_list(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }
        let out = proc::run(
            "cargo",
            &[
                "install",
                &candidate.id.native,
                "--version",
                candidate.available.raw(),
            ],
            INSTALL_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("cargo install {}", candidate.id.native),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = proc::run("cargo", &["install", "--list"], SCAN_TIMEOUT).await?;
        for line in out.stdout.lines() {
            if let Some((name, rest)) = line.split_once(' ') {
                if name == candidate.id.native {
                    if let Some(v) = rest.strip_prefix('v') {
                        return Ok(Some(v.trim_end_matches(':').trim().to_string()));
                    }
                }
            }
        }
        Ok(None)
    }
}

/// Parse `cargo install --list`.
///
/// ```text
/// ripgrep v14.1.0:
///     rg
/// cargo-edit v0.12.2 (/home/u/src/cargo-edit):
///     cargo-add
/// ```
///
/// Crate lines start at column 0; the binaries each crate installs are indented
/// beneath it and must not be mistaken for packages.
fn parse_cargo_install_list(stdout: &str) -> Vec<InstalledPackage> {
    stdout
        .lines()
        .filter(|line| !line.starts_with(char::is_whitespace))
        .filter_map(|line| {
            let (name, rest) = line.trim_end().split_once(' ')?;
            // The version token is `vX.Y.Z`; a path-installed crate appends the
            // source directory in parentheses, which is not part of the version.
            let version = rest.strip_prefix('v')?.split_whitespace().next()?;
            let version = version.trim_end_matches(':');
            if name.is_empty() || version.is_empty() {
                return None;
            }
            Some(InstalledPackage {
                id: PackageId::new(BackendKind::Cargo, name),
                name: name.to_string(),
                version: version.to_string(),
            })
        })
        .collect()
}

// ── Npm ──────────────────────────────────────────────────────────────────────

pub struct NpmBackend;

impl NpmBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NpmBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for NpmBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Npm
    }
    fn display_name(&self) -> &str {
        "Node.js npm (global)"
    }

    async fn is_available(&self) -> bool {
        proc::exists("npm", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("npm", &["outdated", "-g", "--json"], SCAN_TIMEOUT).await?;
        // npm outdated exits non-zero when updates are available
        let packages: std::collections::HashMap<String, NpmOutdated> =
            serde_json::from_str(&out.stdout).unwrap_or_default();
        let kind = self.kind();
        Ok(packages
            .into_iter()
            .map(|(name, p)| {
                let id_name = name.clone();
                UpdateCandidate {
                    id: PackageId::new(kind, id_name),
                    name,
                    installed: Version::parse(&p.current.unwrap_or_default()),
                    available: Version::parse(&p.latest.unwrap_or_default()),
                    size_bytes: None,
                    expected_sha256: None,
                }
            })
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }
        let pkg = format!("{}@{}", candidate.id.native, candidate.available.raw());
        let out = proc::run("npm", &["install", "-g", &pkg], INSTALL_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("npm install -g {pkg}"),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let out = proc::run("npm", &["list", "-g", "--depth=0", "--json"], SCAN_TIMEOUT).await?;
        Ok(parse_npm_global_list(&out.stdout))
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = proc::run(
            "npm",
            &["list", "-g", &candidate.id.native, "--json"],
            SCAN_TIMEOUT,
        )
        .await?;
        let parsed: serde_json::Value = serde_json::from_str(&out.stdout).unwrap_or_default();
        if let Some(version) = parsed
            .pointer(&format!("/dependencies/{}/version", candidate.id.native))
            .and_then(|v| v.as_str())
        {
            return Ok(Some(version.to_string()));
        }
        Ok(None)
    }
}

#[derive(serde::Deserialize)]
struct NpmOutdated {
    current: Option<String>,
    /// Present in npm's output; Odysync always targets `latest`.
    #[allow(dead_code)]
    wanted: Option<String>,
    latest: Option<String>,
}

/// Parse `npm list -g --depth=0 --json`.
///
/// ```json
/// {"name": "lib", "dependencies": {"npm": {"version": "10.5.0"}}}
/// ```
///
/// Entries npm marks as missing carry no `version`; those are skipped rather
/// than reported with an empty version string.
fn parse_npm_global_list(stdout: &str) -> Vec<InstalledPackage> {
    let parsed: serde_json::Value = match serde_json::from_str(stdout) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(deps) = parsed.get("dependencies").and_then(|d| d.as_object()) else {
        return Vec::new();
    };
    deps.iter()
        .filter_map(|(name, entry)| {
            let version = entry.get("version")?.as_str()?;
            Some(InstalledPackage {
                id: PackageId::new(BackendKind::Npm, name.as_str()),
                name: name.clone(),
                version: version.to_string(),
            })
        })
        .collect()
}

// ── Go ───────────────────────────────────────────────────────────────────────

pub struct GoBackend;

impl GoBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GoBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for GoBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Go
    }
    fn display_name(&self) -> &str {
        "Go modules"
    }

    async fn is_available(&self) -> bool {
        proc::exists("go", &["version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // go list -m -u all lists modules with available updates
        let out = proc::run("go", &["list", "-m", "-u", "all"], SCAN_TIMEOUT).await?;
        let kind = self.kind();
        let mut candidates = Vec::new();
        for line in out.stdout.lines() {
            // Format: "module version [latest]"
            // Lines with updates have " [latest_version]" suffix
            if let Some(idx) = line.rfind(" [") {
                let rest = &line[idx + 2..];
                if let Some(latest) = rest.strip_suffix(']') {
                    let parts: Vec<&str> = line[..idx].split_whitespace().collect();
                    if parts.len() >= 2 {
                        candidates.push(UpdateCandidate {
                            id: PackageId::new(kind, parts[0]),
                            name: parts[0].to_string(),
                            installed: Version::parse(parts[1]),
                            available: Version::parse(latest),
                            size_bytes: None,
                            expected_sha256: None,
                        });
                    }
                }
            }
        }
        Ok(candidates)
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }
        // `go install pkg@version` is the supported way to install a pinned
        // tool. `go get -u` upgraded to *latest* (ignoring the target version)
        // and is deprecated for installing executables outside a module.
        let spec = format!("{}@{}", candidate.id.native, candidate.available.raw());
        let out = proc::run("go", &["install", &spec], INSTALL_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("go install {spec}"),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = proc::run("go", &["list", "-m", &candidate.id.native], SCAN_TIMEOUT).await?;
        let parts: Vec<&str> = out.stdout.split_whitespace().collect();
        if parts.len() >= 2 {
            return Ok(Some(parts[1].to_string()));
        }
        Ok(None)
    }
}

// ── Dotnet Tool ──────────────────────────────────────────────────────────────

pub struct DotnetToolBackend;

impl DotnetToolBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DotnetToolBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for DotnetToolBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::DotnetTool
    }
    fn display_name(&self) -> &str {
        ".NET global tools"
    }

    async fn is_available(&self) -> bool {
        proc::exists("dotnet", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("dotnet", &["tool", "list", "-g"], SCAN_TIMEOUT).await?;
        let kind = self.kind();
        let mut candidates = Vec::new();
        for line in out.stdout.lines().skip(2) {
            // Format: "package_id    version    commands"
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                // We don't know the latest version without querying nuget;
                // report with Unknown available for now
                candidates.push(UpdateCandidate {
                    id: PackageId::new(kind, parts[0]),
                    name: parts[0].to_string(),
                    installed: Version::parse(parts[1]),
                    available: Version::Unknown(String::new()),
                    size_bytes: None,
                    expected_sha256: None,
                });
            }
        }
        Ok(candidates)
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }
        let out = proc::run(
            "dotnet",
            &[
                "tool",
                "update",
                "-g",
                &candidate.id.native,
                "--version",
                candidate.available.raw(),
            ],
            INSTALL_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("dotnet tool update -g {}", candidate.id.native),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let out = proc::run("dotnet", &["tool", "list", "--global"], SCAN_TIMEOUT).await?;
        Ok(parse_dotnet_tool_list(&out.stdout))
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = proc::run("dotnet", &["tool", "list", "-g"], SCAN_TIMEOUT).await?;
        for line in out.stdout.lines().skip(2) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[0] == candidate.id.native {
                return Ok(Some(parts[1].to_string()));
            }
        }
        Ok(None)
    }
}

/// Parse `dotnet tool list --global`.
///
/// ```text
/// Package Id      Version      Commands
/// --------------------------------------
/// dotnet-ef       8.0.0        dotnet-ef
/// ```
///
/// The two header lines are skipped, matching `installed_version`.
fn parse_dotnet_tool_list(stdout: &str) -> Vec<InstalledPackage> {
    stdout
        .lines()
        .skip(2)
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                return None;
            }
            Some(InstalledPackage {
                id: PackageId::new(BackendKind::DotnetTool, parts[0]),
                name: parts[0].to_string(),
                version: parts[1].to_string(),
            })
        })
        .collect()
}

// ── VS Code Extensions ───────────────────────────────────────────────────────

pub struct VscodeExtensionBackend;

impl VscodeExtensionBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for VscodeExtensionBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for VscodeExtensionBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::VscodeExtension
    }
    fn display_name(&self) -> &str {
        "VS Code extensions"
    }

    async fn is_available(&self) -> bool {
        // Check for either `code` or `code-insiders`
        proc::exists("code", &["--version"]).await
            || proc::exists("code-insiders", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let cmd = if proc::exists("code", &["--version"]).await {
            "code"
        } else {
            "code-insiders"
        };
        let out = proc::run(cmd, &["--list-extensions", "--show-versions"], SCAN_TIMEOUT).await?;
        let kind = self.kind();
        let mut candidates = Vec::new();
        for line in out.stdout.lines() {
            // Format: "extension.id@1.2.3"
            if let Some((id, version)) = line.rsplit_once('@') {
                candidates.push(UpdateCandidate {
                    id: PackageId::new(kind, id),
                    name: id.to_string(),
                    installed: Version::parse(version),
                    available: Version::Unknown(String::new()),
                    size_bytes: None,
                    expected_sha256: None,
                });
            }
        }
        Ok(candidates)
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        let cmd = if proc::exists("code", &["--version"]).await {
            "code"
        } else {
            "code-insiders"
        };
        let out = proc::run(
            cmd,
            &["--install-extension", &candidate.id.native, "--force"],
            INSTALL_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("{} --install-extension {}", cmd, candidate.id.native),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let cmd = if proc::exists("code", &["--version"]).await {
            "code"
        } else {
            "code-insiders"
        };
        let out = proc::run(cmd, &["--list-extensions", "--show-versions"], SCAN_TIMEOUT).await?;
        Ok(parse_vscode_extensions(&out.stdout))
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let cmd = if proc::exists("code", &["--version"]).await {
            "code"
        } else {
            "code-insiders"
        };
        let out = proc::run(cmd, &["--list-extensions", "--show-versions"], SCAN_TIMEOUT).await?;
        for line in out.stdout.lines() {
            if let Some((id, version)) = line.rsplit_once('@') {
                if id == candidate.id.native {
                    return Ok(Some(version.to_string()));
                }
            }
        }
        Ok(None)
    }
}

/// Parse `code --list-extensions --show-versions`: one `publisher.name@version`
/// per line.
fn parse_vscode_extensions(stdout: &str) -> Vec<InstalledPackage> {
    stdout
        .lines()
        .filter_map(|line| {
            let (id, version) = line.trim().rsplit_once('@')?;
            if id.is_empty() || version.is_empty() {
                return None;
            }
            Some(InstalledPackage {
                id: PackageId::new(BackendKind::VscodeExtension, id),
                name: id.to_string(),
                version: version.to_string(),
            })
        })
        .collect()
}

// ── PowerShell Modules ───────────────────────────────────────────────────────

pub struct PowerShellModuleBackend;

impl PowerShellModuleBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PowerShellModuleBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for PowerShellModuleBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::PowerShellModule
    }
    fn display_name(&self) -> &str {
        "PowerShell modules"
    }

    async fn is_available(&self) -> bool {
        // Short-circuit on the cheap cfg check before spawning anything.
        cfg!(windows)
            && (proc::exists("pwsh", &["--version"]).await
                || proc::exists("powershell", &["-Command", "$PSVersionTable.PSVersion"]).await)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let pwsh = if proc::exists("pwsh", &["--version"]).await {
            "pwsh"
        } else {
            "powershell"
        };
        let script = "Get-InstalledModule | Select-Object Name, Version | ConvertTo-Json";
        let out = proc::run(pwsh, &["-NoProfile", "-Command", script], SCAN_TIMEOUT).await?;
        let kind = self.kind();
        let modules: Vec<PsModule> = serde_json::from_str(&out.stdout).unwrap_or_default();
        Ok(modules
            .into_iter()
            .map(|m| {
                let name = m.name.clone();
                UpdateCandidate {
                    id: PackageId::new(kind, m.name),
                    name,
                    installed: Version::parse(&m.version),
                    available: Version::Unknown(String::new()),
                    size_bytes: None,
                    expected_sha256: None,
                }
            })
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        let pwsh = if proc::exists("pwsh", &["--version"]).await {
            "pwsh"
        } else {
            "powershell"
        };
        let out = proc::run(
            pwsh,
            &[
                "-NoProfile",
                "-Command",
                &format!("Update-Module -Name {} -Force", candidate.id.native),
            ],
            INSTALL_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("Update-Module {}", candidate.id.native),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let pwsh = if proc::exists("pwsh", &["--version"]).await {
            "pwsh"
        } else {
            "powershell"
        };
        let script = format!(
            "Get-InstalledModule -Name {} | Select-Object -ExpandProperty Version",
            candidate.id.native
        );
        let out = proc::run(pwsh, &["-NoProfile", "-Command", &script], SCAN_TIMEOUT).await?;
        let v = out.stdout.trim().to_string();
        if v.is_empty() {
            Ok(None)
        } else {
            Ok(Some(v))
        }
    }
}

#[derive(serde::Deserialize)]
struct PsModule {
    name: String,
    version: String,
}

// ── JetBrains Plugins ────────────────────────────────────────────────────────

pub struct JetbrainsPluginBackend;

impl JetbrainsPluginBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JetbrainsPluginBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for JetbrainsPluginBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::JetbrainsPlugin
    }
    fn display_name(&self) -> &str {
        "JetBrains IDE plugins"
    }

    async fn is_available(&self) -> bool {
        // Check if any JetBrains IDE config directory exists
        if cfg!(windows) {
            std::path::Path::new(&std::env::var("APPDATA").unwrap_or_default())
                .join("JetBrains")
                .exists()
        } else if cfg!(target_os = "macos") {
            std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
                .join("Library/Application Support/JetBrains")
                .exists()
        } else {
            std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
                .join(".config/JetBrains")
                .exists()
        }
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // Scan JetBrains plugin directories for installed plugins
        // Plugins are stored in <IDE-config>/plugins/ as directories with plugin.xml
        let kind = self.kind();
        let mut candidates = Vec::new();

        let base = if cfg!(windows) {
            std::env::var("APPDATA").unwrap_or_default() + "/JetBrains"
        } else if cfg!(target_os = "macos") {
            std::env::var("HOME").unwrap_or_default() + "/Library/Application Support/JetBrains"
        } else {
            std::env::var("HOME").unwrap_or_default() + "/.config/JetBrains"
        };

        let base_path = std::path::Path::new(&base);
        if !base_path.exists() {
            return Ok(candidates);
        }

        // Iterate over IDE directories (e.g., IntelliJIdea2024.3)
        if let Ok(entries) = std::fs::read_dir(base_path) {
            for entry in entries.flatten() {
                let plugins_dir = entry.path().join("plugins");
                if !plugins_dir.exists() {
                    continue;
                }
                if let Ok(plugin_entries) = std::fs::read_dir(&plugins_dir) {
                    for plugin_entry in plugin_entries.flatten() {
                        let plugin_path = plugin_entry.path();
                        let plugin_name = plugin_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        // Read version from plugin.xml if available
                        let meta_inf = plugin_path.join("META-INF/plugin.xml");
                        let version = if let Ok(content) = std::fs::read_to_string(&meta_inf) {
                            extract_xml_version(&content)
                        } else {
                            None
                        };
                        candidates.push(UpdateCandidate {
                            id: PackageId::new(kind, &plugin_name),
                            name: plugin_name,
                            installed: Version::parse(&version.unwrap_or_default()),
                            available: Version::Unknown(String::new()),
                            size_bytes: None,
                            expected_sha256: None,
                        });
                    }
                }
            }
        }

        Ok(candidates)
    }

    async fn apply(&self, _candidate: &UpdateCandidate) -> Result<()> {
        // JetBrains plugins are managed by the IDE itself; we can't install
        // them from the CLI without the JetBrains Marketplace CLI tool.
        Err(Error::Verification {
            package: _candidate.id.to_string(),
            detail: "JetBrains plugins must be updated from within the IDE".into(),
        })
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        // Scan plugin dirs for this specific plugin
        let base = if cfg!(windows) {
            std::env::var("APPDATA").unwrap_or_default() + "/JetBrains"
        } else if cfg!(target_os = "macos") {
            std::env::var("HOME").unwrap_or_default() + "/Library/Application Support/JetBrains"
        } else {
            std::env::var("HOME").unwrap_or_default() + "/.config/JetBrains"
        };

        let base_path = std::path::Path::new(&base);
        if let Ok(entries) = std::fs::read_dir(base_path) {
            for entry in entries.flatten() {
                let plugin_path = entry.path().join("plugins").join(&candidate.id.native);
                let meta_inf = plugin_path.join("META-INF/plugin.xml");
                if let Ok(content) = std::fs::read_to_string(&meta_inf) {
                    if let Some(v) = extract_xml_version(&content) {
                        return Ok(Some(v));
                    }
                }
            }
        }
        Ok(None)
    }
}

fn extract_xml_version(xml: &str) -> Option<String> {
    // Simple extraction of version="..." from the <idea-plugin> tag
    if let Some(start) = xml.find("version=\"") {
        let rest = &xml[start + 9..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }
    None
}

// ── Windows Optional Features ────────────────────────────────────────────────

pub struct WindowsOptionalFeatureBackend;

impl WindowsOptionalFeatureBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WindowsOptionalFeatureBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for WindowsOptionalFeatureBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::WindowsOptionalFeature
    }
    fn display_name(&self) -> &str {
        "Windows optional features"
    }

    async fn is_available(&self) -> bool {
        cfg!(windows)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }
        let out = proc::run(
            "powershell",
            &["-NoProfile", "-Command", "Get-WindowsOptionalFeature -Online | Where-Object {$_.State -eq 'Enabled'} | Select-Object FeatureName | ConvertTo-Json"],
            SCAN_TIMEOUT,
        ).await?;

        let kind = self.kind();
        let features: Vec<PsFeature> = serde_json::from_str(&out.stdout).unwrap_or_default();
        Ok(features
            .into_iter()
            .map(|f| UpdateCandidate {
                id: PackageId::new(kind, &f.feature_name),
                name: f.feature_name,
                installed: Version::parse("1.0"),
                available: Version::parse("1.0"),
                size_bytes: None,
                expected_sha256: None,
            })
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        let out = proc::run(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                &format!(
                    "Enable-WindowsOptionalFeature -Online -FeatureName {} -NoRestart",
                    candidate.id.native
                ),
            ],
            INSTALL_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("Enable-WindowsOptionalFeature {}", candidate.id.native),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn installed_version(&self, _candidate: &UpdateCandidate) -> Result<Option<String>> {
        // Features don't have versions; return a static value
        Ok(Some("1.0".to_string()))
    }
}

#[derive(serde::Deserialize)]
struct PsFeature {
    feature_name: String,
}

// ── NVIDIA GeForce Experience ────────────────────────────────────────────────

pub struct NvidiaGeForceExperienceBackend;

impl NvidiaGeForceExperienceBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NvidiaGeForceExperienceBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for NvidiaGeForceExperienceBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::NvidiaGeForceExperience
    }
    fn display_name(&self) -> &str {
        "NVIDIA GeForce Experience"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        // Check for NVIDIA GeForce Experience installation
        let program_files = std::env::var("ProgramFiles").unwrap_or_default();
        std::path::Path::new(&program_files)
            .join("NVIDIA Corporation/NVIDIA GeForce Experience/NVIDIA GeForce Experience.exe")
            .exists()
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // GeForce Experience doesn't have a CLI; we report the installed
        // driver version from the registry as an informational entry.
        if !cfg!(windows) {
            return Ok(Vec::new());
        }
        let kind = self.kind();
        // Read driver version from registry
        let out = proc::run(
            "powershell",
            &["-NoProfile", "-Command",
             "Get-ItemProperty 'HKLM:\\SOFTWARE\\NVIDIA Corporation\\Global\\Driver' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Version"],
            SCAN_TIMEOUT,
        ).await?;

        let version = out.stdout.trim().to_string();
        if version.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![UpdateCandidate {
            id: PackageId::new(kind, "nvidia-driver"),
            name: "NVIDIA Display Driver".to_string(),
            installed: Version::parse(&version),
            available: Version::Unknown(String::new()),
            size_bytes: None,
            expected_sha256: None,
        }])
    }

    async fn apply(&self, _candidate: &UpdateCandidate) -> Result<()> {
        Err(Error::Verification {
            package: _candidate.id.to_string(),
            detail: "NVIDIA driver updates require GeForce Experience GUI or the NVIDIA website"
                .into(),
        })
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        if candidate.id.native != "nvidia-driver" {
            return Ok(None);
        }
        let out = proc::run(
            "powershell",
            &["-NoProfile", "-Command",
             "Get-ItemProperty 'HKLM:\\SOFTWARE\\NVIDIA Corporation\\Global\\Driver' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Version"],
            SCAN_TIMEOUT,
        ).await?;
        let v = out.stdout.trim().to_string();
        if v.is_empty() {
            Ok(None)
        } else {
            Ok(Some(v))
        }
    }
}

// ── Intel DSA ────────────────────────────────────────────────────────────────

pub struct IntelDsaBackend;

impl IntelDsaBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for IntelDsaBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for IntelDsaBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::IntelDsa
    }
    fn display_name(&self) -> &str {
        "Intel Driver & Support Assistant"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        let program_files = std::env::var("ProgramFiles").unwrap_or_default();
        std::path::Path::new(&program_files)
            .join("Intel/Driver Support Assistant/Intel.DSA.exe")
            .exists()
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // Intel DSA doesn't have a public CLI API; we report it as informational
        Ok(Vec::new())
    }

    async fn apply(&self, _candidate: &UpdateCandidate) -> Result<()> {
        Err(Error::Verification {
            package: _candidate.id.to_string(),
            detail: "Intel DSA updates require the DSA GUI".into(),
        })
    }

    async fn installed_version(&self, _candidate: &UpdateCandidate) -> Result<Option<String>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_pins_the_version_in_one_argument() {
        // Regression: passing name and version as separate args made pip try to
        // install a second package literally named "1.5.1", failing every
        // upgrade. The pin must be a single `name==version` token.
        assert_eq!(pip_install_spec("yfinance", "1.5.1"), "yfinance==1.5.1");
    }

    #[test]
    fn parses_pip_list_json() {
        let out = r#"[{"name": "requests", "version": "2.32.3"},
                      {"name": "urllib3", "version": "2.2.2"}]"#;
        let installed = parse_pip_list(out).unwrap();
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id.native, "requests");
        assert_eq!(installed[0].version, "2.32.3");
        assert_eq!(installed[1].name, "urllib3");
    }

    #[test]
    fn pip_list_reports_a_parse_error_rather_than_an_empty_inventory() {
        assert!(parse_pip_list("not json").is_err());
    }

    #[test]
    fn parses_cargo_install_list() {
        let out = "\
ripgrep v14.1.0:
    rg
cargo-edit v0.12.2 (/home/u/src/cargo-edit):
    cargo-add
    cargo-rm
";
        let installed = parse_cargo_install_list(out);
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id.native, "ripgrep");
        assert_eq!(installed[0].version, "14.1.0");
        // The path suffix is not part of the version, and the indented binary
        // names are not packages.
        assert_eq!(installed[1].id.native, "cargo-edit");
        assert_eq!(installed[1].version, "0.12.2");
    }

    #[test]
    fn cargo_install_list_empty_output_yields_empty_vec() {
        assert!(parse_cargo_install_list("").is_empty());
    }

    #[test]
    fn parses_npm_global_list_json() {
        let out = r#"{
            "name": "lib",
            "dependencies": {
                "npm": {"version": "10.5.0"},
                "typescript": {"version": "5.5.4"}
            }
        }"#;
        let mut installed = parse_npm_global_list(out);
        installed.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id.native, "npm");
        assert_eq!(installed[0].version, "10.5.0");
        assert_eq!(installed[1].version, "5.5.4");
    }

    #[test]
    fn npm_global_list_skips_entries_without_a_version() {
        let out = r#"{"dependencies": {"broken": {"missing": true}, "ok": {"version": "1.0.0"}}}"#;
        let installed = parse_npm_global_list(out);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].id.native, "ok");
    }

    #[test]
    fn npm_global_list_tolerates_non_json_output() {
        assert!(parse_npm_global_list("npm ERR! something").is_empty());
        assert!(parse_npm_global_list("{}").is_empty());
    }

    #[test]
    fn parses_dotnet_tool_list() {
        let out = "\
Package Id                 Version      Commands
-------------------------------------------------
dotnet-ef                  8.0.8        dotnet-ef
dotnet-format              5.1.250801   dotnet-format
";
        let installed = parse_dotnet_tool_list(out);
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id.native, "dotnet-ef");
        assert_eq!(installed[0].version, "8.0.8");
        assert_eq!(installed[1].version, "5.1.250801");
    }

    #[test]
    fn dotnet_tool_list_with_only_headers_yields_empty_vec() {
        let out = "Package Id      Version      Commands\n------------------------------\n";
        assert!(parse_dotnet_tool_list(out).is_empty());
    }

    #[test]
    fn parses_vscode_extensions() {
        let out = "\
ms-python.python@2024.14.0
rust-lang.rust-analyzer@0.3.2050
";
        let installed = parse_vscode_extensions(out);
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id.native, "ms-python.python");
        assert_eq!(installed[0].version, "2024.14.0");
        assert_eq!(installed[1].id.native, "rust-lang.rust-analyzer");
    }

    #[test]
    fn vscode_extension_lines_without_a_version_are_skipped() {
        assert!(parse_vscode_extensions("ms-python.python\n").is_empty());
        assert!(parse_vscode_extensions("").is_empty());
    }

    #[tokio::test]
    async fn cargo_scan_reports_no_candidates_without_a_registry_lookup() {
        // cargo cannot know a "latest" version offline; scan must be honest
        // about that instead of spawning cargo and discarding the output.
        assert!(CargoBackend::new().scan().await.unwrap().is_empty());
    }
}
