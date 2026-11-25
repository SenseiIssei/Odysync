# Changelog

All notable changes to Odysync will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security
- **Elevated installers run from a protected directory.** The installer file was
  locked, but another program of the same user could have put a library next to
  it, which would have been loaded with administrator rights.
- Windows' own tools, among them the one that checks every installer's
  signature, are run by their full path instead of through `PATH`.
- Pre-releases such as `1.0.0-rc1`, `2.0b3` and `5.0 Beta 2` were offered as
  stable versions. `3.5a` to `3.5` counted as an upgrade.
- Installing asks the publisher afresh and never uses an answer stored on disk.
  "Is the program running?" is asked again after the download, also works for
  programs whose location is not recorded, and stops the install when it cannot
  be answered.
- An installer that ran into its time limit is never started a second time, and
  what it started is waited for. What is installed is read again right before:
  a program that updated itself since the scan is not reinstalled or downgraded.
- `4.2.8-p15` to `4.2.8` and `7.1-full_build` to `7.1` counted as upgrades.
- Package ids and versions that could be read as options are refused.
- cookie, url and other transitive fixes (RUSTSEC-2026-0594,
  RUSTSEC-2026-0592); chrono, which is no longer maintained, is replaced by
  jiff.

### Fixed
- **Windows Update and driver installs.** The result codes were off by two:
  every successful install was reported as failed, and a real failure passed as
  success. A search that fails is no longer "up to date", an update that waits
  for a restart counts as installed, and a hung Windows Update service no longer
  blocks Odysync.
- **Linux security updates were held back.** Debian and Ubuntu revisions such as
  `+deb12u5` to `+deb12u6` compared as equal, and versions with an epoch could
  not be read. dpkg, rpm and pacman versions follow their own rules now.
- Sources that could not find or install anything: dnf, snap, zypper, fwupd,
  Scoop, Nix, Chocolatey 2, npm packages with a scope, Go tools, cargo, winget
  lists with short columns.
- KeePassXC and PowerShell had silently stopped being updated after their
  publishers renamed or dropped files.
- Being offline, or a source that fails, is shown as such instead of as
  "everything is up to date" with a full score.
- A leftover uninstall entry no longer offers the same update for ever.
- The scan interval, notification, proxy and timeout settings had no effect. The
  system proxy is used when none is set.
- Automatic care and other runs from the command line appear in the history,
  which is also written after every package and survives a damaged file.
- The maximize button, the excluded-packages box and number fields in Settings.
- `--json` prints exactly one document, also for dry runs and with nothing to do.

### Added
- Scans show which sources are still busy, have a time limit each, and can be
  cancelled; installs can be cancelled between packages, with Ctrl-C too.
- Only one Odysync installs at a time, across app, terminal and schedule.
  Quitting during an install asks first.
- An update is not started when the disk has no room for it.
- Background scans at the interval set in Settings, not on battery, with a
  notification.
- `odysync apply --unattended`. Schedules and the daemon leave drivers,
  firmware, system upgrades and developer packages to a person.
- Holds can be set and released in the app.
- Release files carry provenance attestations.

### Changed
- **Faster.** A full scan on the test machine went from 31 to 45 s to 11 to
  24 s. Naming one source asks only that one.
- Windows schedules make up for a missed run and are not stopped when a laptop
  is unplugged. Create yours again to get this.
- pip lists only packages nothing else depends on, and refuses an update that
  would break the rest.
- PowerShell and Node.js stay on the installed line.
- Text and controls meet WCAG AA contrast in both themes; keyboard, focus and
  screen reader support throughout the app.
- macOS: no forced restarts, and a new major version is never an ordinary
  update.

### Removed
- Sources that install a set instead of the approved item, or whose newest
  version nobody publishes: Dell, HP, Lenovo and other maker tools, AMD, Intel
  and Qualcomm graphics through winget, guest tools, VS Code and JetBrains
  extensions, .NET tools, AppImage, Windows optional features. Pacman is scan
  only.
- The settings for parallel sources and automatic installation, which did
  nothing. Automatic care does the second, safely.

## [3.1.1] - 2026-10-30

### Security
- rustls-webpki 0.103.16 and webpki-root-certs 1.0.11: certificate checks
  could accept distrusted intermediates (RUSTSEC-2026-0382, RUSTSEC-2026-0383).
  Every download Odysync verifies goes through this TLS stack.

