//! Per-platform code-signature verification.
//!
//! We shell out to the platform's own verifier rather than reimplementing PKI:
//! Authenticode on Windows and `codesign` on macOS both consult the live system
//! trust store, so revoked certificates and expired chains are caught without
//! us shipping and refreshing a root bundle.
//!
//! Linux has no equivalent per-binary notion — packages are signed at the
//! repository level and `apt`/`dnf`/`pacman` verify that themselves — so the
//! status there is [`SignatureStatus::Unsupported`] and integrity rests on the
//! package manager's own GPG checks.

use std::path::Path;
use std::time::Duration;

/// Outcome of checking a file's code signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureStatus {
    /// Signed, and the platform trusts the chain right now.
    Valid { subject: String },
    /// Carries no signature at all.
    Unsigned,
    /// Signed, but the signature does not validate (expired, revoked,
    /// tampered, untrusted root).
    Invalid { detail: String },
    /// This platform has no per-file signature model.
    Unsupported,
}

/// Timeout for the verifier itself. Certificate revocation checks hit the
/// network, so this is generous — but bounded, because a captive portal can
/// otherwise stall it indefinitely.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(45);

#[cfg(windows)]
pub async fn verify_signature(path: &Path) -> SignatureStatus {
    use odysync_core::proc;

    // Get-AuthenticodeSignature returns a Status enum; NotSigned and Valid are
    // the two we can act on, everything else is a failed validation.
    // We print Status and SignerCertificate subject on separate lines.
    let script = format!(
        r#"$ErrorActionPreference='Stop'
try {{
  $s = Get-AuthenticodeSignature -LiteralPath '{}'
  Write-Output ("STATUS=" + $s.Status)
  if ($s.SignerCertificate) {{ Write-Output ("SUBJECT=" + $s.SignerCertificate.Subject) }}
}} catch {{
  Write-Output ("STATUS=Error")
  Write-Output ("SUBJECT=" + $_.Exception.Message)
}}"#,
        // Escape single quotes for the PowerShell literal string.
        path.display().to_string().replace('\'', "''")
    );

    let out = match proc::powershell(&script, VERIFY_TIMEOUT).await {
        Ok(o) => o,
        Err(e) => {
            return SignatureStatus::Invalid {
                detail: format!("could not run Authenticode check: {e}"),
            }
        }
    };

    parse_authenticode(&out.stdout)
}

/// Parse the `STATUS=`/`SUBJECT=` pair emitted by the PowerShell snippet.
#[cfg(windows)]
fn parse_authenticode(stdout: &str) -> SignatureStatus {
    let mut status = "";
    let mut subject = String::new();

    for line in stdout.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("STATUS=") {
            status = v.trim();
        } else if let Some(v) = line.strip_prefix("SUBJECT=") {
            subject = v.trim().to_string();
        }
    }

    match status {
        "Valid" => SignatureStatus::Valid {
            subject: if subject.is_empty() {
                "unknown signer".into()
            } else {
                subject
            },
        },
        "NotSigned" => SignatureStatus::Unsigned,
        "" => SignatureStatus::Invalid {
            detail: "Authenticode check produced no status".into(),
        },
        other => SignatureStatus::Invalid {
            detail: format!("Authenticode status: {other}"),
        },
    }
}

#[cfg(target_os = "macos")]
pub async fn verify_signature(path: &Path) -> SignatureStatus {
    use odysync_core::proc;

    let path_str = path.display().to_string();
    let out = match proc::run(
        "codesign",
        &["--verify", "--deep", "--strict", "-vv", &path_str],
        VERIFY_TIMEOUT,
    )
    .await
    {
        Ok(o) => o,
        Err(e) => {
            return SignatureStatus::Invalid {
                detail: format!("could not run codesign: {e}"),
            }
        }
    };

    // codesign writes its findings to stderr even on success.
    let text = format!("{}{}", out.stdout, out.stderr);

    if out.success() {
        SignatureStatus::Valid {
            subject: text
                .lines()
                .find_map(|l| l.trim().strip_prefix("Authority=").map(str::to_string))
                .unwrap_or_else(|| "unknown signer".into()),
        }
    } else if text.contains("code object is not signed") {
        SignatureStatus::Unsigned
    } else {
        SignatureStatus::Invalid {
            detail: text.trim().lines().next().unwrap_or("invalid").to_string(),
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub async fn verify_signature(_path: &Path) -> SignatureStatus {
    // Linux package managers verify repository signatures themselves.
    SignatureStatus::Unsupported
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn valid_status_is_parsed_with_its_subject() {
        let out = "STATUS=Valid\nSUBJECT=CN=Mozilla Corporation, O=Mozilla\n";
        assert_eq!(
            parse_authenticode(out),
            SignatureStatus::Valid {
                subject: "CN=Mozilla Corporation, O=Mozilla".into()
            }
        );
    }

    #[test]
    fn unsigned_is_distinguished_from_invalid() {
        assert_eq!(
            parse_authenticode("STATUS=NotSigned\n"),
            SignatureStatus::Unsigned
        );
        assert!(matches!(
            parse_authenticode("STATUS=HashMismatch\n"),
            SignatureStatus::Invalid { .. }
        ));
    }

    #[test]
    fn a_tampered_binary_reports_invalid_not_unsigned() {
        // HashMismatch means the file changed after signing — the exact case
        // that must never be treated as merely "unsigned".
        assert!(matches!(
            parse_authenticode("STATUS=HashMismatch\nSUBJECT=CN=Someone\n"),
            SignatureStatus::Invalid { .. }
        ));
    }

    #[test]
    fn empty_output_is_invalid_rather_than_valid() {
        assert!(matches!(
            parse_authenticode(""),
            SignatureStatus::Invalid { .. }
        ));
    }
}
