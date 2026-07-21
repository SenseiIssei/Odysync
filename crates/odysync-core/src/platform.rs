//! Small platform probes shared by the CLI and the backends.

/// Whether this process holds administrator (Windows) or root (Unix) rights.
///
/// Used to decide which backends may run, and — just as importantly — to refuse
/// Microsoft Store updates while elevated, which silently corrupts their
/// install state.
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    // SAFETY: GetCurrentProcess returns a pseudo-handle that is always valid.
    let mut token: HANDLE = Default::default();
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok.is_err() {
        return false;
    }

    // SAFETY: token is a valid handle from OpenProcessToken. The buffer is a
    // TOKEN_ELEVATION struct, which is what TokenElevation expects.
    let mut elevation = TOKEN_ELEVATION::default();
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut 0u32,
        )
    };

    // SAFETY: token is a valid handle; CloseHandle on a pseudo-handle is a no-op
    // but we call it for correctness.
    unsafe { _ = windows::Win32::Foundation::CloseHandle(token) };

    ok.is_ok() && elevation.TokenIsElevated != 0
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
