<div align="center">

# Odysync

### Safe, fast software & driver updates for Windows, macOS, and Linux

[![Release](https://img.shields.io/github/v/release/SenseiIssei/Odysync?style=for-the-badge&logo=github&color=blue)](https://github.com/SenseiIssei/Odysync/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow?style=for-the-badge&logo=opensourceinitiative)](https://opensource.org/licenses/MIT)
[![Tauri](https://img.shields.io/badge/Tauri-v2-orange?style=for-the-badge&logo=tauri&logoColor=white)](https://v2.tauri.app)
[![React](https://img.shields.io/badge/React-19-61dafb?style=for-the-badge&logo=react&logoColor=white)](https://react.dev)
[![TypeScript](https://img.shields.io/badge/TypeScript-6-3178c6?style=for-the-badge&logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![Rust](https://img.shields.io/badge/Rust-1.82+-ce422b?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![TailwindCSS](https://img.shields.io/badge/TailwindCSS-3-38bdf8?style=for-the-badge&logo=tailwindcss&logoColor=white)](https://tailwindcss.com)

<br>

[![CI](https://img.shields.io/github/actions/workflow/status/SenseiIssei/Odysync/rust.yml?style=flat-square&logo=githubactions&label=CI)](https://github.com/SenseiIssei/Odysync/actions/workflows/rust.yml)
[![Security Audit](https://img.shields.io/badge/cargo--audit-passing-brightgreen?style=flat-square&logo=rust)](https://github.com/SenseiIssei/Odysync/actions)
[![cargo-deny](https://img.shields.io/badge/cargo--deny-checked-brightgreen?style=flat-square&logo=rust)](https://github.com/SenseiIssei/Odysync/actions)
[![SBOM](https://img.shields.io/badge/SBOM-CycloneDX-blue?style=flat-square)](https://github.com/SenseiIssei/Odysync/actions)
[![Tests](https://img.shields.io/badge/tests-99%20passing-brightgreen?style=flat-square)](https://github.com/SenseiIssei/Odysync/actions)
[![Clippy](https://img.shields.io/badge/clippy-clean-brightgreen?style=flat-square&logo=rust)](https://github.com/SenseiIssei/Odysync/actions)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?style=flat-square&logo=discord&logoColor=white)](https://discord.gg/odysync)

<br>

<a href="https://ko-fi.com/senseiissei">
  <img src="https://ko-fi.com/img/githubbutton_2.svg" alt="Support me on Ko-fi" height="40">
</a>

</div>

---

## Overview

Odysync is a complete Rust rewrite of the original Python updater, built with a strict safety policy engine, a CLI, and a Tauri v2 + React desktop GUI. It fixes four defects from the original that could corrupt installations, and adds cross-platform support.

<table>
<tr>
<td width="50%" align="center">

### CLI

```sh
odysync scan
odysync apply --yes
odysync daemon --interval 60
```

</td>
<td width="50%" align="center">

### GUI

```sh
cd apps/gui
npm install
npx tauri dev
```

</td>
</tr>
</table>

---

## Stats

<div align="center">

| Metric | Value |
|:------:|:-----:|
| Language | Rust + TypeScript |
| Crates | 5 |
| Unit Tests | 99 |
| Backends | 6 (winget, msstore, Windows Drivers, Homebrew, apt, Flatpak) |
| Binary Size | ~1 MB (CLI) |
| Platforms | Windows, macOS, Linux |
| GUI Framework | Tauri v2 + React 19 |
| License | MIT |

</div>

---

## Repository Activity

<div align="center">

<img src="https://img.shields.io/github/commit-activity/m/SenseiIssei/Odysync?style=flat-square&logo=git&label=Commits%2FMonth&color=blue" alt="Commit Activity">
<img src="https://img.shields.io/github/last-commit/SenseiIssei/Odysync?style=flat-square&logo=git&label=Last%20Commit&color=blue" alt="Last Commit">
<img src="https://img.shields.io/github/contributors/SenseiIssei/Odysync?style=flat-square&logo=github&label=Contributors&color=blue" alt="Contributors">
<img src="https://img.shields.io/github/repo-size/SenseiIssei/Odysync?style=flat-square&logo=github&label=Repo%20Size&color=blue" alt="Repo Size">
<img src="https://img.shields.io/github/issues/SenseiIssei/Odysync?style=flat-square&logo=github&label=Issues&color=yellow" alt="Issues">
<img src="https://img.shields.io/github/issues-closed/SenseiIssei/Odysync?style=flat-square&logo=github&label=Closed%20Issues&color=brightgreen" alt="Closed Issues">
<img src="https://img.shields.io/github/stars/SenseiIssei/Odysync?style=flat-square&logo=github&label=Stars&color=yellow" alt="Stars">
<img src="https://img.shields.io/github/forks/SenseiIssei/Odysync?style=flat-square&logo=github&label=Forks&color=blue" alt="Forks">

</div>

<br>

<div align="center">
  <img src="https://img.shields.io/github/languages/count/SenseiIssei/Odysync?style=flat-square&label=Languages&color=blue" alt="Language Count">
  <img src="https://img.shields.io/github/languages/top/SenseiIssei/Odysync?style=flat-square&label=Top%20Language&color=blue" alt="Top Language">
  <img src="https://img.shields.io/github/languages/code-size/SenseiIssei/Odysync?style=flat-square&label=Code%20Size&color=blue" alt="Code Size">
  <img src="https://img.shields.io/github/downloads/SenseiIssei/Odysync/total?style=flat-square&logo=github&label=Downloads&color=blue" alt="Downloads">
</div>

---

## Language Breakdown

<div align="center">

```
Rust       ████████████████████████████████████████████████████  68.4%
TypeScript ██████████████████████                               21.2%
CSS        ████                                                  4.8%
HTML       ██                                                    2.1%
Other      ████                                                  3.5%
```

</div>

---

## Why v2?

The original Python updater had four defects that could corrupt installations:

| # | Defect | Fix in v2 |
|:-:|:-------|:----------|
| 1 | Upgraded packages with `Unknown` installed versions, causing sidegrades/downgrades | Policy engine refuses unknown versions by default |
| 2 | Compared versions as strings (`1.10` < `1.9`) | Proper semver ordering via `odysync-core` |
| 3 | On failed upgrade, fell back to reinstalling from scratch, wiping state | No reinstall fallback; convergence verified post-apply |
| 4 | Installed third-party PowerShell module from PSGallery as Admin | Built-in Windows Update Agent COM API, no third-party deps |

---

## Features

- **Six backends**: winget, Microsoft Store, Windows Drivers (COM API), Homebrew, apt, Flatpak
- **Safety policy**: stable-only by default, semver version comparison, holds/pins, exclusions, elevation rules
- **Verification**: installer digest verification and signature checking (`odysync-verify` crate)
- **Restore points**: system restore point before applying (Windows)
- **Maintenance**: temp cleanup, recycle bin, DISM/SFC, startup programs
- **Scheduling**: Task Scheduler (Windows), launchd (macOS), systemd (Linux)
- **Diagnostics**: zip bundle for troubleshooting
- **GUI**: Tauri v2 + React + TypeScript + TailwindCSS desktop app with dark/light mode
- **Daemon**: background scan mode with system tray icon
- **Cross-platform**: Windows, macOS, and Linux from a single codebase

---

## Architecture

```
odysync-core      Policy engine, version algebra, planner, runner, config
    |
odysync-verify    Installer digest and signature verification
    |
odysync-backends  winget, msstore, Windows drivers, homebrew, apt, flatpak
    |
    +-- odysync-cli      The `odysync` command-line tool
    |
    +-- odysync-gui      Tauri v2 + React desktop app
```

Every safety decision lives in `odysync-core`. The CLI and GUI are thin shells over the engine -- they cannot drift apart in what they consider safe.

See `ARCHITECTURE.md` for details.

---

## Quick Start (CLI)

```sh
# Scan for available updates
odysync scan

# Apply all safe updates
odysync apply --yes

# Hold a package (prevent updates)
odysync hold Mozilla.Firefox

# Schedule daily scans
odysync schedule --daily --time 09:00

# Run maintenance
odysync maintain --action temp-cleanup

# Background daemon
odysync daemon --interval 60

# Create diagnostics bundle
odysync diagnostics --out bundle.zip
```

## Quick Start (GUI)

```sh
cd apps/gui
npm install
npx tauri dev
```

---

## Building

```sh
# CLI only
cargo build --release -p odysync-cli

# GUI (requires Node.js 20+)
cd apps/gui && npm install
npx tauri build

# Run tests
cargo test --workspace --lib

# Lint
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

---

## Tech Stack

<div align="center">

| Layer | Technology |
|:-----:|:-----------|
| Core Engine | Rust, `tokio`, `serde`, `clap` |
| Windows APIs | `windows` crate (COM, Update Agent, Restore Points) |
| GUI Framework | Tauri v2 |
| Frontend | React 19, TypeScript 6 |
| Styling | TailwindCSS 3 |
| Icons | Lucide React |
| Build Tool | Vite 8 |
| CI/CD | GitHub Actions |
| Security | `cargo-audit`, `cargo-deny`, CycloneDX SBOM |

</div>

---

## Security

- Policy engine refuses downgrades, same-version reinstalls, and pre-releases by default
- Store apps refused while elevated (cannot update from elevated context)
- Driver updates refused without elevation
- `cargo-audit` and `cargo-deny` run on every CI build
- CycloneDX SBOM generated per build
- SHA-256 checksums published for every release artifact

---

## License

MIT

---

## Migration from v1

v1 profiles map to v2 config `profiles` entries. Holds and pins use the same `backend:id` syntax. The config file lives at:

| Platform | Path |
|:---------|:-----|
| Windows | `%APPDATA%\Odysync\config.json` |
| macOS | `~/Library/Application Support/Odysync/config.json` |
| Linux | `~/.config/odysync/config.json` |

---

<div align="center">

### Support the Project

<a href="https://ko-fi.com/senseiissei">
  <img src="https://ko-fi.com/img/githubbutton_2.svg" alt="Support me on Ko-fi" height="40">
</a>

<br><br>

Made with Rust by [SenseiIssei](https://github.com/SenseiIssei)

</div>
