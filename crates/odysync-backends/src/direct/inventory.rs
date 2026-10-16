//! The OS's own list of installed programs.
//!
//! On Windows that is the `Uninstall` key in three places: the 64-bit and
//! 32-bit views of `HKEY_LOCAL_MACHINE` (machine-wide installs) and
//! `HKEY_CURRENT_USER` (per-user installs). It is the same list Settings →
//! Apps shows, read directly — no winget, no WMI (`Win32_Product` is famously
//! slow and triggers an MSI consistency check on every package it touches).

use std::path::{Path, PathBuf};

use super::catalog::InstalledApp;
#[cfg(windows)]
use super::catalog::{Scope, Technology};

/// The CPU architecture of the machine, as the catalog spells it.
///
/// Not `std::env::consts::ARCH`, which is the architecture this *binary* was
/// built for: an x64 build running under emulation on an ARM64 machine would
/// otherwise conclude the machine is x64.
#[cfg(windows)]
pub fn machine_arch() -> &'static str {
    // A 32-bit process on 64-bit Windows sees its own arch in
    // PROCESSOR_ARCHITECTURE and the machine's in PROCESSOR_ARCHITEW6432.
    let raw = std::env::var("PROCESSOR_ARCHITEW6432")
        .or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE"))
        .unwrap_or_default();
    arch_from_processor_architecture(&raw)
}

/// Map Windows' `PROCESSOR_ARCHITECTURE` spelling onto the catalog's.
pub fn arch_from_processor_architecture(raw: &str) -> &'static str {
    match raw.trim().to_ascii_uppercase().as_str() {
        "ARM64" => "arm64",
        "X86" => "x86",
        // AMD64, and anything unrecognised: x64 is by far the common case,
        // and an asset that does not exist for it is simply not offered.
        _ => "x64",
    }
}

/// The directory a program lives in, from whatever the uninstall entry offers.
///
/// `InstallLocation` is optional and frequently empty — Notepad++ leaves it
/// blank — so the folder of `DisplayIcon` or of the uninstaller is the next
/// best answer. Both can carry quotes and a `,0` icon index.
pub fn install_dir_from(
    install_location: Option<&str>,
    display_icon: Option<&str>,
    uninstall_string: Option<&str>,
) -> Option<PathBuf> {
    if let Some(loc) = install_location.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(loc.trim_matches('"')));
    }
    for raw in [display_icon, uninstall_string].into_iter().flatten() {
        if let Some(exe) = executable_path(raw)
            && let Some(parent) = Path::new(&exe).parent()
            && parent.components().count() > 1
        {
            return Some(parent.to_path_buf());
        }
    }
    None
}

/// The executable path at the start of a command line or icon reference.
fn executable_path(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('"') {
        return rest.split('"').next().map(str::to_string);
    }
    // Unquoted: up to ".exe", then drop an icon index such as ",0".
    let lower = raw.to_ascii_lowercase();
    let end = lower.find(".exe").map(|i| i + 4)?;
    // `MsiExec.exe /X{GUID}` names the Windows Installer, not the program.
    let path = &raw[..end];
    if path.to_ascii_lowercase().ends_with("msiexec.exe") {
        return None;
    }
    Some(path.to_string())
}

#[cfg(windows)]
pub fn installed_apps() -> Vec<InstalledApp> {
    use winreg::RegKey;
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };

    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let native = machine_arch();

    let views: [(RegKey, u32, Scope, &'static str); 3] = [
        (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            KEY_WOW64_64KEY,
            Scope::Machine,
            native,
        ),
        (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            KEY_WOW64_32KEY,
            Scope::Machine,
            "x86",
        ),
        // Per-user installs carry no architecture in their location; the
        // machine's own is the right default, and the catalog only offers an
        // asset that exists for it.
        (RegKey::predef(HKEY_CURRENT_USER), 0, Scope::User, native),
    ];

    let mut out: Vec<InstalledApp> = Vec::new();
    for (hive, view, scope, arch) in views {
        let Ok(root) = hive.open_subkey_with_flags(UNINSTALL, KEY_READ | view) else {
            continue;
        };
        for key_name in root.enum_keys().flatten() {
            let Ok(key) = root.open_subkey_with_flags(&key_name, KEY_READ | view) else {
                continue;
            };
            let s = |name: &str| -> Option<String> {
                key.get_value::<String, _>(name)
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            };
            let d = |name: &str| -> Option<u32> { key.get_value::<u32, _>(name).ok() };

            let Some(display_name) = s("DisplayName") else {
                continue;
            };
            // Components, patches and updates hang off a parent product and
            // are not separately updatable programs.
            if d("SystemComponent") == Some(1)
                || s("ParentKeyName").is_some()
                || matches!(
                    s("ReleaseType").as_deref(),
                    Some("Update" | "Hotfix" | "Security Update")
                )
            {
                continue;
            }

            let technology = if d("WindowsInstaller") == Some(1) {
                Technology::Msi
            } else if key_name.to_ascii_lowercase().ends_with("_is1") {
                Technology::Inno
            } else {
                Technology::Other
            };

            let install_dir = install_dir_from(
                s("InstallLocation").as_deref(),
                s("DisplayIcon").as_deref(),
                s("UninstallString").as_deref(),
            );

            out.push(InstalledApp {
                display_name,
                display_version: s("DisplayVersion"),
                publisher: s("Publisher"),
                scope,
                arch,
                technology,
                install_dir,
            });
        }
    }

    // On 32-bit Windows both HKLM views are the same key; drop the echo.
    out.sort_by(|a, b| {
        (a.display_name.as_str(), a.scope.as_str())
            .cmp(&(b.display_name.as_str(), b.scope.as_str()))
    });
    out.dedup_by(|a, b| {
        a.display_name == b.display_name
            && a.scope == b.scope
            && a.display_version == b.display_version
            && a.install_dir == b.install_dir
    });
    out
}

