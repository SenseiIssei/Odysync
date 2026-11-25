# What is left, and what is known

State of 2026-11-25, after the care round that follows 3.1.1. Everything here is
reconstructable from the repository alone. The plan is in
[ROADMAP.md](ROADMAP.md), Part A; this file says where things stand.

---

## 1. The next five

1. **Exercise every install path on throw-away machines** (A1). Nothing else
   pays back as much; see section 5 for what not doing it cost.
2. **Signing** (A2). The route this project had planned is closed to individuals
   in Germany; [docs/SIGNING.md](docs/SIGNING.md) has the two that are open.
   Needs the owner.
3. **The catalogue from 15 to 150** (A3), drafted by a bot, pinned by a check.
4. **Undo for an update** (A4).
5. **Exploited holes first, a waiting period for the rest** (A5).

## 2. Waiting for the owner

- Allow Actions to open pull requests, or merge the weekly lockfile refresh by
  hand from the issue it opens. The workflow is ready for both.
- The signing route, and whether it may cost anything.
- The elevated helper (A6): per batch, as recommended, or a service.
- Submitting to winget-pkgs; an own Scoop bucket and Homebrew tap.
- Deleting old branches on GitHub: `chore/lockfiles-2026-*` (four), `dev/*`,
  `feat/smart-care`, `backup/*`. `dev/ms3` has three commits no tag contains;
  tag it (`archive/ms3`) first.

## 3. Verified where, and how

| Path | How it was verified |
|---|---|
| `direct`, per-user install | End to end on a real machine (3.0): VS Code 1.140.0 to 1.142.0. Not repeated since; the code around it changed. |
| `direct`, verification chain | Unit tests against real Windows files: right signer passes; another publisher, a changed byte and a wrong hash are refused; the locked file cannot be written or deleted. |
| `direct`, catalogue | All 15 entries resolved against their publishers on 2026-11-25; weekly in CI from now on. |
| `direct`, machine-wide installs | Scan verified. **Install not run**: needs an elevated session. |
| Protected download directory | Unit test: an ordinary process cannot write into it; elevated (CI) the access list is read back. |
| Windows Update search | Run on a real machine: result codes, reboot and preview flags read. |
| Windows Update and driver **install** | **Never run.** The result codes that made it impossible to succeed are fixed and checked against the bindings; the first real install is A1. |
| NVIDIA driver | Lookup verified for an RTX 4080. Install not run. |
| dpkg, rpm, pacman, PEP 440 ordering | The reference test cases of each project. |
| apt, dnf, zypper, snap, flatpak, pacman, nix, fwupd, Homebrew, Chocolatey, Go | Parsers against documented output or the tools' own source, **not** against captured output of the real tool. Compiled and linted for Linux and macOS. |
| pip, npm, cargo, Scoop | Scans run on a real machine. Installs not run. |
| macOS Sparkle updates | Pure parts and a real Ed25519 signature; the bundle swap with fake bundles. **Never run on a real Mac.** |
| Windows schedule | The definition is parsed back in a test; CI registers, finds and removes a real task. Not run on a schedule yet. |
| The app | 91 component tests, contrast computed for every text colour in both themes, the browser demo looked at. The window itself was not opened for this round. |
| Runtimes, device problems, system info, startup manager, cleanup | Unchanged; run against the real machine for 3.0 and 3.1. |

## 4. Known and left

- **Extensions and .NET tools** are not offered any more: nobody says which
  version is newest without an undocumented service. They were listed before,
  always as "could not be determined".
- **Arch** is scan only. pacman has no supported way to update one package.
- **winget** rows with `< 1.2.3` as the installed version are refused.
- **Homebrew casks that install a system package** ask for a password, which
  nothing Odysync starts can do any more: programs run in a session of their own
  so that Ctrl-C does not reach them. They fail; `brew upgrade` by hand works.
- **PowerShell modules** from an untrusted gallery may fail now that `-Force`
  is gone.
- **The offline cache page** shows a cache nothing fills (A15).
- **A console window** appears when a schedule runs on Windows (A9).

---

## 5. Things learned that are not obvious from the code

**A constant written from memory cost four releases.** `ORC_FAILED = 2` looked
right, a comment explained how a "failed" search can still return results, and
the module's own history told of 14 pending updates "found by running the real
API". They were there because the search had succeeded. When a system answers
something that needs an explanation, check the reading first.

**A conditional request is not free at GitHub.** Three `304` answers without a
token took the anonymous allowance from 58 to 56. The comment said otherwise.

