//! Small platform probes shared by the CLI and the backends.

/// Whether this process holds administrator (Windows) or root (Unix) rights.
///
/// Used to decide which backends may run, and — just as importantly — to refuse
/// Microsoft Store updates while elevated, which silently corrupts their
/// install state.
#[cfg(windows)]
pub fn is_elevated() -> bool {
    // Probing the token directly would need the windows crate; `net session`
    // fails for non-elevated callers and needs no extra dependency. It is a
    // cheap, well-defined check that has behaved consistently since XP.
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    Command::new("net")
        .args(["session"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    // SAFETY: geteuid is always safe to call and cannot fail.
    unsafe { libc_geteuid() == 0 }
}

#[cfg(not(windows))]
extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

/// A short label for the current OS, used in reports and the UI.
pub fn os_label() -> &'static str {
    if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    }
}
