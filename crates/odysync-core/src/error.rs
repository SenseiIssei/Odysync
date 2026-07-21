//! Error type shared across the workspace.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// The backend's underlying tool is not installed or not on PATH.
    #[error("{backend} is not available on this system: {detail}")]
    BackendUnavailable { backend: String, detail: String },

    /// A spawned process returned a non-zero exit code.
    #[error("{command} exited with code {code}: {stderr}")]
    CommandFailed {
        command: String,
        code: i32,
        stderr: String,
    },

    /// A spawned process exceeded its time budget and was killed.
    #[error("{command} timed out after {seconds}s")]
    CommandTimeout { command: String, seconds: u64 },

    /// Output from a package manager did not match any known shape.
    #[error("could not parse {what}: {detail}")]
    Parse { what: String, detail: String },

    /// An integrity or signature check failed; nothing was installed.
    #[error("verification failed for {package}: {detail}")]
    Verification { package: String, detail: String },

    /// A transient error that may succeed on retry (network, lock, etc.).
    #[error("transient error in {backend}: {detail}")]
    Transient {
        backend: String,
        detail: String,
        /// How many times this error has been retried so far.
        attempt: u32,
    },

    /// A security violation was detected (path traversal, injection, etc.).
    #[error("security violation in {context}: {detail}")]
    SecurityViolation { context: String, detail: String },

    /// A package ID failed validation (contains shell metacharacters, etc.).
    #[error("invalid package ID '{id}': {detail}")]
    InvalidPackageId { id: String, detail: String },

    /// A pre-update health check failed (disk space, AC power, etc.).
    #[error("health check failed: {check}: {detail}")]
    HealthCheckFailed { check: String, detail: String },

    #[error("configuration error: {0}")]
    Config(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub fn parse(what: impl Into<String>, detail: impl Into<String>) -> Self {
        Error::Parse {
            what: what.into(),
            detail: detail.into(),
        }
    }

    pub fn unavailable(backend: impl Into<String>, detail: impl Into<String>) -> Self {
        Error::BackendUnavailable {
            backend: backend.into(),
            detail: detail.into(),
        }
    }

    pub fn transient(
        backend: impl Into<String>,
        detail: impl Into<String>,
        attempt: u32,
    ) -> Self {
        Error::Transient {
            backend: backend.into(),
            detail: detail.into(),
            attempt,
        }
    }

    pub fn security(context: impl Into<String>, detail: impl Into<String>) -> Self {
        Error::SecurityViolation {
            context: context.into(),
            detail: detail.into(),
        }
    }

    pub fn invalid_package_id(id: impl Into<String>, detail: impl Into<String>) -> Self {
        Error::InvalidPackageId {
            id: id.into(),
            detail: detail.into(),
        }
    }

    pub fn health_check_failed(
        check: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Error::HealthCheckFailed {
            check: check.into(),
            detail: detail.into(),
        }
    }

    /// Whether this error is worth retrying.
    ///
    /// Transient errors and timeouts are retryable. Verification errors,
    /// security violations, and invalid package IDs are never retryable.
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Transient { .. } => true,
            Error::CommandTimeout { .. } => true,
            Error::CommandFailed { code, .. } => {
                *code == 11 || *code == 143
            }
            Error::Io(e) => {
                matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                )
            }
            _ => false,
        }
    }

    /// Sanitize sensitive information from the error message for safe display.
    ///
    /// Redacts file system paths, IP addresses, and environment variable
    /// values that may leak through stderr from subprocesses.
    pub fn sanitize(&self) -> String {
        let raw = self.to_string();
        sanitize_text(&raw)
    }
}

