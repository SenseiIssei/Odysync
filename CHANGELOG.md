# Changelog

All notable changes to Odysync will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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