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

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::error::{Error, Result};

/// Maximum output size per stream (stdout/stderr): 10 MiB.
///
/// Prevents memory exhaustion from a misbehaving package manager that
/// writes unbounded output (e.g. verbose debug logging to stderr).
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// How long to keep reading once the program itself has exited.
///
/// A pipe only ends when *every* holder of its write end is gone, and a
/// program can leave a child behind that inherited it: an installer that
/// starts the freshly installed app, a package manager that starts a daemon.
/// Waiting for the pipe would then wait for that app to be closed, turn a
/// finished install into a timeout, and report it as failed.
const DRAIN_AFTER_EXIT: Duration = Duration::from_secs(2);

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
    if raw.components().count() > 1 {
        return raw.to_path_buf();
    }

    // Windows' own tools come from Windows' own directory, whatever PATH says.
    // This includes the one that checks signatures: found through PATH, a
    // `powershell.exe` put in front by any program of the same user would be
    // run with Odysync's rights and could call every installer valid.
    if let Some(path) = system_tool(program) {
        return path;
    }

    // A name that already carries its extension is something `CreateProcess`
    // resolves correctly on its own.
    if raw.extension().is_some() {
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

/// `System32`, asked of Windows itself: `%SystemRoot%` is an environment
/// variable like any other and can be set per user.
#[cfg(windows)]
pub fn system_directory() -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

    let mut buf = [0u16; 260];
    // SAFETY: the buffer is a live local and its length goes with it.
    let len = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    if len == 0 || len > buf.len() {
        return None;
    }
    Some(std::ffi::OsString::from_wide(&buf[..len]).into())
}

/// The full path of `program` if it is one of Windows' own tools.
#[cfg(windows)]
fn system_tool(program: &str) -> Option<std::path::PathBuf> {
    let system = system_directory()?;
    let lower = program.to_ascii_lowercase();
    let file = if lower.ends_with(".exe") {
        lower
    } else if std::path::Path::new(program).extension().is_none() {
        format!("{lower}.exe")
    } else {
        return None;
    };
    let path = if file == "powershell.exe" {
        system.join(r"WindowsPowerShell\v1.0").join(&file)
    } else {
        system.join(&file)
    };
    path.is_file().then_some(path)
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

    #[cfg(unix)]
    // SAFETY: `setsid` is async-signal-safe and touches nothing of ours. It
    // fails only for a process that already leads a group, which a freshly
    // forked one does not.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
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

/// Read `pipe` to its end into `into`, keeping at most [`MAX_OUTPUT_BYTES`].
///
/// What exceeds the limit is read and dropped: a child blocks on a full pipe
/// that nobody empties.
async fn drain<R: AsyncRead + Unpin>(pipe: Option<R>, into: Arc<Mutex<Vec<u8>>>, program: String) {
    let Some(mut pipe) = pipe else { return };
    let mut buf = vec![0u8; 16 * 1024];
    let mut warned = false;
    while let Ok(n) = pipe.read(&mut buf).await {
        if n == 0 {
            break;
        }
        let mut out = into.lock().unwrap_or_else(|p| p.into_inner());
        let room = MAX_OUTPUT_BYTES.saturating_sub(out.len());
        out.extend_from_slice(&buf[..n.min(room)]);
        if n > room && !warned {
            warned = true;
            tracing::warn!(%program, max = MAX_OUTPUT_BYTES, "output truncated");
        }
    }
}

fn collected(buffer: &Mutex<Vec<u8>>) -> String {
    let raw = buffer.lock().unwrap_or_else(|p| p.into_inner());
    String::from_utf8_lossy(&raw).into_owned()
}

/// Everything a spawned program started, so that it can be stopped as one.
///
/// `kill_on_drop` ends the direct child only. On Windows that child is often
/// just a launcher (`npm.cmd` runs node, `choco` runs the installer), and what
/// it started kept running after the timeout that was supposed to end it.
///
/// Whatever is still running when the program has exited on its own (an app an
/// installer started) is left alone, and may leave the job.
#[cfg(windows)]
struct ProcessTree {
    job: windows::Win32::Foundation::HANDLE,
    armed: bool,
}

// A job handle is a kernel object reference, usable from any thread.
#[cfg(windows)]
unsafe impl Send for ProcessTree {}
#[cfg(windows)]
unsafe impl Sync for ProcessTree {}

#[cfg(windows)]
impl ProcessTree {
    fn of(child: &tokio::process::Child) -> Option<Self> {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        let process = HANDLE(child.raw_handle()?);
        // SAFETY: the process handle belongs to `child`, which outlives this
        // call; the job handle is closed below or in `drop`.
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            // Without this, a program that starts something outside its job
            // (some launchers and updaters do) would be refused.
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            let _ = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of_val(&limits) as u32,
            );
            if AssignProcessToJobObject(job, process).is_err() {
                let _ = CloseHandle(job);
                return None;
            }
            Some(Self { job, armed: true })
        }
    }

    /// The program exited on its own; leave what it started alone.
    fn release(&mut self) {
        self.armed = false;
    }

    /// How many of its processes are still running.
    fn running(&self) -> u32 {
        use windows::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject,
        };
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: the structure is a live local of the size that is named.
        let asked = unsafe {
            QueryInformationJobObject(
                Some(self.job),
                JobObjectBasicAccountingInformation,
                std::ptr::from_mut(&mut info).cast(),
                size_of_val(&info) as u32,
                None,
            )
        };
        if asked.is_ok() {
            info.ActiveProcesses
        } else {
            0
        }
    }
}