/// Sanitize text by redacting sensitive patterns.
fn sanitize_text(text: &str) -> String {
    let mut result = text.to_string();

    // Redact Windows paths: C:\Users\username\...
    if cfg!(windows) {
        if let Some(userprofile) = std::env::var_os("USERPROFILE") {
            let up = userprofile.to_string_lossy();
            if !up.is_empty() {
                result = result.replace(&*up, "%USERPROFILE%");
            }
        }
        let path_re = regex::Regex::new(
            r"[A-Za-z]:\\Users\\[^\\]+\\",
        ).unwrap_or_else(|_| regex::Regex::new(r"").unwrap());
        result = path_re.replace_all(&result, "C:\\Users\\***\\").to_string();
    }

    // Redact Unix home paths: /home/username, /Users/username
    if let Some(home) = std::env::var_os("HOME") {
        let h = home.to_string_lossy();
        if !h.is_empty() {
            result = result.replace(&*h, "$HOME");
        }
    }

    // Redact IP addresses
    let ip_re = regex::Regex::new(
        r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b",
    ).unwrap_or_else(|_| regex::Regex::new(r"").unwrap());
    result = ip_re.replace_all(&result, "[IP REDACTED]").to_string();

    // Redact environment variable values that look like tokens
    let token_re = regex::Regex::new(
        r"(?i)(token|key|secret|password|api[_-]?key)\s*[=:]\s*\S+",
    ).unwrap_or_else(|_| regex::Regex::new(r"").unwrap());
    result = token_re.replace_all(&result, "$1=[REDACTED]").to_string();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_errors_are_retryable() {
        let e = Error::transient("apt", "could not acquire lock", 0);
        assert!(e.is_retryable());
    }

    #[test]
    fn timeouts_are_retryable() {
        let e = Error::CommandTimeout {
            command: "dnf check-update".into(),
            seconds: 300,
        };
        assert!(e.is_retryable());
    }

    #[test]
    fn verification_errors_are_not_retryable() {
        let e = Error::Verification {
            package: "test".into(),
            detail: "checksum mismatch".into(),
        };
        assert!(!e.is_retryable());
    }

    #[test]
    fn security_violations_are_not_retryable() {
        let e = Error::security("path validation", "path traversal detected");
        assert!(!e.is_retryable());
    }

    #[test]
    fn invalid_package_ids_are_not_retryable() {
        let e = Error::invalid_package_id("test; rm -rf /", "shell metacharacters");
        assert!(!e.is_retryable());
    }

    #[test]
    fn sanitize_redacts_ip_addresses() {
        let e = Error::CommandFailed {
            command: "curl".into(),
            code: 1,
            stderr: "Failed to connect to 192.168.1.100".into(),
        };
        let sanitized = e.sanitize();
        assert!(!sanitized.contains("192.168.1.100"));
        assert!(sanitized.contains("[IP REDACTED]"));
    }

    #[test]
    fn sanitize_redacts_token_values() {
        let e = Error::CommandFailed {
            command: "test".into(),
            code: 1,
            stderr: "api_key=abc123secret".into(),
        };
        let sanitized = e.sanitize();
        assert!(!sanitized.contains("abc123secret"));
        assert!(sanitized.contains("[REDACTED]"));
    }

    #[test]
    fn sanitize_redacts_home_paths() {
        std::env::set_var("HOME", "/home/testuser");
        let e = Error::CommandFailed {
            command: "test".into(),
            code: 1,
            stderr: "error in /home/testuser/file".into(),
        };
        let sanitized = e.sanitize();
        assert!(!sanitized.contains("/home/testuser"));
        assert!(sanitized.contains("$HOME"));
    }

    #[test]
    fn helper_constructors_work() {
        let e = Error::transient("dnf", "lock error", 1);
        assert!(matches!(e, Error::Transient { attempt: 1, .. }));

        let e = Error::security("proc", "path traversal");
        assert!(matches!(e, Error::SecurityViolation { .. }));

        let e = Error::invalid_package_id("bad;id", "metacharacters");
        assert!(matches!(e, Error::InvalidPackageId { .. }));

        let e = Error::health_check_failed("disk", "low space");
        assert!(matches!(e, Error::HealthCheckFailed { .. }));
    }
}