### Changed
- **Faster scans.** Security updates, Defender definitions and feature upgrades
  share one Windows Update search instead of each running its own; the Update
  Agent answered them one after another. A full scan on the test machine went
  from 34 s to 12 s, and from several minutes to under one on a cold start.
- The app shows what the last scan found as soon as it opens and refreshes it
  in the background.

### Fixed
- PowerShell modules: the scan never found an update (it misread PowerShell's
  output) and took 30 s doing so. It now compares the modules installed from
  the PowerShell Gallery against the Gallery in one query, and installs exactly
  the version it found.

### Maintenance
- A weekly workflow refreshes both lockfiles, so fixes in transitive
  dependencies arrive without waiting for a release of the crate above them.
- A failing weekly CI run on main now opens an issue.

## [3.1.0] - 2026-10-16

### Added
- **Smart Care.** The app opens on one care score with every point taken off
  listed next to its reason, and a single *Fix everything* button that
  installs the safe updates and clears the junk files. Drivers, firmware,
  operating-system upgrades and developer packages (pip, npm, cargo …) are
  never part of it; they are listed for you to start.
- **Automatic care.** One switch schedules `odysync care` daily or weekly: the
  updates that pass every safety rule, then the cleanup categories that are
  safe by default. Also from the command line: `odysync care`,
  `odysync schedule --care`.
- **Disk and battery health** on Windows, macOS and Linux (`odysync health`,
  *System & health* in the app): SMART/NVMe status, wear, temperature, power-on
  hours and battery capacity against its design value, read from the hardware.
- PowerShell 7 and Shotcut in the direct-from-publisher catalog (15 apps).
- Dependabot patch and minor updates are merged automatically once the full CI
  has passed on that exact commit; CI also runs weekly.

### Changed
- A new look: near-black canvas with slow light, glass cards, an animated care
  ring, page transitions, and Plus Jakarta Sans for headings — light and dark.
- Simpler navigation: Home plus Updates, Drivers, Cleanup and Security; the
  rest sits under a collapsible *More tools*.
- Rust edition 2024.

## [3.0.0] - 2026-10-16

### Added
- **Updates without winget.** A new `direct` source reads the operating
  system's own list of installed programs, recognises them from a curated
  catalog and fetches installers straight from the publisher (GitHub releases,
  the VS Code update API, the Node.js release index). Before one runs: the
  version must equal the approved one, the URL must belong to the publisher,
  the file is locked and re-hashed, the published SHA-256 must match, and the
  Authenticode signer must be the one the catalog pins. Installers only run
  over an install of the same technology, never while the program is running.
- **NVIDIA drivers straight from NVIDIA**, looked up per GPU device id on
  NVIDIA's own service, WHQL only, refused unless signed by NVIDIA Corporation.
- **Mac apps through their own Sparkle feeds**: Ed25519 signature from the
  installed app's key, same Team ID, Gatekeeper approval, reversible swap.
- **Cross-platform cleanup**: Trash, caches and crash reports on macOS and
  Linux; new opt-in categories for GPU shader caches and the recently-opened
  files list; `odysync large-files` (report only).
- **Runtimes** (`odysync components`): Visual C++ v14, the legacy DirectX
  runtime and .NET patches, from Microsoft's installers, checked against the
  SHA-512 Microsoft publishes.
- **Device problems** (`odysync devices`): Device Manager codes in plain words.
- **Startup manager on Linux and macOS** (XDG autostart overrides, launchctl),
  and Odysync's own start-at-sign-in on both. `odysync startup`.
- **System info on every platform** (`odysync sysinfo`).
- **Redesigned app**: calm light and dark themes, grouped navigation, update
  tables, cleanup grouped by risk, runtimes, device problems and driver
  backups in the app, optional sources switchable in Settings.
- Platform-native packaging: NSIS on Windows, `.dmg` on macOS, `.deb`, `.rpm`
  and AppImage on Linux, each with the CLI bundled for scheduled runs.
- `--backend` on `scan` and `apply`; `--only` matches catalog ids.
- Frontend tests (Vitest + Testing Library) in CI on all three platforms.

### Changed
- **winget is opt-in**, together with everything that shells out to it
  (Microsoft Store, AMD/Intel/Qualcomm GPU, virtualization guest tools).
- Dependencies: reqwest 0.13, zip 9, directories 6, sha2 0.11, Tailwind 4,
  TypeScript 7, lucide 1.x; framer-motion removed. MSRV 1.90.
