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
cargo test --workspace --lib --bins
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
- **Never write a system's constant from memory.** Import it, or assert in a
  test that it equals the imported one. Windows Update's result codes were off
  by two for four releases.
- **`apply` asks again.** It installs exactly the approved version or refuses,
  starts with `guard::precheck`, and never runs an installer a second time.
- **A source that cannot work is not available.** "Found nothing" and "cannot
  tell" must never look the same.
- **Tests never touch the machine they run on.** One that has to (a scheduled
  task, a real install) is `#[ignore]`d and refuses to run outside CI.
- Comments explain *why*. The code already says what.

## Reporting a problem

`odysync diagnostics --out bundle.zip` collects what is needed; attach it to
an issue, after checking it for anything you would rather not share.
