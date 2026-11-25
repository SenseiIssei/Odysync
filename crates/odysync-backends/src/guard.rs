//! The refusals every package manager's `apply` starts with.
//!
//! Every one of these tools installs an exact version, and every one of them
//! would also install a package that is not there, or move one backwards, when
//! asked to. Shared by all backends that drive a command line tool.

use odysync_core::error::{Error, Result};
use odysync_core::model::UpdateCandidate;
use odysync_core::proc::Output;
use odysync_core::version::Version;
use std::cmp::Ordering;

/// A refusal: nothing was run.
pub fn refused(candidate: &UpdateCandidate, detail: impl Into<String>) -> Error {
    Error::Verification {
        package: candidate.id.to_string(),
        detail: detail.into(),
    }
}

/// Check what needs no tool: the id can be an argument, and there is an
/// exact version to pin.
///
/// An id or a version that starts with `-` would be read as an option, and
/// not every tool understands `--` as the end of its options.
pub fn precheck(candidate: &UpdateCandidate) -> Result<()> {
    if !candidate.available.is_known() {
        return Err(refused(
            candidate,
            "refusing to install without an exact target version",
        ));
    }
    candidate.id.validate()?;
    if candidate.id.native.trim_start().starts_with('-') {
        return Err(Error::invalid_package_id(
            &candidate.id.native,
            "package ID starts with '-' and would be read as an option",
        ));
    }
    let version = candidate.available.raw();
    if version.trim_start().starts_with('-') || version.chars().any(char::is_control) {
        return Err(refused(
            candidate,
            "refusing a target version that would be read as an option",
        ));
    }
    Ok(())
}

/// Refuse when the manager no longer offers the version that was approved.
pub fn same_target(candidate: &UpdateCandidate, offered: Option<&str>) -> Result<()> {
    let approved = candidate.available.raw();
    match offered.map(str::trim) {
        Some(now) if now == approved => Ok(()),
        Some(now) => Err(refused(
            candidate,
            format!("{approved} was approved but the source now offers {now}; scan again"),
        )),
        None => Err(refused(
            candidate,
            format!("{approved} was approved but the source no longer offers an update"),
        )),
    }
}

/// Refuse unless what is installed right now is older than the target.
///
/// Several tools install a package that is missing and downgrade one that
/// is newer when they are given an exact version. Both end here.
pub fn still_older(candidate: &UpdateCandidate, current: Option<&str>) -> Result<()> {
    let Some(current) = current else {
        return Err(refused(
            candidate,
            "the installed version can no longer be read; refusing to install from scratch",
        ));
    };
    let installed = candidate.available.parse_like(current);
    if installed.is_upgrade_to(&candidate.available) {
        Ok(())
    } else {
        Err(refused(
            candidate,
            format!(
                "{current} is installed, which is not older than {}",
                candidate.available.raw()
            ),
        ))
    }
}

/// A failed command, with whichever stream carries the message.
pub fn failed(command: impl Into<String>, out: Output) -> Error {
    Error::CommandFailed {
        command: command.into(),
        code: out.code,
        stderr: if out.stderr.trim().is_empty() {
            out.stdout
        } else {
            out.stderr
        },
    }
}

/// The newest of several versions, or `None` when two of them cannot be
/// ordered.
pub fn newest<'a>(
    versions: impl IntoIterator<Item = &'a str>,
    parse: impl Fn(&str) -> Version,
) -> Option<&'a str> {
    let mut best: Option<&'a str> = None;
    for v in versions {
        best = match best {
            None => Some(v),
            Some(b) if b == v => Some(b),
            Some(b) => match parse(b).compare(&parse(v))? {
                Ordering::Less => Some(v),
                _ => Some(b),
            },
        };
    }
    best
}
