//! One install run at a time, across every Odysync on the machine.
//!
//! The app, a terminal and the scheduler are separate processes. Two of them
//! installing at once would race over the same packages and the same history
//! file, so whoever starts first holds a lock file until it is done. The lock
//! is the operating system's, not the file's existence: a process that dies
//! releases it.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Held for as long as updates are being installed.
#[derive(Debug)]
pub struct ApplyLock {
    _file: File,
}

impl ApplyLock {
    /// Take the lock, or say that someone else has it.
    pub fn acquire() -> Result<Self> {
        Self::acquire_at(&default_path()?)
    }

    /// [`acquire`](Self::acquire) on a given file.
    pub fn acquire_at(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut create = File::options();
        create.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Readable for everyone, and never through a link somebody else
            // put there: the directory is shared.
            create.mode(0o644).custom_flags(libc::O_NOFOLLOW);
        }
        let file = match create.open(path) {
            Ok(file) => file,
            // Somebody else's, from a run as root or as another user. A lock
            // needs no more than reading.
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => File::open(path)?,
            Err(e) => return Err(e.into()),
        };
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(Error::unavailable(
                "Odysync",
                "another Odysync is installing updates right now; wait for it to finish",
            )),
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }
}

/// One place for every user of the machine: an elevated run and an ordinary
/// one can belong to different accounts.
fn default_path() -> Result<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .map(|p| p.join("Odysync").join("apply.lock"))
            .ok_or_else(|| Error::Config("ProgramData is not set".into()))
    } else {
        Ok(PathBuf::from("/tmp/odysync-apply.lock"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_run_holds_the_lock_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("apply.lock");
        let first = ApplyLock::acquire_at(&path).expect("free");
        let err = ApplyLock::acquire_at(&path).expect_err("taken");
        assert!(err.to_string().contains("another Odysync"), "{err}");
        drop(first);
        ApplyLock::acquire_at(&path).expect("free again");
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_link_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("victim");
        let link = dir.path().join("apply.lock");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(ApplyLock::acquire_at(&link).is_err());
        assert!(!target.exists());
    }
}
