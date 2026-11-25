//! A download directory only administrators can write to.
//!
//! An installer that runs with administrator rights must not run from a
//! directory an ordinary process can write to; see
//! [`super::http::DownloadDir`]. The directory is created with its access list
//! in the same call, so there is no moment in which it is open, and under a
//! name nobody can guess, so it cannot be put in place beforehand: creating a
//! directory that already exists fails.

use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::CreateDirectoryW;
use windows::core::{HSTRING, PCWSTR};

use odysync_core::error::{Error, Result};

const PREFIX: &str = "Odysync-download-";

/// Protected (nothing inherited): full control for Administrators and SYSTEM,
/// nothing for anyone else.
const ADMINS_ONLY: &str = "D:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)";

/// Create a fresh directory for `label` under ProgramData.
pub fn create(label: &str) -> Result<PathBuf> {
    let root = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| Error::Config("ProgramData is not set".into()))?;
    super::http::remove_leftovers(&root, PREFIX);
    create_in(&root, label)
}

fn create_in(root: &Path, label: &str) -> Result<PathBuf> {
    let label: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))
        .take(40)
        .collect();
    // `RandomState` is seeded by the operating system for every instance.
    let random = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    let dir = root.join(format!("{PREFIX}{label}-{random:016x}"));
    create_protected(&dir)?;
    Ok(dir)
}

fn create_protected(dir: &Path) -> Result<()> {
    let sddl = HSTRING::from(ADMINS_ONLY);
    let path = HSTRING::from(dir.as_os_str());
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: every pointer is to a live local; the descriptor Windows
    // allocates is released below.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(std::io::Error::from)?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let created = CreateDirectoryW(PCWSTR(path.as_ptr()), Some(&attributes));
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        created.map_err(std::io::Error::from)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The access list as Windows reports it, e.g. `D:PAI(A;OICI;FA;;;BA)...`.
    fn access_list(dir: &Path) -> String {
        let out = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "(New-Object System.Security.AccessControl.DirectorySecurity \
                     '{}', 'Access').GetSecurityDescriptorSddlForm('Access')",
                    dir.display()
                ),
            ])
            .env_remove("PSModulePath")
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn only_administrators_can_write_to_it() {
        let root = tempfile::tempdir().unwrap();
        let dir = create_in(root.path(), "git-2.56.1").expect("create");
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("Odysync-download-git-2.56.1-"), "{name}");

        if odysync_core::platform::is_elevated() {
            // CI runs elevated: there the access list itself can be read.
            let acl = access_list(&dir);
            assert!(acl.starts_with("D:P"), "nothing is inherited: {acl}");
            assert_eq!(acl.matches("(A;").count(), 2, "{acl}");
            assert!(acl.contains(";;;BA)") && acl.contains(";;;SY)"), "{acl}");
        } else {
            // This is the attack: an ordinary process of the same user puts a
            // library next to the installer.
            let err = std::fs::write(dir.join("version.dll"), b"evil").expect_err("refused");
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        }
    }

    #[test]
    fn a_directory_that_is_already_there_is_not_used() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("planted");
        std::fs::create_dir(&dir).unwrap();
        assert!(create_protected(&dir).is_err());
    }

    #[test]
    fn names_are_not_guessable_and_stay_inside_the_root() {
        let root = tempfile::tempdir().unwrap();
        let a = create_in(root.path(), r"..\..\evil").expect("create");
        let b = create_in(root.path(), r"..\..\evil").expect("create");
        assert_ne!(a, b);
        assert_eq!(a.parent(), Some(root.path()));
    }
}
