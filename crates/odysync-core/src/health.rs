//! Is there room for this update?
//!
//! An installer that runs out of disk space in the middle leaves a program
//! that is neither the old version nor the new one, and few of them clean up
//! after themselves. Asking beforehand costs nothing.

use std::path::{Path, PathBuf};

/// What must stay free on the system drive whatever is installed.
const RESERVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// How much an installer needs for itself, in multiples of its download: the
/// file, its unpacked contents, and the program it writes.
const SPACE_PER_DOWNLOADED_BYTE: u64 = 3;

/// Why there is not enough room for a download of `size` bytes, if so.
///
/// `None` as well when the free space cannot be read, or says nothing (a
/// read-only system image reports none at all): that is no reason to keep
/// updates away from a machine.
pub fn not_enough_space(size: Option<u64>) -> Option<String> {
    let drive = system_drive();
    shortage(free_space(&drive)?, size).map(|s| format!("{s} on {}", drive.display()))
}

fn shortage(free: u64, size: Option<u64>) -> Option<String> {
    let needed =
        RESERVE_BYTES.saturating_add(size.unwrap_or(0).saturating_mul(SPACE_PER_DOWNLOADED_BYTE));
    let gb = |bytes: u64| bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    (free < needed).then(|| {
        format!(
            "only {:.1} GB are free and {:.1} GB are needed",
            gb(free),
            gb(needed)
        )
    })
}

fn system_drive() -> PathBuf {
    if cfg!(windows) {
        // `SystemDrive` is `C:`, which alone means "the current directory on
        // C:"; with the separator it is the root.
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        PathBuf::from(format!("{drive}\\"))
    } else {
        // Where packages are downloaded and unpacked. `/` itself is a
        // read-only image on macOS and on immutable Linux systems.
        PathBuf::from("/var")
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn free_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut buf = unsafe { std::mem::zeroed::<libc::statvfs>() };
        let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut buf) };
        let says_nothing = buf.f_blocks == 0 || (buf.f_flag & libc::ST_RDONLY) != 0;
        if ret == 0 && !says_nothing {
            Some(buf.f_bavail as u64 * buf.f_frsize as u64)
        } else {
            None
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe extern "system" {
            fn GetDiskFreeSpaceExW(
                directory: *const u16,
                free_bytes_available: *mut u64,
                total_bytes: *mut u64,
                total_free_bytes: *mut u64,
            ) -> i32;
        }

        let mut free: u64 = 0;
        let mut total: u64 = 0;
        let mut total_free: u64 = 0;
        let ret =
            unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) };
        if ret != 0 { Some(free) } else { None }
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn free_space(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn a_small_update_needs_the_reserve() {
        assert_eq!(shortage(2 * GB, None), None);
        assert!(shortage(2 * GB - 1, None).is_some());
    }

    #[test]
    fn a_large_download_needs_room_to_unpack() {
        assert!(shortage(4 * GB, Some(GB)).is_some(), "2 + 3 GB needed");
        assert_eq!(shortage(5 * GB, Some(GB)), None);
        let text = shortage(GB, Some(GB)).expect("too little");
        assert!(text.contains("1.0 GB") && text.contains("5.0 GB"), "{text}");
    }

    #[test]
    fn an_absurd_size_does_not_overflow() {
        assert!(shortage(u64::MAX - 1, Some(u64::MAX)).is_some());
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[test]
    fn the_free_space_of_the_system_drive_can_be_read() {
        assert!(free_space(&system_drive()).is_some_and(|b| b > 0));
    }
}