- PowerShell output is read as UTF-8, so localised names survive.

### Fixed
- GPU detection found no adapter on non-English Windows: pnputil translates
  its labels. Adapters now come from `Win32_VideoController`.
- Signature checks failed when Odysync was started from PowerShell 7, whose
  module path Windows PowerShell inherited.
- Schedules created in the app registered the GUI binary, which ignores
  command-line verbs, so they never updated anything.
- Optional sources could not be enabled from the app.
- The build failed on Linux and four cleanup tests failed on macOS/Linux.

### Removed
- `odysync-version-source`: never called, and its default endpoint pointed at
  a domain that does not exist.

The changes listed under 2.2.0 below were never released on their own and
ship as part of 3.0.0.

## [2.2.0] - not released separately

### Unreleased fixes folded into 3.0.0

### Fixed
- **A driver restore reported failure when it had succeeded.** `pnputil
  /add-driver /install` returns `ERROR_NO_MORE_ITEMS` (259) when the package
  was added to the store but no device needed it — every matching device
  already had a driver at least as good. That is the normal outcome of
  restoring a still-current package, and it is also what happens in the case
  the module documents at length: Windows ranks driver packages, so a newer one
  still in the store keeps winning. `RestoreReport` now has a third bucket,
  `already_current`, because folding it into `failed` misreports a working
  restore and folding it into `restored` claims a device changed when none did.
  Found by running a real restore elevated, which printed *"Re-added 0
  package(s); 1 failed"* for an operation that had done exactly the right
  thing.
- **Windows Update errors bypassed the translation table that existed to
  explain them.** `explain_hresult` was only ever reached by download and
  install *result codes*, never by a failure of the search itself — which is
  the most common way this goes wrong. A real elevated run surfaced
  `the update search failed: 0x8024001E` and left the user with a hex number.
  `wua_err` now takes the `HRESULT` and routes it through the table.
- Added `0x8024001E` (the Update Agent would not start its server for this
  session — seen when UAC elevates into a different account than the one
  signed in) and `0x8024001F` (no network) to that table.

### Added
- `odysync drivers backup --only oem23.inf` backs up named packages instead of
  the whole driver store. A name that matches nothing is an error rather than
  an empty backup, which would look like success right until someone needed it.

### Included from the 2.2 line

### Added
- **Windows Update.** Odysync now installs Windows' own security, quality and
  Defender updates, not just drivers. Three backends over one shared Windows
  Update Agent wrapper: `windows-update` (security and quality),
  `windows-defender-update` (definitions) and `windows-feature-update`
  (version upgrades). Updates are classified by category **GUID**, never by
  name, because category names are localised.
- Feature upgrades are **off until opted into** by name in `enabled-backends`.
  Gating them through `BackendKind::enabled_by_default` rather than a default
  `disabled-backends` entry means the gate also applies to config files
  written by earlier versions.
- `BackendKind::ALL` and `BackendKind::from_id`, replacing the hand-written
  46-arm id match in the Tauri layer. A test keeps the list exhaustive.
- **Hold a Windows update by its KB number.** Its native id has to be
  `<guid>.<revision>` — that is what the Update Agent accepts, and pinning the
  exact revision is required by the backend contract — but nobody holds an
  update by GUID. `PackageId` now carries an optional alias that the policy
  engine also matches, so `odysync hold KB5101650` works. The alias is
  deliberately excluded from equality and hashing: `PackageId` is a map key and
  correlates a scan result with its apply outcome, so a label must not change
  identity.
- **`odysync pause <backend> --days N` and `odysync resume <backend>`.** A
  deadline rather than an off switch — "off" gets set during one bad week and
  is still off a year later, which is how machines end up unpatched. An
  unparseable deadline in a hand-edited config is treated as *expired* rather
  than permanent, so a typo cannot silently stop security updates forever.
- **A Windows Update card on the Updates page**, breaking pending updates down
  by class — security & quality, Defender definitions, drivers, feature
  upgrades — with the blocking reason per row. The generic list flattens a real
  distinction: on an unelevated run every Windows update lands in *skipped*, so
  13 pending security updates sat inside a list of 49 skipped entries and read
  as "nothing to do".
- An empty or whitespace-only pattern in `exclude` or `holds` now matches
  nothing. It previously compared equal to a package whose native id was
  empty, and silently skipping an update is the exact failure the policy engine
  exists to prevent.