**A pipe ends when the last holder lets go, not when the program exits.** An
installer that starts the app it installed kept its output pipe open, so the
install "ran" until the app was closed and then counted as timed out, and was
started again.

**Locking the installer does not protect the installer.** A program loads
libraries from its own directory first. The directory needs the lock too.

**PATH is an input.** The signature check ran whatever `powershell.exe` came
first in a list any program of the same user can write.

**Dependabot cannot raise something that lives in two ecosystems.** Tauri's Rust
and npm halves must match; each half alone failed the packaging job every week.
And a workflow's `gh pr create` needs a repository setting that is off by
default, so the refresh that was to fix this failed silently as well.

**"Not found" and "could not look" must never look the same.** Offline, a
publisher that renamed a file, a Windows Update that failed: all three read as
"up to date" until this round.

**Windows PowerShell inherits PowerShell 7's module path.** Started from
`pwsh`, `powershell.exe` loads PS7's copy of `Microsoft.PowerShell.Security`,
which fails, so `Get-AuthenticodeSignature` "could not be loaded" and every
signature check failed. CI found it — GitHub's Windows runners run steps in
`pwsh`. `proc::build` now removes `PSModulePath` for `powershell.exe`.

**Console tools translate their labels — the GPU detector had the same bug as
the driver-backup parser.** It matched `Instance ID:` in pnputil output; a
German Windows prints `Instanz-ID:`, so no GPU was ever found and all four GPU
backends were silently dead. Read values from CIM/JSON, never labels.

**Windows PowerShell writes redirected output in the OEM code page.** A device
named "Unbekanntes USB-Gerät" arrived with a replacement character. Every
script now starts by setting `[Console]::OutputEncoding` to UTF-8.

**A schedule must point at a binary that understands the schedule.** The
scheduler registered `current_exe() apply --yes`; from the app, that is the
GUI, which ignores arguments — so the schedule opened a window and updated
nothing. The app now bundles the CLI and schedules that.

**Shader caches are the biggest junk on a gaming PC.** 50.6 GB on the
development machine, more than temp files and browser caches together.

**`pnputil` output is fully localised, and the console mangles it.** On a
German machine `Published Name` reads `Veröffentlichter Name`, with the umlaut
damaged by the code page. `driver_backup::parse_enum_drivers` therefore reads
*values*, never labels: an `oem<n>.inf` token is a published name in every
locale. Do not "improve" it by matching on field names.

**That parser had a bug unit tests could not catch.** It split records on
`"\n\n"`. `pnputil` is a Windows console program, so its blank line is
`\r\n\r\n` and the split never matched — the whole output collapsed into one
block and it returned **1 package on a machine with 215**. The tests passed
throughout, because they were written with `\n` and only ever tested their own
assumption. Every parser test now runs against both line endings, and one
asserts a record *count*, because the failure mode returns exactly one item no
matter how many exist. **Assert counts against ground truth, not shapes.**

**A full driver-store export is about 5 GB.** Measured:
`DriverStore\FileRepository` was 4.7 GB across 5 229 files, and a single
exported package was 22.5 MB × 215 packages. That measurement is why automatic
pre-apply backup is off by default — at the default retention it would spend
over ten gigabytes of the disk the tool is meant to be looking after, and
Windows already keeps a single-step rollback in Device Manager.

**The obvious way to measure the Recycle Bin is wrong.**
`Shell.Application.NameSpace(0xA).Items().Size` reported **1.1 MB** where the
truth was **881.7 MB**: `.Size` does not recurse into deleted folders, and it
missed the `D:` drive entirely. `SHQueryRecycleBinW` is authoritative. This is
also why `cleanup` does not walk `$Recycle.Bin` itself — that path is
per-user, permission-guarded, and exactly the shape the root-safety rule
refuses.

**Odysync almost never holds an installer file.** winget, apt, Homebrew and the
Update Agent each download, verify and execute the payload inside their own
process. This is why `odysync-verify` had no call sites for so long, and why
wiring it into the runner would have produced a hook that never fires. The one
real call site is the offline cache. `verification_of` exists to answer "who
checked this?" honestly instead of showing a tick that could be misread.

**Measure before trusting a number the tool prints.** Every figure in the 2.2 plan (`docs/history/`)
was checked against an independent count. Two matched exactly, one did not —
and the one that did not was the *check* being wrong, not the tool. That is
only distinguishable because both were computed.