/// No inventory source is implemented for this platform yet; its own package
/// managers (apt, dnf, Homebrew…) already cover installed software there.
#[cfg(not(windows))]
pub fn installed_apps() -> Vec<InstalledApp> {
    Vec::new()
}

/// Full paths of every running executable this user can see.
///
/// Used to refuse updating a program that is open: an installer that cannot
/// replace a locked file either fails half-way or, worse, schedules the
/// replacement for the next reboot and reports success now.
#[cfg(windows)]
pub async fn running_executables() -> Vec<PathBuf> {
    let script = "Get-Process | ForEach-Object { $_.Path } | Where-Object { $_ }";
    match odysync_core::proc::powershell(script, std::time::Duration::from_secs(60)).await {
        Ok(out) => out
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "could not list running processes");
            Vec::new()
        }
    }
}

/// The first running executable inside `dir`, if any.
pub fn running_inside<'a>(dir: &Path, running: &'a [PathBuf]) -> Option<&'a PathBuf> {
    running
        .iter()
        .find(|exe| odysync_core::cleanup::is_within(dir, exe) && exe.as_path() != dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processor_architecture_is_mapped_to_catalog_names() {
        assert_eq!(arch_from_processor_architecture("AMD64"), "x64");
        assert_eq!(arch_from_processor_architecture("ARM64"), "arm64");
        assert_eq!(arch_from_processor_architecture("x86"), "x86");
        assert_eq!(arch_from_processor_architecture(""), "x64");
    }

    #[cfg(windows)]
    #[test]
    fn the_install_dir_falls_back_to_the_icon_then_the_uninstaller() {
        assert_eq!(
            install_dir_from(Some(r"C:\Program Files\Git\"), None, None),
            Some(PathBuf::from(r"C:\Program Files\Git\"))
        );
        // Notepad++ leaves InstallLocation empty.
        assert_eq!(
            install_dir_from(
                Some(""),
                Some(r"C:\Program Files\Notepad++\notepad++.exe,0"),
                None
            ),
            Some(PathBuf::from(r"C:\Program Files\Notepad++"))
        );
        assert_eq!(
            install_dir_from(
                None,
                None,
                Some(r#""C:\Program Files\Notepad++\uninstall.exe" /S"#)
            ),
            Some(PathBuf::from(r"C:\Program Files\Notepad++"))
        );
        // An MSI's uninstall string names msiexec, which is not the program.
        assert_eq!(
            install_dir_from(
                None,
                None,
                Some("MsiExec.exe /X{B268D2DF-B30F-4883-8866-22B3CFCC2387}")
            ),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_running_program_inside_the_install_dir_is_found() {
        let running = vec![
            PathBuf::from(r"C:\Windows\explorer.exe"),
            PathBuf::from(r"C:\Program Files\Notepad++\notepad++.exe"),
        ];
        assert_eq!(
            running_inside(Path::new(r"C:\Program Files\Notepad++"), &running),
            Some(&running[1])
        );
        assert_eq!(
            running_inside(Path::new(r"C:\Program Files\Git"), &running),
            None
        );
        // A sibling directory sharing a prefix is not inside.
        assert_eq!(
            running_inside(Path::new(r"C:\Program Files\Notepad"), &running),
            None
        );
    }
}