- `odysync backends` now names **who** verifies each backend's payloads —
  winget, apt, the Windows Update Agent — instead of implying Odysync did.
  Odysync almost never holds an installer file: the package manager downloads
  and validates it inside its own process. AppImage is reported as *not
  verified*, with the reason.
- **Disk cleanup.** `odysync clean` measures ten categories — temp files,
  thumbnail caches, Recycle Bin, error reports, crash dumps, Windows Update
  downloads, Delivery Optimization, browser caches, superseded components,
  `Windows.old` — and deletes nothing. `--apply` is required to remove
  anything, `--only` to touch a category that is not safe by default, and
  anything irreversible is named individually before the prompt.
  - Three independent path checks guard every deletion: roots are filtered
    through `is_safe_cleanup_root` so an unset `%LOCALAPPDATA%` cannot collapse
    one into `\Temp`; the walk never follows a directory symlink, so a junction
    inside a cache cannot lead it elsewhere; and each path is re-checked with
    `is_within` immediately before removal.
  - Browser caches are per profile and **refused while the browser is
    running**. Only `Cache`, `Code Cache`, `GPUCache` and `cache2` — never
    `Cookies`, `Login Data` or `Network`. A test asserts no declared directory
    is a credential store.
  - `Windows.old` is measured and never deleted: it is owned by
    TrustedInstaller, and a partial reimplementation of Disk Cleanup would
    leave the tree in a state neither tool understands.
  - `DISM /StartComponentCleanup` runs **without** `/ResetBase`, which would
    additionally make every installed update permanently un-uninstallable.
- **`odysync optimize-disk`** — `Optimize-Volume`, TRIM on SSD and defrag on
  HDD, chosen from the media type with no override. Defragmenting an SSD writes
  the whole drive for no benefit.
- **`odysync self-check`** reports whether a newer Odysync exists, comparing
  versions with the same algebra the policy engine uses rather than as strings.
  It **refuses to install one**: downloading an unsigned executable and running
  it elevated is the same supply-chain hole the v1 rewrite existed to close, so
  code signing comes first. A test enforces that refusal.
- **The daemon no longer scans on battery.** A scan spawns one process per
  detected backend — over thirty on a well-equipped Windows machine — and doing
  that hourly drains a laptop for nothing. `--on-battery` opts back in, and an
  explicit `--once` run always proceeds.
- The daemon logs its resident memory at startup, so the 15 MB idle target in
  `ROADMAP.md` §4 is measured rather than assumed.
- **Release signing wired into CI**, gated on a `SIGNING_ENABLED` repository
  variable so the workflow keeps working until the Azure Trusted Signing
  procurement completes and starts signing the moment it does. Both the CLI
  binaries and the NSIS installer are signed, with an RFC 3161 timestamp.
  `docs/SIGNING.md` covers the setup, the certificate choice and the ordering.
- SHA-256 checksums are now published for the installer, not just the CLI
  archives, and CI generates **winget manifests** for each release so Odysync
  becomes installable with `winget install`.
- **Driver backup and rollback.** `odysync drivers backup | list |
  rollback --to <id> | prune --keep <n>` exports the Windows driver store per
  package, with a manifest, and can re-add a saved package later. Exporting
  works unelevated; restoring does not.
  - Nothing parses a `pnputil` label — its output is fully localised and the
    console code page mangles non-ASCII on the way out. The parser reads
    values: `oem<n>.inf` is a published name in every language.
  - Automatic backup before applying driver updates is **off by default**:
    `DriverStore\FileRepository` measured 4.7 GB here, so an automatic export
    at the default retention would spend over ten gigabytes of the user's disk.
    `odysync apply` instead names the most recent backup, or says plainly that
    there is none. `driver-backup-before-apply` opts in.
  - Restore is not a forced downgrade, and says so on every run: Windows ranks
    driver packages, so a newer one still in the store can keep winning.
- Dependabot for cargo, npm and GitHub Actions, grouped weekly.
- `npm audit --audit-level=moderate` in the GUI CI job. Rust dependencies were
  audited by `cargo-audit` and `cargo-deny`; the 116 npm packages were not
  audited at all.

### Security
- **The offline cache refused nothing.** `download_and_cache` accepted an
  arbitrary URL from the front-end, including plaintext `http://`, where an
  installer can be replaced in transit — and `expected_sha256` is optional, so
  often nothing would have caught the swap. It is now HTTPS-only, and other
  schemes are refused.
