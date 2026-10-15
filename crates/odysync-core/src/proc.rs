//! Process execution that never flashes a console window.
//!
//! The old version spawned `winget` and PowerShell through the shell, so every
//! scan and every install popped a terminal on screen — unusable for a tool
//! meant to run in the background. Two things fix that:
//!
//!   * `CREATE_NO_WINDOW` on Windows, so no console is allocated for the child
//!   * never routing through `cmd.exe`/`sh`; arguments are passed as a vector,
//!     which also removes the shell-injection surface entirely
//!
//! Every call is time-boxed. A package manager that hangs on a network stall
//! must not wedge a background service forever.

use std::ffi::OsStr;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::{Error, Result};

/// Maximum output size per stream (stdout/stderr): 10 MiB.
///
/// Prevents memory exhaustion from a misbehaving package manager that
/// writes unbounded output (e.g. verbose debug logging to stderr).
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Windows `CREATE_NO_WINDOW`: run without allocating a console.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// What a finished process produced.
#[derive(Debug, Clone)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == 0
    }

    /// Return stdout, or a [`Error::CommandFailed`] when the exit code was
    /// non-zero.
    pub fn ok_stdout(self, command: &str) -> Result<String> {
        if self.success() {
            Ok(self.stdout)
        } else {
            Err(Error::CommandFailed {
                command: command.to_string(),
                code: self.code,
                // Some tools report errors on stdout; include both so the user
                // sees the actual message.
                stderr: if self.stderr.trim().is_empty() {
                    self.stdout
                } else {
                    self.stderr
                },
            })
        }
    }
}

/// Resolve a bare program name against `PATH` and `PATHEXT` on Windows.
///
/// `CreateProcess` searches `PATH` but only ever appends `.exe`, so
/// `Command::new("npm")` fails on a machine where npm exists solely as
/// `npm.cmd` — which is every Windows install of Node. The npm and VS Code
/// backends therefore reported themselves "not available" on hosts that
/// plainly had them, and no error was ever surfaced because an unavailable
/// backend is simply skipped.
///
/// Doing the `PATHEXT` walk here rather than routing through `cmd.exe` keeps
/// the module's promise: arguments stay a vector, so there is still no shell
/// and no injection surface.
#[cfg(windows)]
fn resolve_program(program: &str) -> std::path::PathBuf {
    use std::path::{Path, PathBuf};

    let raw = Path::new(program);
    // An explicit path, or a name that already carries its extension, is
    // something `CreateProcess` resolves correctly on its own.
    if raw.components().count() > 1 || raw.extension().is_some() {
        return raw.to_path_buf();
    }

    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());

    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for ext in pathext.split(';').filter(|e| !e.is_empty()) {
                let candidate = dir.join(format!("{program}{ext}"));
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }

    // Nothing matched. Hand the bare name over anyway so the caller still gets
    // the familiar "not found on PATH" error from the spawn.
    PathBuf::from(program)
}

#[cfg(not(windows))]
fn resolve_program(program: &str) -> &str {
    program
}

/// Build a `Command` configured for silent, non-interactive background use.
fn build<S: AsRef<OsStr>>(program: &str, args: &[S]) -> Command {
    let mut cmd = Command::new(resolve_program(program));
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
        // Windows PowerShell 5.1 must not inherit a PowerShell 7 module path.
        // Started from `pwsh`, Odysync's environment carries PSModulePath
        // entries for PowerShell 7's own modules; 5.1 then loads those first,
        // they fail to load, and every built-in cmdlet that lives in a module —
        // `Get-AuthenticodeSignature` among them — reports that its module
        // "could not be loaded". Unset, 5.1 computes its own default path.
        if is_windows_powershell(program) {
            cmd.env_remove("PSModulePath");
        }
    }

    // Force machine-readable, untranslated output. Parsing localised package
    // manager text was a recurring source of bugs in the previous version.
    cmd.env("LC_ALL", "C").env("LANG", "C");

    cmd
}

/// Whether `program` names Windows PowerShell (5.1), however it is spelled.
#[cfg(windows)]
fn is_windows_powershell(program: &str) -> bool {
    let name = std::path::Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name == "powershell.exe" || name == "powershell"
}

/// Run `program` with `args`, capturing output, killed after `timeout`.
///
/// The child is killed on timeout; `kill_on_drop` ensures it also dies if the
/// future is cancelled, so a cancelled scan cannot leave orphaned processes.
pub async fn run<S: AsRef<OsStr>>(program: &str, args: &[S], timeout: Duration) -> Result<Output> {
    let mut cmd = build(program, args);
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::unavailable(program, format!("{program} was not found on PATH"))
        } else {
            Error::Io(e)
        }
    })?;

    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    // Drain both pipes concurrently with the wait. Reading them sequentially
    // deadlocks as soon as a child fills the pipe buffer it is not being read
    // from, which winget does on large upgrade lists.
    let collect = async {
        let stdout_fut = async {
            let mut stdout = String::new();
            if let Some(p) = stdout_pipe.as_mut() {
                let mut raw = Vec::new();
                p.read_to_end(&mut raw).await?;
                truncate_output(&mut raw, program);
                stdout = String::from_utf8_lossy(&raw).into_owned();
            }
            Ok::<_, std::io::Error>(stdout)
        };
        let stderr_fut = async {
            let mut stderr = String::new();
            if let Some(p) = stderr_pipe.as_mut() {
                let mut raw = Vec::new();
                p.read_to_end(&mut raw).await?;
                truncate_output(&mut raw, program);
                stderr = String::from_utf8_lossy(&raw).into_owned();
            }
            Ok::<_, std::io::Error>(stderr)
        };
        let (stdout, stderr) = tokio::try_join!(stdout_fut, stderr_fut)?;
        Ok::<_, std::io::Error>((stdout, stderr))
    };

    let status = tokio::time::timeout(timeout, async {
        let (out, err) = collect.await?;
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((status, out, err))
    })
    .await;

    match status {
        Ok(Ok((status, stdout, stderr))) => Ok(Output {
            code: status.code().unwrap_or(-1),
            stdout,
            stderr,
        }),
        Ok(Err(e)) => Err(Error::Io(e)),
        Err(_) => Err(Error::CommandTimeout {
            command: program.to_string(),
            seconds: timeout.as_secs(),
        }),
    }
}