/// On Unix the program leads a session of its own, and its process group is
/// what gets stopped.
///
/// Its own session also keeps Ctrl-C in the terminal from reaching it: that
/// would end apt in the middle of unpacking while Odysync says it is waiting
/// for it. The price is that nothing it runs can ask for a password on the
/// terminal; it fails instead.
#[cfg(unix)]
struct ProcessTree {
    group: i32,
    armed: bool,
}

#[cfg(unix)]
impl ProcessTree {
    fn of(child: &tokio::process::Child) -> Option<Self> {
        Some(Self {
            group: i32::try_from(child.id()?).ok()?,
            armed: true,
        })
    }

    fn release(&mut self) {
        self.armed = false;
    }
}

#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        if self.armed {
            // SAFETY: a plain system call; the group is the child's own.
            unsafe {
                libc::killpg(self.group, libc::SIGKILL);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::JobObjects::TerminateJobObject;
        // SAFETY: the handle is ours and still open.
        unsafe {
            if self.armed {
                let _ = TerminateJobObject(self.job, 1);
            }
            let _ = CloseHandle(self.job);
        }
    }
}

/// Run `program` with `args`, capturing output, killed after `timeout`.
///
/// On timeout, and when the future is dropped (a cancelled scan), the program
/// is stopped together with everything it started.
pub async fn run<S: AsRef<OsStr>>(program: &str, args: &[S], timeout: Duration) -> Result<Output> {
    run_and_settle(program, args, timeout, Duration::ZERO).await
}

/// How long an installer's own children may go on after it exited.
pub const INSTALLER_SETTLE: Duration = Duration::from_secs(3 * 60);

/// [`run`] for an installer.
///
/// Some start the real setup and return at once. What the caller does next
/// (read the version back, delete the download, start the next installer) must
/// wait for that one too, so this waits for everything the installer started,
/// for at most [`INSTALLER_SETTLE`]: an app that was launched stays open.
pub async fn run_installer<S: AsRef<OsStr>>(
    program: &str,
    args: &[S],
    timeout: Duration,
) -> Result<Output> {
    run_and_settle(program, args, timeout, INSTALLER_SETTLE).await
}