- The offline cache is now the first real call site of `odysync-verify`. A
  download is staged under a `.partial` name, its Authenticode signature is
  checked, and only then is it moved into place. An **invalid** signature
  deletes the file and fails the download; unsigned is allowed but recorded in
  the manifest as `CachedSignature::Unsigned` rather than passing silently.
- A malformed `expected_sha256` is rejected as a caller error instead of
  degrading into an ordinary "digest did not match".

### Fixed
- **No driver could ever be installed.** The driver backend set
  `available = Version::parse("{update_guid}.{revision}")`, which collapses to
  the GUID's first block and parses as `Version::Unknown`. The policy engine
  blocks unknown versions and `apply` refused them outright, so drivers were
  listed and never installable. Windows updates now carry an orderable
  `0` → revision pair instead.
- **Driver installs never downloaded and never accepted a EULA.** The install
  path went straight from `SetUpdates` to `Install`, which fails with
  `WU_E_NOT_DOWNLOADED` unless Windows had already cached the update.
- **A failed search discarded valid results.** `orcFailed` was treated as "no
  updates", but the Windows Update Agent returns that code alongside a
  populated collection when one source in the aggregate fails. On the machine
  this was found on, that meant throwing away 17 pending updates.
- **Windows Update appeared to require elevation, and did not.** The WUA
  backends activated the COM object with `CLSCTX_LOCAL_SERVER`, forcing
  out-of-process activation that an unelevated caller may not perform —
  `E_ACCESSDENIED`. With `CLSCTX_ALL` the search runs unelevated, so an
  ordinary user now sees pending security updates and the reason they cannot
  be applied yet, instead of an empty page.
- **`RunReport::reboot_required` was never set to `true`.** It had three
  display sites in the GUI, one in the CLI renderer, and no assignment
  anywhere. The runner now asks the OS through `platform::reboot_pending`,
  which covers every backend rather than only the one that installed.
- **The npm and VS Code backends never worked on Windows.** `CreateProcess`
  only appends `.exe` when searching `PATH`; npm ships as `npm.cmd` and the VS
  Code CLI as `code.cmd`, so both reported themselves unavailable on every
  Windows host. `proc::resolve_program` now walks `PATH` against `PATHEXT`
  without routing through a shell.
- Duplicate Windows Update offerings — the same Defender signature listed once
  per servicing channel — are deduplicated on identity.
- The GUI lockfile version, which had drifted to `2.0.0-alpha.1` against a
  `2.1.0` `package.json`.

### Removed
- The v1 Python implementation (`legacy/`, 23 files). The Rust CLI has had
  feature parity since Phase 2, and this was the last open item on the
  rewrite roadmap. The Python tool remains available at the `v1.3.0` tag.

## [2.1.0]

### Added
- **Security page** — posture and indicator-of-compromise audit across five
  sections (Defender, persistence, integrity, network, hardening). Drives
  Microsoft Defender for malware scanning rather than implementing an engine;
  each section fails independently so a failure is never mistaken for a clean
  result. Remediation quarantines instead of deleting, refuses paths under
  `C:\Windows` or `Program Files`, and requires explicit confirmation.
- **Hardware Updates page** — drivers, firmware and vendor tools, grouped, with
  a separate confirmation step for firmware and a forced restore point.
- Start with Windows, optionally minimised to the tray (`--minimized`).
- `Backend::list_installed()` — a real inventory call, implemented for winget,
  chocolatey, scoop, pip, npm, cargo, dotnet and VS Code.
- Log viewer auto-scroll; error boundaries and frontend crash reporting into
  `odysync.log`.

### Fixed
- **Saving settings reset the entire config**, destroying `policy.holds` and
  `policy.exclude`: `Config` serializes kebab-case, the GUI posted snake_case,
  and `#[serde(default)]` with no `deny_unknown_fields` read that as "all
  fields absent". A held package silently became updatable again.
- **Arbitrary file deletion** via `remove_offline_entry`: an unvalidated
  `filename` from `manifest.json` was joined to the cache directory, and an
  absolute path there discards the base entirely.
- **PowerShell command injection** in `toggle_startup_program`, which
  interpolated registry-sourced strings unescaped; the same function referenced
  an undefined `$enable`, so entries could be disabled but never re-enabled.
