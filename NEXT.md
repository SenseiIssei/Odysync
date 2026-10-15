# What is left, and what is known

State after 3.0.0 (2026-10-16). Everything here is reconstructable from the
repository alone.

---

## 1. Code signing — still the one thing gating the rest

Fully documented in [`docs/SIGNING.md`](docs/SIGNING.md). The release workflow
signs the CLI and the Windows installer as soon as the `SIGNING_ENABLED`
repository variable and the Trusted Signing secrets exist; nothing else has to
change. Until then SmartScreen warns on every install, and **self-update stays
check-only** (`UpdateCheck::is_installable`, and the test
`odysync_refuses_to_install_its_own_updates_while_releases_are_unsigned`, which
will fail on purpose the day signing lands).

macOS needs a Developer ID certificate and notarisation before the `.dmg`
opens without the right-click workaround the README describes.

## 2. Verified where, and how

| Path | How it was verified |
|---|---|
| `direct`, per-user install | End to end on a real machine: VS Code 1.140.0 → 1.142.0 downloaded, hash and signature checked, installed silently, version read back. |
| `direct`, verification chain | Unit tests against real Windows files: right signer passes; another publisher, a changed byte and a wrong SHA-256/SHA-512 are refused; the locked file cannot be written or deleted. |
| `direct`, machine-wide installs (MSI/NSIS/Inno) | Scan verified (Notepad++, Git, GitHub CLI, Node.js found with correct versions); install not run — it needs an elevated session. Same code path as the per-user install apart from `msiexec`. |
| NVIDIA driver | Lookup verified against NVIDIA's service for an RTX 4080 (616.92 → 617.42). The 1 GB install was not run: it needs elevation and restarts the display stack. |
| Runtimes, device problems, system info, startup manager, cleanup | Run against the real machine (Windows) and, for cleanup and startup, a Linux container as an ordinary user. |
| macOS Sparkle updates | Pure parts tested everywhere, including a real Rectangle release's Ed25519 signature; the bundle swap is tested with fake bundles. The full install has not run on a real Mac — **first thing to try on one**. |
| Windows Update install, driver restore | Unchanged from 2.2: unit-tested, not exercised elevated. See the history notes. |

## 3. Worth doing next

- **Grow the catalog.** Each entry needs its signer read from a real installer;
  a pin that stops matching fails safe (the update is refused), so a signer
  change at a publisher shows up as a refused update, not a bad install.
- **AMD and Intel GPU drivers without winget.** Neither vendor publishes a
  machine-readable feed; today these come from Windows Update (default) or
  winget (opt-in).
- **Startup: offer "delay" as well as "off".** Still no fake impact score.
- **Duplicate files**, report-only, if a full-disk hash walk can be made fast
  enough to verify properly.
- **Old branches.** `dev/rust-rewrite`, `dev/updateMilestone2` and
  `dev/iobit-parity` are fully contained in `main`. `dev/ms3` has three
  commits no tag contains — tag it (`archive/ms3`) before deleting it.

---

## 4. Things learned that are not obvious from the code

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
