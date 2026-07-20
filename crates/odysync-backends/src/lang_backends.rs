//! Language runtime package managers: pip, cargo, npm, go, dotnet, vscode, powershell.

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::version::Version;
use odysync_core::proc;

const SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const INSTALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

// ── Pip ──────────────────────────────────────────────────────────────────────

pub struct PipBackend;

impl PipBackend {
    pub fn new() -> Self { Self }
}

impl Default for PipBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for PipBackend {
    fn kind(&self) -> BackendKind { BackendKind::Pip }
    fn display_name(&self) -> &str { "Python pip" }

    async fn is_available(&self) -> bool {
        proc::exists("pip", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("pip", &["list", "--outdated", "--format=json"], SCAN_TIMEOUT).await?;
        let packages: Vec<PipOutdated> = serde_json::from_str(&out.stdout)
            .map_err(|e| Error::parse("pip", format!("JSON parse: {e}")))?;
        let kind = self.kind();
        Ok(packages.into_iter().map(|p| {
            let name = p.name.clone();
            UpdateCandidate {
            id: PackageId::new(kind, p.name),
            name,
            installed: Version::parse(&p.version),
            available: Version::parse(&p.latest_version),
            size_bytes: None,
            expected_sha256: None,
        }}).collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }
        let out = proc::run("pip", &["install", "--upgrade", &candidate.id.native, &candidate.available.raw()], INSTALL_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("pip install --upgrade {}", candidate.id.native),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
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

// ── Cargo ────────────────────────────────────────────────────────────────────

pub struct CargoBackend;

impl CargoBackend {
    pub fn new() -> Self { Self }
}

impl Default for CargoBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for CargoBackend {
    fn kind(&self) -> BackendKind { BackendKind::Cargo }
    fn display_name(&self) -> &str { "Rust cargo" }

    async fn is_available(&self) -> bool {
        proc::exists("cargo", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("cargo", &["install", "--list"], SCAN_TIMEOUT).await?;
        let kind = self.kind();
        let candidates = Vec::new();
        for line in out.stdout.lines() {
            // cargo install --list outputs lines like:
            //   ripgrep v14.1.0:
            //     ripgrep v14.1.0
            if let Some((name, rest)) = line.split_once(' ') {
                if let Some(version) = rest.strip_prefix('v') {
                    let version = version.trim_end_matches(':').trim();
                    // Check crates.io for latest (simplified: we report installed only)
                    // A full implementation would query crates.io API
                    let _ = version;
                    let _ = name;
                }
            }
        }
        let _ = kind;
        Ok(candidates)
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }
        let out = proc::run("cargo", &["install", &candidate.id.native, "--version", &candidate.available.raw()], INSTALL_TIMEOUT).await?;
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

// ── Npm ──────────────────────────────────────────────────────────────────────

pub struct NpmBackend;

impl NpmBackend {
    pub fn new() -> Self { Self }
}

impl Default for NpmBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for NpmBackend {
    fn kind(&self) -> BackendKind { BackendKind::Npm }
    fn display_name(&self) -> &str { "Node.js npm (global)" }

    async fn is_available(&self) -> bool {
        proc::exists("npm", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("npm", &["outdated", "-g", "--json"], SCAN_TIMEOUT).await?;
        // npm outdated exits non-zero when updates are available
        let packages: std::collections::HashMap<String, NpmOutdated> =
            serde_json::from_str(&out.stdout).unwrap_or_default();
        let kind = self.kind();
        Ok(packages.into_iter().map(|(name, p)| {
            let id_name = name.clone();
            UpdateCandidate {
            id: PackageId::new(kind, id_name),
            name,
            installed: Version::parse(&p.current.unwrap_or_default()),
            available: Version::parse(&p.latest.unwrap_or_default()),
            size_bytes: None,
            expected_sha256: None,
        }}).collect())
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

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = proc::run("npm", &["list", "-g", &candidate.id.native, "--json"], SCAN_TIMEOUT).await?;
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
    wanted: Option<String>,
    latest: Option<String>,
}

// ── Go ───────────────────────────────────────────────────────────────────────

pub struct GoBackend;

impl GoBackend {
    pub fn new() -> Self { Self }
}

impl Default for GoBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for GoBackend {
    fn kind(&self) -> BackendKind { BackendKind::Go }
    fn display_name(&self) -> &str { "Go modules" }

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
        let out = proc::run("go", &["get", "-u", &candidate.id.native], INSTALL_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("go get -u {}", candidate.id.native),
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
    pub fn new() -> Self { Self }
}

impl Default for DotnetToolBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for DotnetToolBackend {
    fn kind(&self) -> BackendKind { BackendKind::DotnetTool }
    fn display_name(&self) -> &str { ".NET global tools" }

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
        let out = proc::run("dotnet", &["tool", "update", "-g", &candidate.id.native, "--version", &candidate.available.raw()], INSTALL_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("dotnet tool update -g {}", candidate.id.native),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
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

// ── VS Code Extensions ───────────────────────────────────────────────────────

pub struct VscodeExtensionBackend;

impl VscodeExtensionBackend {
    pub fn new() -> Self { Self }
}

impl Default for VscodeExtensionBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for VscodeExtensionBackend {
    fn kind(&self) -> BackendKind { BackendKind::VscodeExtension }
    fn display_name(&self) -> &str { "VS Code extensions" }

    async fn is_available(&self) -> bool {
        // Check for either `code` or `code-insiders`
        proc::exists("code", &["--version"]).await || proc::exists("code-insiders", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let cmd = if proc::exists("code", &["--version"]).await { "code" } else { "code-insiders" };
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
        let cmd = if proc::exists("code", &["--version"]).await { "code" } else { "code-insiders" };
        let out = proc::run(cmd, &["--install-extension", &candidate.id.native, "--force"], INSTALL_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: format!("{} --install-extension {}", cmd, candidate.id.native),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let cmd = if proc::exists("code", &["--version"]).await { "code" } else { "code-insiders" };
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

// ── PowerShell Modules ───────────────────────────────────────────────────────

pub struct PowerShellModuleBackend;

impl PowerShellModuleBackend {
    pub fn new() -> Self { Self }
}

impl Default for PowerShellModuleBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for PowerShellModuleBackend {
    fn kind(&self) -> BackendKind { BackendKind::PowerShellModule }
    fn display_name(&self) -> &str { "PowerShell modules" }

    async fn is_available(&self) -> bool {
        cfg!(windows) && proc::exists("pwsh", &["--version"]).await
            || (cfg!(windows) && proc::exists("powershell", &["-Command", "$PSVersionTable.PSVersion"]).await)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let pwsh = if proc::exists("pwsh", &["--version"]).await { "pwsh" } else { "powershell" };
        let script = "Get-InstalledModule | Select-Object Name, Version | ConvertTo-Json";
        let out = proc::run(pwsh, &["-NoProfile", "-Command", script], SCAN_TIMEOUT).await?;
        let kind = self.kind();
        let modules: Vec<PsModule> = serde_json::from_str(&out.stdout).unwrap_or_default();
        Ok(modules.into_iter().map(|m| {
            let name = m.name.clone();
            UpdateCandidate {
            id: PackageId::new(kind, m.name),
            name,
            installed: Version::parse(&m.version),
            available: Version::Unknown(String::new()),
            size_bytes: None,
            expected_sha256: None,
        }}).collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        let pwsh = if proc::exists("pwsh", &["--version"]).await { "pwsh" } else { "powershell" };
        let out = proc::run(pwsh, &["-NoProfile", "-Command", &format!("Update-Module -Name {} -Force", candidate.id.native)], INSTALL_TIMEOUT).await?;
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
        let pwsh = if proc::exists("pwsh", &["--version"]).await { "pwsh" } else { "powershell" };
        let script = format!("Get-InstalledModule -Name {} | Select-Object -ExpandProperty Version", candidate.id.native);
        let out = proc::run(pwsh, &["-NoProfile", "-Command", &script], SCAN_TIMEOUT).await?;
        let v = out.stdout.trim().to_string();
        if v.is_empty() { Ok(None) } else { Ok(Some(v)) }
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
    pub fn new() -> Self { Self }
}

impl Default for JetbrainsPluginBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for JetbrainsPluginBackend {
    fn kind(&self) -> BackendKind { BackendKind::JetbrainsPlugin }
    fn display_name(&self) -> &str { "JetBrains IDE plugins" }

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
                        let plugin_name = plugin_path.file_name()
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
    pub fn new() -> Self { Self }
}

impl Default for WindowsOptionalFeatureBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for WindowsOptionalFeatureBackend {
    fn kind(&self) -> BackendKind { BackendKind::WindowsOptionalFeature }
    fn display_name(&self) -> &str { "Windows optional features" }

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
        Ok(features.into_iter().map(|f| UpdateCandidate {
            id: PackageId::new(kind, &f.feature_name),
            name: f.feature_name,
            installed: Version::parse("1.0"),
            available: Version::parse("1.0"),
            size_bytes: None,
            expected_sha256: None,
        }).collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        let out = proc::run(
            "powershell",
            &["-NoProfile", "-Command", &format!("Enable-WindowsOptionalFeature -Online -FeatureName {} -NoRestart", candidate.id.native)],
            INSTALL_TIMEOUT,
        ).await?;
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
    pub fn new() -> Self { Self }
}

impl Default for NvidiaGeForceExperienceBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for NvidiaGeForceExperienceBackend {
    fn kind(&self) -> BackendKind { BackendKind::NvidiaGeForceExperience }
    fn display_name(&self) -> &str { "NVIDIA GeForce Experience" }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) { return false; }
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
            detail: "NVIDIA driver updates require GeForce Experience GUI or the NVIDIA website".into(),
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
        if v.is_empty() { Ok(None) } else { Ok(Some(v)) }
    }
}

// ── Intel DSA ────────────────────────────────────────────────────────────────

pub struct IntelDsaBackend;

impl IntelDsaBackend {
    pub fn new() -> Self { Self }
}

impl Default for IntelDsaBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for IntelDsaBackend {
    fn kind(&self) -> BackendKind { BackendKind::IntelDsa }
    fn display_name(&self) -> &str { "Intel Driver & Support Assistant" }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) { return false; }
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