- Every page refetched on navigation, losing state and re-scanning in a loop.
- `restore_backup` used the list index as the restore-point sequence number.
- Update history was never written (`Runner::with_history` had no call sites)
  and used a different config directory than everything else.
- "Installed Packages" listed only *upgradable* packages.
- Backend availability was re-probed (~36 process spawns) on every page load
  and always reported `true`.
- Failures reported as facts: a denied restore-point query as "none found", a
  disabled System Protection as the 24-hour throttle, unexpanded `%SystemRoot%`
  paths as deleted files, and an unparsed `{"value":[…]}` collection as
  "Defender absent or superseded by a third-party AV".
- `Remove-MpThreat -ThreatID` — no such parameter exists.

### Changed
- Severity policy: "unsigned", "outside `C:\Windows`" and "user-writable" are
  treated as context rather than signals, since per-user installs are how most
  software ships and self-built binaries are unsigned by definition. Only real
  indicators escalate. Handled Defender detections that never executed drop to
  informational; ones that *ran* keep full severity.
- Apply outcomes expose a stable status discriminant instead of `Debug` output.
- `Config::max_retries` is now actually used by the runner.

## [Unreleased] - v2.0.0

### Added
- Complete Rust rewrite of the Python toolchain
- `odysync-core` crate: policy engine, version algebra, planner, runner, config
- `odysync-verify` crate: installer digest and signature verification
- `odysync-backends` crate: winget, msstore, Windows drivers, homebrew, apt, flatpak
- `odysync-cli` crate: the `odysync` CLI (scan, apply, backends, hold, unhold, config, maintain, schedule, unschedule, diagnostics, daemon)
- `odysync-gui` crate: Tauri v2 + React + TypeScript + TailwindCSS desktop GUI
  - Updates tab: scan, select, apply with dry-run and restore point options
  - Maintenance tab: temp cleanup, recycle bin, system health, startup programs
  - Schedule tab: create/remove daily/weekly scheduled tasks
  - Settings tab: policy toggles, exclusions, backend status, config save
  - Dark/light mode with system-following default
- `odysync daemon` command for background scan/apply loops
- System tray icon with Show/Scan/Quit menu
- CI: cargo-audit, cargo-deny, CycloneDX SBOM generation
- Release workflow: cross-platform CLI builds with SHA-256 checksums, Tauri GUI NSIS installer

### Changed
- Windows driver updates now use the built-in Windows Update Agent COM API
- Version comparison uses proper semver ordering instead of string comparison
- Failed updates no longer fall back to reinstalling from scratch
- README rewritten for v2

### Fixed
- Lexical version comparison trap (`1.10` vs `1.9`)
- Unknown-version upgrades that could sidegrade or downgrade packages
- Reinstall-on-failure that wiped working package state
- Supply-chain hole from runtime-installed PowerShell module
- `is_elevated()` now uses `OpenProcessToken` + `GetTokenInformation`

### Security
- Policy engine refuses downgrades, same-version reinstalls, and pre-releases by default
- Store apps refused while elevated; driver updates refused without elevation

## [1.3.0] - 2025-10-20
### Added
- Visible progress animation during silent and interactive installs.
- Upgrade-scan cache with TTL (default 15 minutes), configurable via settings.
- Profiles export/import via CLI and Menu.
- Microsoft Store Library helper to open updates page.
- Log files for every run in `%LOCALAPPDATA%\Odysync\logs\`.

### Changed
- Faster, more resilient winget parsing with graceful timeouts and cache fallback.
- Settings file now includes defaults and cache TTL.

### Fixed
- Menu responsiveness while scanning; clearer feedback when winget is slow.

## [1.2.0] - 2025-10-20
### Added
- Run summaries and exportable reports (`--report json|txt`, `--out <path>`)
- Profiles and non-interactive updates (`--profile <name>`, `--yes`)
- Diagnostics pack (`--diagnostics`, `--diag-out <zip>`)
- Pending reboot detection with safety warning
- Scheduling via Windows Task Scheduler (`--schedule weekly|monthly`, `--time`, `--task-name`, `--unschedule`)
- EXE-first build flow and build script
- Modular package structure under `src/`

### Changed
- Smarter handling of Microsoft Store apps with context guidance
- Aggregated results for winget updates (updated, interactive, reinstalled, skipped, store-skipped, failed)
- Expanded README and troubleshooting

### Fixed
- Encoding issues by enforcing UTF-8 in PowerShell and console

### Notes
- No telemetry; privacy by default