/// True when `program` can be executed at all.
pub async fn exists(program: &str, probe_args: &[&str]) -> bool {
    matches!(
        run(program, probe_args, Duration::from_secs(15)).await,
        Ok(o) if o.success()
    )
}

/// Run a PowerShell snippet without a window, without loading the user profile.
///
/// `-NonInteractive` matters: without it a script that hits an unexpected
/// prompt blocks forever behind an invisible console.
#[cfg(windows)]
pub async fn powershell(script: &str, timeout: Duration) -> Result<Output> {
    // Windows PowerShell writes redirected output in the console's OEM code
    // page, so a device called "Unbekanntes USB-Gerät" arrived as
    // "Unbekanntes USB-Ger\u{fffd}t" once decoded as UTF-8. Asking for UTF-8
    // first makes every value — names, paths, publishers — survive intact.
    let script = format!("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n{script}");
    run(
        "powershell.exe",
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-OutputFormat",
            "Text",
            "-Command",
            &script,
        ],
        timeout,
    )
    .await
}

/// Truncate output to `MAX_OUTPUT_BYTES`, logging a warning if truncation occurred.
fn truncate_output(raw: &mut Vec<u8>, program: &str) {
    if raw.len() > MAX_OUTPUT_BYTES {
        tracing::warn!(
            program = program,
            original_len = raw.len(),
            max = MAX_OUTPUT_BYTES,
            "output truncated to prevent memory exhaustion"
        );
        raw.truncate(MAX_OUTPUT_BYTES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_cmd() -> (&'static str, Vec<&'static str>) {
        if cfg!(windows) {
            ("cmd", vec!["/c", "echo", "hello"])
        } else {
            ("echo", vec!["hello"])
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_bare_name_resolves_through_pathext() {
        // `cmd` lives in System32 as `cmd.exe` and is always on PATH, so this
        // asserts the lookup finds a real file and appends an extension.
        let resolved = resolve_program("cmd");
        assert!(resolved.is_file(), "did not resolve cmd: {resolved:?}");
        assert!(resolved.extension().is_some());
    }

    #[cfg(windows)]
    #[test]
    fn an_explicit_extension_or_path_is_left_alone() {
        assert_eq!(
            resolve_program("powershell.exe"),
            std::path::Path::new("powershell.exe")
        );
        assert_eq!(
            resolve_program(r"C:\Windows\System32\cmd.exe"),
            std::path::Path::new(r"C:\Windows\System32\cmd.exe")
        );
    }

    #[cfg(windows)]
    #[test]
    fn an_unresolvable_name_is_passed_through_unchanged() {
        assert_eq!(
            resolve_program("odysync-definitely-not-a-real-program"),
            std::path::Path::new("odysync-definitely-not-a-real-program")
        );
    }

    #[tokio::test]
    async fn captures_stdout_and_exit_code() {
        let (prog, args) = echo_cmd();
        let out = run(prog, &args, Duration::from_secs(10)).await.unwrap();
        assert!(out.success());
        assert!(out.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn reports_a_missing_program_as_unavailable() {
        let err = run(
            "odysync-definitely-not-a-real-program",
            &["--version"],
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::BackendUnavailable { .. }));
    }

    #[tokio::test]
    async fn non_zero_exit_becomes_command_failed() {
        let (prog, args) = if cfg!(windows) {
            ("cmd", vec!["/c", "exit", "3"])
        } else {
            ("sh", vec!["-c", "exit 3"])
        };
        let out = run(prog, &args, Duration::from_secs(10)).await.unwrap();
        assert_eq!(out.code, 3);
        assert!(matches!(
            out.ok_stdout("test").unwrap_err(),
            Error::CommandFailed { code: 3, .. }
        ));
    }

    #[tokio::test]
    async fn a_hanging_process_is_killed_at_the_timeout() {
        let (prog, args) = if cfg!(windows) {
            // timeout.exe needs a console; ping a nonexistent loopback delay instead.
            ("cmd", vec!["/c", "ping", "-n", "30", "127.0.0.1"])
        } else {
            ("sleep", vec!["30"])
        };
        let err = run(prog, &args, Duration::from_millis(500))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::CommandTimeout { .. }));
    }

    #[tokio::test]
    async fn exists_distinguishes_real_and_fake_programs() {
        let (prog, args) = if cfg!(windows) {
            ("cmd", vec!["/c", "echo", "x"])
        } else {
            ("echo", vec!["x"])
        };
        assert!(exists(prog, &args).await);
        assert!(!exists("odysync-not-real-at-all", &["--version"]).await);
    }

    #[test]
    fn truncate_output_cuts_at_max() {
        let mut raw = vec![b'a'; MAX_OUTPUT_BYTES + 100];
        truncate_output(&mut raw, "test");
        assert_eq!(raw.len(), MAX_OUTPUT_BYTES);
    }

    #[test]
    fn truncate_output_leaves_small_output_alone() {
        let mut raw = vec![b'a'; 100];
        truncate_output(&mut raw, "test");
        assert_eq!(raw.len(), 100);
    }
}
