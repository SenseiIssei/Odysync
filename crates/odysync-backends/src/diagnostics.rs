//! Diagnostics bundle creation.
//!
//! Collects environment info, backend outputs, config, and the latest run
//! report into a zip file. No data is uploaded anywhere — this is purely
//! for the user to inspect or share manually.

use std::io::Write;
use std::path::Path;

use odysync_core::config::Config;
use odysync_core::error::Result;
use odysync_core::platform;
use odysync_core::proc;
use odysync_core::report::RunReport;

/// Create a diagnostics zip bundle at `out_path`.
///
/// Contents:
///   - `env.txt` — OS, odysync version, elevation status
///   - `config.json` — the resolved config
///   - `report.json` / `report.txt` — the last run report (if provided)
///   - `backends.txt` — list of detected backends
///   - Platform-specific command outputs (winget --version, etc.)
pub async fn create_diagnostics(
    out_path: &Path,
    config: &Config,
    report: Option<&RunReport>,
) -> Result<()> {
    let file = std::fs::File::create(out_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // env.txt
    let env_text = format!(
        "Odysync diagnostics\n\
         OS: {}\n\
         Elevated: {}\n\
         Generated: {}\n",
        platform::os_label(),
        platform::is_elevated(),
        chrono::Utc::now(),
    );
    zip.start_file("env.txt", options)
        .map_err(|e| odysync_core::error::Error::Io(std::io::Error::other(e.to_string())))?;
    zip.write_all(env_text.as_bytes())?;

    // config.json
    let cfg_text = serde_json::to_string_pretty(config)?;
    zip.start_file("config.json", options)
        .map_err(|e| odysync_core::error::Error::Io(std::io::Error::other(e.to_string())))?;
    zip.write_all(cfg_text.as_bytes())?;

    // report
    if let Some(report) = report {
        zip.start_file("report.json", options)
            .map_err(|e| odysync_core::error::Error::Io(std::io::Error::other(e.to_string())))?;
        zip.write_all(serde_json::to_string_pretty(report)?.as_bytes())?;
        zip.start_file("report.txt", options)
            .map_err(|e| odysync_core::error::Error::Io(std::io::Error::other(e.to_string())))?;
        zip.write_all(report.to_text().as_bytes())?;
    }

    // Platform-specific command outputs
    let commands: Vec<(&str, Vec<&str>)> = if cfg!(windows) {
        vec![
            ("winget_version.txt", vec!["winget", "--version"]),
            ("winget_upgrade.txt", vec!["winget", "upgrade"]),
            ("winget_list.txt", vec!["winget", "list"]),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            ("brew_version.txt", vec!["brew", "--version"]),
            ("brew_outdated.txt", vec!["brew", "outdated"]),
        ]
    } else {
        vec![
            ("apt_version.txt", vec!["apt-get", "--version"]),
            ("flatpak_version.txt", vec!["flatpak", "--version"]),
        ]
    };

    for (filename, cmd) in commands {
        let program = cmd[0];
        let args = &cmd[1..];
        let output = proc::run(
            program,
            args,
            std::time::Duration::from_secs(60),
        )
        .await;

        let text = match output {
            Ok(o) => format!(
                "$ {} {}\nexit: {}\n\n--- stdout ---\n{}\n--- stderr ---\n{}",
                program,
                args.join(" "),
                o.code,
                o.stdout,
                o.stderr,
            ),
            Err(e) => format!("$ {} {}\nerror: {e}", program, args.join(" ")),
        };

        zip.start_file(filename, options)
            .map_err(|e| odysync_core::error::Error::Io(std::io::Error::other(e.to_string())))?;
        zip.write_all(text.as_bytes())?;
    }

    zip.finish()
        .map_err(|e| odysync_core::error::Error::Io(std::io::Error::other(e.to_string())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use odysync_core::config::Config;

    #[tokio::test]
    async fn creates_a_valid_zip() {
        let dir = std::env::temp_dir().join("odysync-diag-test");
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("diag.zip");

        let cfg = Config::default();
        create_diagnostics(&zip_path, &cfg, None)
            .await
            .unwrap();

        assert!(zip_path.exists());
        assert!(zip_path.metadata().unwrap().len() > 0);

        // Verify it's a valid zip by reading it back.
        let file = std::fs::File::open(&zip_path).unwrap();
        let archive = zip::ZipArchive::new(file).unwrap();
        assert!(archive.len() >= 2); // env.txt + config.json at minimum

        std::fs::remove_dir_all(&dir).ok();
    }
}
