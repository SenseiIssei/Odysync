# Changelog

All notable changes to Odysync will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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