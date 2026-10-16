# Code signing

Odysync verifies other people's installers and ships unsigned itself. That
asymmetry has been open since `ROADMAP.md` §5 and it is the last thing standing
between the project and a release a stranger can install without being trained
to click through a warning.

This document is the part that can be written down. The rest is procurement,
which has a lead time measured in days and cannot be automated.

---

## Why it matters more than it looks

Without an Authenticode signature, Windows SmartScreen shows *"Windows
protected your PC"* on every download, for every user, forever. Reputation is
per-certificate and per-publisher, so an unsigned binary never accumulates any.

The damage is not the friction. It is that the only way to install Odysync is
to click past the exact warning that protects users from malware — so the
project spends its first interaction teaching people to ignore the thing that
would have saved them. A tool whose pitch is "safe updates" cannot afford that.

It also gates **automatic self-update**. `odysync self-check` reports that a
release exists and refuses to install it, and that refusal is deliberate:
`ARCHITECTURE.md` criticises v1 for fetching a PowerShell module at runtime as
Administrator and trusting whatever answered. Downloading an unsigned
executable and running it elevated would be the same hole with our own name on
it. Signing first, self-update second — not a preference, an ordering.

---

## Which certificate

| Option | Cost | Lead time | SmartScreen |
|---|---|---|---|
| **Azure Trusted Signing** | ~10 USD/month | Days | Immediate reputation, inherits Microsoft's root |
| OV certificate | 200–400 USD/year | Days | Builds reputation slowly; warns until it does |
| EV certificate | 300–500 USD/year | 1–3 weeks, hardware token | Immediate |

**Azure Trusted Signing is the right answer for an individual publisher.** It
is an order of magnitude cheaper than an EV certificate, needs no hardware
token to keep track of, and the signature carries Microsoft-rooted reputation
from the first release. Its one requirement is the awkward part: the identity
must have existed for **three years** for an individual subscriber. If that is
not satisfiable, an OV certificate is the fallback, with the caveat that early
releases will still warn.

### Setting it up

1. Azure subscription → create a **Trusted Signing account** (it is region-bound;
   pick the one nearest your users).
2. Create an **Identity Validation** request. This is the step with the lead
   time — individual validation needs government ID and takes a few days.
3. Once validated, create a **Certificate Profile** of type
   `PublicTrustIdentityValidation`.
4. Register an Entra ID **app registration** for CI, and grant it the
   **Trusted Signing Certificate Profile Signer** role on the account.

---

## Wiring it to CI

`release.yml` already has the signing steps. They are gated on a repository
**variable**, so the workflow keeps working today and starts signing the moment
the values exist — rather than being a separate change somebody has to remember
to make at the same moment the validation clears.

Set one repository variable to switch it on:

| Kind | Name | Value |
|---|---|---|
| Variable | `SIGNING_ENABLED` | `true` |
| Variable | `TRUSTED_SIGNING_ENDPOINT` | `https://<region>.codesigning.azure.net` |
| Variable | `TRUSTED_SIGNING_ACCOUNT` | your account name |
| Variable | `TRUSTED_SIGNING_PROFILE` | your certificate profile name |
| Secret | `AZURE_TENANT_ID` | directory (tenant) id |
| Secret | `AZURE_CLIENT_ID` | application (client) id |
| Secret | `AZURE_CLIENT_SECRET` | client secret |

Both the CLI binaries and the NSIS installer are signed, with an RFC 3161
timestamp so signatures stay valid after the certificate expires.

### Checking it worked

```powershell
Get-AuthenticodeSignature .\Odysync_2.2.0_x64-setup.exe | Format-List Status, SignerCertificate
```

`Status` must be `Valid`. Anything else means the release should not be
published — a broken signature is worse than none, because it looks like
tampering to every tool that checks.

---

## After signing lands

In this order:

1. Publish one signed release and confirm SmartScreen is quiet on a machine
   that has never seen Odysync.
2. Submit the generated winget manifests (`release.yml` builds them into a
   `winget-manifests` artifact) to `microsoft/winget-pkgs`. winget requires a
   signed installer for a reason.
3. Only then revisit `self_update.rs`. `UpdateCheck::is_installable` is the
   single place the decision lives, and the signature check it needs already
   exists in `odysync-verify`. The test
   `odysync_refuses_to_install_its_own_updates_while_releases_are_unsigned`
   fails deliberately at that point — that failure is the reminder to also
   remove this paragraph.

## macOS and Linux

Not started, and lower priority because neither platform has a SmartScreen
equivalent that punishes an unsigned build as hard.

- **macOS**: an Apple Developer ID (99 USD/year), plus notarisation and
  stapling. Gatekeeper refuses unnotarised apps outright, so this is required
  before a macOS GUI release, not merely advisable.
- **Linux**: `.deb`/`.rpm`/AppImage with a signed repository. Distribution
  packaging matters more than per-binary signing there.