async fn run_and_settle<S: AsRef<OsStr>>(
    program: &str,
    args: &[S],
    timeout: Duration,
    settle: Duration,
) -> Result<Output> {
    let mut cmd = build(program, args);
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::unavailable(program, format!("{program} was not found on PATH"))
        } else {
            Error::Io(e)
        }
    })?;
    #[cfg(any(windows, unix))]
    let mut tree = ProcessTree::of(&child);

    // Both pipes are drained while waiting. Reading them one after the other
    // deadlocks as soon as a child fills the pipe that is not being read,
    // which winget does on large upgrade lists.
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let mut readers = tokio::task::JoinSet::new();
    readers.spawn(drain(
        child.stdout.take(),
        Arc::clone(&stdout),
        program.to_string(),
    ));
    readers.spawn(drain(
        child.stderr.take(),
        Arc::clone(&stderr),
        program.to_string(),
    ));

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            return Err(Error::CommandTimeout {
                command: program.to_string(),
                seconds: timeout.as_secs(),
            });
        }
    };
    #[cfg(any(windows, unix))]
    if let Some(tree) = &mut tree {
        tree.release();
    }
    #[cfg(windows)]
    if let Some(tree) = &tree {
        let until = tokio::time::Instant::now() + settle;
        while tree.running() > 0 && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    #[cfg(not(windows))]
    let _ = settle;

    // Dropping the set ends readers that are still waiting on a pipe some
    // left-behind child holds open.
    let _ = tokio::time::timeout(DRAIN_AFTER_EXIT, async {
        while readers.join_next().await.is_some() {}
    })
    .await;

    Ok(Output {
        code: status.code().unwrap_or(-1),
        stdout: collected(&stdout),
        stderr: collected(&stderr),
    })
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

    /// The signature check must not go through whatever `powershell.exe` is
    /// first in a PATH that any program of the same user can change.
    #[cfg(windows)]
    #[test]
    fn windows_own_tools_are_used_by_their_full_path() {
        let system = system_directory().expect("System32");
        assert!(system.is_absolute());
        for name in ["powershell.exe", "powershell", "PowerShell.exe"] {
            let path = resolve_program(name);
            assert!(path.starts_with(&system), "{name}: {path:?}");
            assert!(is_windows_powershell(&path.to_string_lossy()));
        }
        for name in ["msiexec.exe", "schtasks", "pnputil"] {
            let path = resolve_program(name);
            assert!(path.starts_with(&system), "{name}: {path:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn an_explicit_extension_or_path_is_left_alone() {
        assert_eq!(
            resolve_program("odysync-not-a-tool.exe"),
            std::path::Path::new("odysync-not-a-tool.exe")
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

    #[tokio::test]
    async fn output_is_capped_while_it_is_read() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let input = std::io::Cursor::new(vec![b'a'; MAX_OUTPUT_BYTES + 100_000]);
        drain(Some(input), Arc::clone(&buffer), "test".into()).await;
        assert_eq!(buffer.lock().unwrap().len(), MAX_OUTPUT_BYTES);
    }

    #[tokio::test]
    async fn small_output_is_kept_whole() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let input = std::io::Cursor::new(b"abc".to_vec());
        drain(Some(input), Arc::clone(&buffer), "test".into()).await;
        assert_eq!(collected(&buffer), "abc");
    }

    /// An installer that starts the app it installed leaves a child behind
    /// that inherited its output pipe. The install is done when the installer
    /// exits, not when that app is closed.
    #[tokio::test]
    async fn a_left_behind_child_does_not_hold_up_the_result() {
        let started = std::time::Instant::now();
        let (prog, args) = if cfg!(windows) {
            (
                "cmd",
                vec!["/c", "echo done & start /b ping -n 20 127.0.0.1"],
            )
        } else {
            ("sh", vec!["-c", "echo done; sleep 20 &"])
        };
        let out = run(prog, &args, Duration::from_secs(15)).await.unwrap();
        assert!(out.success());
        assert!(out.stdout.contains("done"));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "took {:?}",
            started.elapsed()
        );
    }

    /// An installer that hands over to its real setup and returns is not done
    /// until that one is.
    #[cfg(windows)]
    #[tokio::test]
    async fn an_installer_is_done_when_what_it_started_is_done() {
        let dir = tempfile::tempdir().unwrap();
        let done = dir.path().join("done.txt");
        // Files, because quotes inside `cmd /c` arguments do not survive.
        let setup = dir.path().join("setup.cmd");
        std::fs::write(
            &setup,
            format!(
                "@ping -n 4 127.0.0.1 >nul
@echo x> \"{}\"
",
                done.display()
            ),
        )
        .unwrap();
        let stub = dir.path().join("stub.cmd");
        std::fs::write(
            &stub,
            format!(
                "@start /b cmd /c \"{}\"
",
                setup.display()
            ),
        )
        .unwrap();
        let stub = stub.to_string_lossy().into_owned();
        let out = run_installer("cmd", &["/c", &stub], Duration::from_secs(60))
            .await
            .unwrap();
        assert!(out.success());
        assert!(done.exists(), "returned before the real setup had finished");
    }

    /// Ctrl-C in the terminal goes to its foreground group; what Odysync runs
    /// must not be in it.
    #[cfg(unix)]
    #[tokio::test]
    async fn programs_run_in_a_session_of_their_own() {
        let out = run(
            "sh",
            &["-c", "ps -o pgid= -p $$; echo $$"],
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        let ids: Vec<&str> = out.stdout.split_whitespace().collect();
        assert_eq!(ids.len(), 2, "{:?}", out.stdout);
        assert_eq!(ids[0], ids[1], "the program leads its own group");
    }

    /// What a launcher started must not outlive the timeout that ended it.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_timeout_ends_what_the_program_started_too() {
        // A distinctive ping target identifies the grandchild.
        let marker = "127.77.31.99";
        let err = run(
            "cmd",
            &["/c", "cmd /c ping -n 60 127.77.31.99"],
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::CommandTimeout { .. }));

        let query = format!(
            "@(Get-CimInstance Win32_Process -Filter \"Name='PING.EXE'\" | \
             Where-Object {{ $_.CommandLine -like '*{marker}*' }}).Count"
        );
        let out = powershell(&query, Duration::from_secs(60)).await.unwrap();
        assert_eq!(out.stdout.trim(), "0", "the grandchild is still running");
    }
}
