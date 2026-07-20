//! Integrity and authenticity checks applied before an installer is executed.
//!
//! Two independent questions, deliberately kept separate:
//!
//!   * **Integrity** — is this the exact file the package manifest promised?
//!     Answered by comparing SHA-256 against a manifest-supplied digest.
//!   * **Authenticity** — was this file signed by someone, and is that
//!     signature currently valid? Answered per-platform (Authenticode on
//!     Windows, codesign/Gatekeeper on macOS).
//!
//! A hash match with no signature is still meaningful (the manifest vouched for
//! it). A valid signature with a hash *mismatch* is not — the file is not what
//! we were promised, so a mismatch always wins.

use std::path::Path;

use odysync_core::error::{Error, Result};
use sha2::{Digest, Sha256};

mod signature;

pub use signature::{verify_signature, SignatureStatus};

/// The combined result of verifying one installer file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub hash_checked: bool,
    pub signature: SignatureStatus,
}

impl Verdict {
    /// Whether it is safe to execute this installer under `require_signature`.
    pub fn is_acceptable(&self, require_signature: bool) -> bool {
        match self.signature {
            SignatureStatus::Valid { .. } => true,
            // Unsupported platform: fall back to the hash check alone.
            SignatureStatus::Unsupported => self.hash_checked || !require_signature,
            SignatureStatus::Unsigned | SignatureStatus::Invalid { .. } => !require_signature,
        }
    }
}

/// Compute the SHA-256 of a file, streaming so large installers do not have to
/// fit in memory.
pub async fn sha256_file(path: &Path) -> Result<String> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    // 64 KiB balances syscall count against peak memory; an 800 MB installer
    // still costs us only this buffer.
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Check a file against an expected SHA-256 digest.
///
/// Comparison is case-insensitive on the hex text but otherwise exact.
pub async fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let expected = expected.trim();
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Verification {
            package: path.display().to_string(),
            detail: format!("expected digest is not a SHA-256 hex string: {expected:?}"),
        });
    }

    let actual = sha256_file(path).await?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(Error::Verification {
            package: path.display().to_string(),
            detail: format!("SHA-256 mismatch: expected {expected}, got {actual}"),
        })
    }
}

/// Full pre-execution check for an installer.
///
/// `expected_sha256` is optional because not every backend can supply a digest;
/// when it is absent we rely on the signature, and when `require_signature` is
/// set and neither is available the file is rejected.
pub async fn verify_installer(
    path: &Path,
    expected_sha256: Option<&str>,
    require_signature: bool,
) -> Result<Verdict> {
    if !path.exists() {
        return Err(Error::Verification {
            package: path.display().to_string(),
            detail: "installer file does not exist".into(),
        });
    }

    // Integrity first: if the bytes are wrong, nothing else matters.
    let hash_checked = match expected_sha256 {
        Some(expected) => {
            verify_sha256(path, expected).await?;
            true
        }
        None => false,
    };

    let signature = verify_signature(path).await;

    let verdict = Verdict {
        hash_checked,
        signature,
    };

    if !verdict.is_acceptable(require_signature) {
        return Err(Error::Verification {
            package: path.display().to_string(),
            detail: format!(
                "refusing to execute: signature status is {:?} and no trusted digest was \
                 available. Disable require-signature only if you trust this source.",
                verdict.signature
            ),
        });
    }

    Ok(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_file(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("odysync-verify-tests");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join(name);
        tokio::fs::write(&path, contents).await.unwrap();
        path
    }

    #[tokio::test]
    async fn hashes_a_known_vector() {
        let path = temp_file("abc.bin", b"abc").await;
        // The canonical SHA-256 of "abc".
        assert_eq!(
            sha256_file(&path).await.unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn accepts_a_matching_digest_in_either_case() {
        let path = temp_file("match.bin", b"abc").await;
        let upper = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        assert!(verify_sha256(&path, upper).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_a_mismatched_digest() {
        let path = temp_file("mismatch.bin", b"tampered").await;
        let err = verify_sha256(&path, &"a".repeat(64)).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }

    #[tokio::test]
    async fn rejects_a_malformed_expected_digest() {
        let path = temp_file("malformed.bin", b"abc").await;
        // Too short, and not hex — a manifest bug must not silently pass.
        assert!(verify_sha256(&path, "deadbeef").await.is_err());
        assert!(verify_sha256(&path, &"z".repeat(64)).await.is_err());
    }

    #[tokio::test]
    async fn missing_file_is_rejected() {
        let path = std::env::temp_dir().join("odysync-verify-tests/definitely-absent.bin");
        assert!(verify_installer(&path, None, false).await.is_err());
    }

    #[test]
    fn unsigned_is_acceptable_only_when_signatures_are_not_required() {
        let v = Verdict {
            hash_checked: true,
            signature: SignatureStatus::Unsigned,
        };
        assert!(v.is_acceptable(false));
        assert!(!v.is_acceptable(true));
    }

    #[test]
    fn an_invalid_signature_is_never_acceptable_when_required() {
        let v = Verdict {
            hash_checked: true,
            signature: SignatureStatus::Invalid {
                detail: "chain broken".into(),
            },
        };
        assert!(!v.is_acceptable(true));
    }

    #[test]
    fn a_valid_signature_is_acceptable_without_a_digest() {
        let v = Verdict {
            hash_checked: false,
            signature: SignatureStatus::Valid {
                subject: "CN=Mozilla Corporation".into(),
            },
        };
        assert!(v.is_acceptable(true));
    }
}
