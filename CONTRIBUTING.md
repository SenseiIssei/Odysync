# Contributing

Thank you for helping improve Odysync.

## Setup

- Rust 1.90 or newer (`rustup update stable`)
- Node.js 22 or newer, for the app
- On Linux, the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
  (WebKitGTK 4.1 and friends)

```sh
cargo build -p odysync-cli                 # the CLI
cd apps/gui && npm ci && npx tauri dev     # the app
cd apps/gui && npm run dev                 # the UI alone, in a browser, on demo data
```

## Before you open a pull request

CI runs exactly this on Windows, macOS and Linux:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cd apps/gui && npx tsc -b && npx oxlint && npm test
```

## Ground rules

- **Safety rules live in `odysync-core`**, not in a backend or a front-end.
  If a change needs a new rule, it goes there with a test.
- **Read values, not labels.** Console tools translate their output; a parser
  that matches on `Instance ID:` breaks on a German Windows. Prefer JSON, CIM,
  exit codes and identifiers.
- **Test parsers against captured real output**, including Windows line
  endings, and assert counts, not just shapes.
- **Adding an app to the direct-update catalog** (`crates/odysync-backends/src/direct/catalog.json`)
  needs the signer read from the real installer (`Get-AuthenticodeSignature`).
  Publishers who do not sign their installers are not added.
- Comments explain *why*. The code already says what.

## Reporting a problem

`odysync diagnostics --out bundle.zip` collects what is needed; attach it to
an issue, after checking it for anything you would rather not share.
