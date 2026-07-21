//! Microsoft Defender integration.
//!
//! Writing another signature engine would be absurd: the machine already has a
//! kernel-integrated, continuously-updated one. This section reads its state
//! (`Get-MpComputerStatus`), reads what it has already caught
//! (`Get-MpThreat` / `Get-MpThreatDetection`), and drives it
//! (`Start-MpScan`, `Update-MpSignature`, `Remove-MpThreat`).
//!
//! Limits worth knowing: if a third-party AV is installed, Defender drops to
//! passive mode and most of these values become uninteresting or absent, and
//! the whole `Defender` PowerShell module is missing on Windows Server core
//! installs without the feature. Both cases surface as a failed section rather
//! than a false "all clear".

#[cfg(windows)]
use std::time::Duration;

use serde::{Deserialize, Serialize};

use odysync_core::error::{Error, Result};

use super::{Finding, Severity};

/// How stale signatures may get before it is worth telling the user.
const SIGNATURE_AGE_WARN_DAYS: i64 = 7;

/// Read-only status query. Fast — one WMI round trip.
#[cfg(windows)]
const QUERY_TIMEOUT: Duration = Duration::from_secs(90);

/// The subset of `Get-MpComputerStatus` this module reasons about.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DefenderStatus {
    pub real_time_protection_enabled: Option<bool>,
    pub antivirus_enabled: Option<bool>,
    #[serde(rename = "AMServiceEnabled")]
    pub am_service_enabled: Option<bool>,
    pub behavior_monitor_enabled: Option<bool>,
    #[serde(rename = "IoavProtectionEnabled")]
    pub ioav_protection_enabled: Option<bool>,
    pub is_tamper_protected: Option<bool>,
    pub antivirus_signature_age: Option<i64>,
    pub antispyware_signature_age: Option<i64>,
    pub antivirus_signature_last_updated: Option<String>,
    pub quick_scan_age: Option<i64>,
    pub full_scan_age: Option<i64>,
    pub antivirus_signature_version: Option<String>,
}

/// One row of `Get-MpThreat`: a threat Defender knows about on this machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ThreatRow {
    #[serde(rename = "ThreatID")]
    pub threat_id: Option<u64>,
    pub threat_name: Option<String>,
    /// Defender's own scale: 1 Low, 2 Moderate, 3 High, 4 Severe, 5 Unknown.
    #[serde(rename = "SeverityID")]
    pub severity_id: Option<i64>,
    pub is_active: Option<bool>,
    pub did_threat_execute: Option<bool>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub resources: Vec<String>,
}

/// One row of `Get-MpThreatDetection`: an individual detection event.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DetectionRow {
    #[serde(rename = "ThreatID")]
    pub threat_id: Option<u64>,
    pub detection_time: Option<String>,
    pub action_success: Option<bool>,
    /// 1 unknown, 2 quarantined, 3 removed, ... 105 cleaned, 106 allowed.
    #[serde(rename = "ThreatStatusID")]
    pub threat_status_id: Option<i64>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub resources: Vec<String>,
}

/// Everything one PowerShell round trip collects.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DefenderSnapshot {
    pub status: Option<DefenderStatus>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub threats: Vec<ThreatRow>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub detections: Vec<DetectionRow>,
}

/// Everything is fetched in one process: three separate PowerShell spawns cost
/// well over a second on a cold machine, and these queries are independent.
#[cfg(windows)]
const SNAPSHOT_SCRIPT: &str = r#"$ErrorActionPreference='SilentlyContinue'
$status = Get-MpComputerStatus | Select-Object -First 1 RealTimeProtectionEnabled,AntivirusEnabled,AMServiceEnabled,BehaviorMonitorEnabled,IoavProtectionEnabled,IsTamperProtected,AntivirusSignatureAge,AntispywareSignatureAge,QuickScanAge,FullScanAge,AntivirusSignatureVersion,@{n='AntivirusSignatureLastUpdated';e={if($_.AntivirusSignatureLastUpdated){$_.AntivirusSignatureLastUpdated.ToString('o')}else{$null}}}
$threats = @(Get-MpThreat | Select-Object ThreatID,ThreatName,SeverityID,IsActive,DidThreatExecute,@{n='Resources';e={@($_.Resources)}})
$detections = @(Get-MpThreatDetection | Select-Object ThreatID,ActionSuccess,ThreatStatusID,@{n='DetectionTime';e={if($_.InitialDetectionTime){$_.InitialDetectionTime.ToString('o')}else{$null}}},@{n='Resources';e={@($_.Resources)}})
[pscustomobject]@{ Status = $status; Threats = $threats; Detections = $detections } | ConvertTo-Json -Depth 5 -Compress"#;

/// Read Defender's state and everything it has flagged, unanalysed.
///
/// Exposed separately from [`scan`] so a UI can show live protection state
/// without paying for, or waiting on, the full finding analysis.
#[cfg(windows)]
pub async fn snapshot() -> Result<DefenderSnapshot> {
    let stdout = super::ps_query(SNAPSHOT_SCRIPT, QUERY_TIMEOUT).await?;
    super::parse_ps_object(&stdout).ok_or_else(|| {
        Error::parse(
            "Get-MpComputerStatus",
            "the Defender PowerShell module returned nothing parseable; \
             it may be absent (Server core) or superseded by a third-party AV",
        )
    })
}

/// Non-Windows stub: Defender does not exist elsewhere.
#[cfg(not(windows))]
pub async fn snapshot() -> Result<DefenderSnapshot> {
    Ok(DefenderSnapshot::default())
}

/// Read Defender's state and everything it has flagged.
#[cfg(windows)]
pub async fn scan() -> Result<Vec<Finding>> {
    Ok(analyze(&snapshot().await?))
}

/// Non-Windows stub: Defender does not exist elsewhere.
#[cfg(not(windows))]
pub async fn scan() -> Result<Vec<Finding>> {
    Ok(Vec::new())
}

/// Turn a snapshot into findings. Pure — this is where all the judgement is.
pub fn analyze(snapshot: &DefenderSnapshot) -> Vec<Finding> {
    let mut findings = Vec::new();

    if let Some(status) = &snapshot.status {
        findings.extend(analyze_status(status));
    }

    // Detections carry the timestamps and the on-disk paths; threats carry the
    // name and severity. Join them so one finding says both what it is and
    // where it was seen.
    for threat in &snapshot.threats {
        findings.push(threat_finding(threat, &snapshot.detections));
    }

    findings
}

/// Configuration findings from `Get-MpComputerStatus`.
pub fn analyze_status(s: &DefenderStatus) -> Vec<Finding> {
    let mut out = Vec::new();

    if s.real_time_protection_enabled == Some(false) {
        out.push(
            Finding::new(
                "defender-realtime-off",
                Severity::Critical,
                "posture",
                "Defender real-time protection is off",
                "Nothing is inspecting files as they are written or executed. \
                 Malware disabling real-time protection is one of the first things \
                 a stealer does after it runs, so if you did not turn this off \
                 yourself, treat it as evidence of compromise rather than a \
                 setting to flip back.",
            )
            .with_remediation(super::Remediation::Manual {
                instructions: "Windows Security > Virus & threat protection > \
                     Manage settings > Real-time protection > On. If it refuses to \
                     stay on, or the toggle is greyed out by policy, assume the \
                     machine is compromised and rebuild it."
                    .into(),
            }),
        );
    }

    if s.antivirus_enabled == Some(false) || s.am_service_enabled == Some(false) {
        out.push(Finding::new(
            "defender-service-off",
            Severity::High,
            "posture",
            "The Defender antimalware service is not running",
            "Defender's service is stopped or disabled. This is normal if you \
             deliberately installed a third-party antivirus, and alarming if you \
             did not.",
        ));
    }

    if s.behavior_monitor_enabled == Some(false) {
        out.push(Finding::new(
            "defender-behavior-monitor-off",
            Severity::High,
            "posture",
            "Defender behaviour monitoring is off",
            "Behaviour monitoring catches malicious activity that signatures miss. \
             Turning it off is a common step in a manual bypass.",
        ));
    }

    if s.ioav_protection_enabled == Some(false) {
        out.push(Finding::new(
            "defender-ioav-off",
            Severity::Medium,
            "posture",
            "Scanning of downloads and attachments is off",
            "Files arriving from the browser and mail clients are no longer \
             scanned on arrival.",
        ));
    }

    if s.is_tamper_protected == Some(false) {
        out.push(
            Finding::new(
                "defender-tamper-protection-off",
                Severity::High,
                "posture",
                "Tamper protection is off",
                "Tamper protection is what stops malware from simply switching \
                 Defender off through the registry. Without it, every other \
                 Defender setting is advisory.",
            )
            .with_remediation(super::Remediation::Manual {
                instructions: "Windows Security > Virus & threat protection > \
                     Manage settings > Tamper Protection > On."
                    .into(),
            }),
        );
    }

    let age = s
        .antivirus_signature_age
        .into_iter()
        .chain(s.antispyware_signature_age)
        .max();
    if let Some(age) = age {
        if age > SIGNATURE_AGE_WARN_DAYS {
            let mut evidence = vec![format!("signature age: {age} day(s)")];
            if let Some(v) = &s.antivirus_signature_version {
                evidence.push(format!("version: {v}"));
            }
            if let Some(t) = &s.antivirus_signature_last_updated {
                evidence.push(format!("last updated: {t}"));
            }
            out.push(
                Finding::new(
                    "defender-signatures-stale",
                    if age > 30 {
                        Severity::High
                    } else {
                        Severity::Medium
                    },
                    "posture",
                    format!("Defender signatures are {age} days old"),
                    "Definitions older than a week miss recent malware families, \
                     and stealer kits are rebuilt constantly. Signature updates \
                     that stop arriving can also mean update traffic is being \
                     blocked on purpose.",
                )
                .with_evidence(evidence),
            );
        }
    }

    if let Some(q) = s.quick_scan_age {
        if q > 14 {
            out.push(Finding::new(
                "defender-no-recent-scan",
                Severity::Low,
                "posture",
                format!("No Defender scan in {q} days"),
                "A scheduled scan has not completed recently, so anything sitting \
                 dormant on disk may never have been looked at.",
            ));
        }
    }

    out
}

/// Build the finding for one known threat, folding in its detection events.
fn threat_finding(threat: &ThreatRow, detections: &[DetectionRow]) -> Finding {
    let name = threat
        .threat_name
        .clone()
        .unwrap_or_else(|| "Unknown threat".to_string());
    let severity = severity_from_defender_row(
        threat.severity_id,
        threat.is_active,
        threat.did_threat_execute,
    );

    let mut evidence: Vec<String> = Vec::new();
    for r in &threat.resources {
        evidence.push(r.clone());
    }

    let mine: Vec<&DetectionRow> = detections
        .iter()
        .filter(|d| d.threat_id.is_some() && d.threat_id == threat.threat_id)
        .collect();
    for d in &mine {
        if let Some(t) = &d.detection_time {
            evidence.push(format!("detected: {t}"));
        }
        for r in &d.resources {
            if !evidence.contains(r) {
                evidence.push(r.clone());
            }
        }
        evidence.push(format!(
            "status: {}",
            threat_status_label(d.threat_status_id)
        ));
    }

    let executed = threat.did_threat_execute == Some(true);
    let active = threat.is_active == Some(true);

    let detail = if active {
        format!(
            "Defender has flagged {name} and it is still marked active — the file \
             was not successfully removed. Remove it now, then change every \
             password that was stored on or typed into this machine."
        )
    } else if executed {
        format!(
            "Defender caught {name}, but it had already run at least once. \
             Cleaning the file does not undo what it did while it was running: \
             assume anything it could reach — session tokens, saved passwords, \
             browser cookies — is in someone else's hands."
        )
    } else {
        format!(
            "Defender detected and handled {name}. No action is required beyond \
             confirming it is gone, but its presence tells you how something got \
             onto the machine."
        )
    };

    let mut finding = Finding::new(
        format!("defender-threat:{name}"),
        severity,
        "malware",
        format!("Defender detection: {name}"),
        detail,
    )
    .with_evidence(evidence);

    // Only offer removal for a threat Defender still considers active. Offering
    // "Fix" on one it has already quarantined contradicted the finding's own
    // text ("no action is required") and, because Defender has no per-threat
    // removal, the button could only ever act on everything or fail.
    match (active, threat.threat_id) {
        (true, Some(id)) => {
            finding = finding.with_remediation(super::Remediation::RemoveDefenderThreat {
                threat_id: id.to_string(),
            });
        }
        (false, _) => {
            finding = finding.with_remediation(super::Remediation::Manual {
                instructions: "Nothing to remove — Defender already dealt with this. \
                     It is listed so you know it happened. If you want certainty that \
                     nothing survived, run a Microsoft Defender Offline scan."
                    .into(),
            });
        }
        _ => {}
    }
    finding
}

/// Map Defender's `SeverityID` onto ours, escalating anything still active.
///
/// An "active" threat means Defender knows about a file it did not manage to
/// remove, which is materially worse than the same family caught cleanly.
/// Severity for one `Get-MpThreat` row.
///
/// `Get-MpThreat` is a *history*, not a live threat list: it keeps every
/// detection Defender has ever made on the machine, indefinitely. Scoring those
/// by their original danger meant a PUA bundler quarantined in 2024 sat at the
/// top of the page as a Critical alongside anything happening now, and no
/// amount of cleaning could ever remove it.
///
/// So the live state decides:
///   * still active — Defender has not dealt with it — Critical, always.
///   * handled, but it ran before being caught — kept visible, because
///     quarantine does not undo what already executed (credential theft in
///     particular).
///   * handled and never executed — history. Demoted to `Info`, which puts it
///     under "Watching" instead of the threat list.
pub fn severity_from_defender(severity_id: Option<i64>, is_active: Option<bool>) -> Severity {
    severity_from_defender_row(severity_id, is_active, None)
}

/// As [`severity_from_defender`], but able to see whether the threat executed.
pub fn severity_from_defender_row(
    severity_id: Option<i64>,
    is_active: Option<bool>,
    did_execute: Option<bool>,
) -> Severity {
    let base = match severity_id {
        Some(4) | Some(5) => Severity::Critical,
        Some(3) => Severity::High,
        Some(2) => Severity::Medium,
        Some(1) => Severity::Low,
        _ => Severity::Medium,
    };

    if is_active == Some(true) {
        return base.escalate(Severity::Critical);
    }

    // Only a *positive* "not active" means Defender dealt with it. An absent
    // field means we could not tell, and unknown must never be read as safe.
    if is_active != Some(false) {
        return base;
    }

    if did_execute == Some(true) {
        // Quarantined, but it ran first. Deliberately *not* demoted: whatever
        // it stole or installed happened before Defender caught it, and cleanup
        // does not undo that. This is the case that explains a stolen token.
        return base;
    }

    // Caught before it ran and already dealt with: history, not a live threat.
    Severity::Info
}

/// Human label for `ThreatStatusID`.
pub fn threat_status_label(id: Option<i64>) -> &'static str {
    match id {
        Some(1) => "unknown",
        Some(2) => "quarantined",
        Some(3) => "removed",
        Some(4) => "cleaned",
        Some(5) => "allowed",
        Some(102) => "no action taken",
        Some(103) => "cleaned",
        Some(104) => "quarantined",
        Some(105) => "removed",
        Some(106) => "allowed by the user",
        Some(107) => "detection blocked",
        _ => "unreported",
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// A quick scan touches memory, autostart locations and the usual drop
/// directories. Minutes, not hours.
#[cfg(windows)]
const QUICK_SCAN_TIMEOUT: Duration = Duration::from_secs(60 * 30);

/// A full scan walks every file on every fixed drive. On a large spinning disk
/// this genuinely takes hours, so the budget is deliberately enormous — the
/// alternative is killing the scan halfway and reporting a lie.
#[cfg(windows)]
const FULL_SCAN_TIMEOUT: Duration = Duration::from_secs(60 * 60 * 2);

/// Start a Defender quick scan and wait for it to finish (up to 30 minutes).
#[cfg(windows)]
pub async fn quick_scan() -> Result<String> {
    super::ps_mutate(
        "Start-MpScan -ScanType QuickScan",
        "$ErrorActionPreference='Stop'; Start-MpScan -ScanType QuickScan; 'ok'",
        QUICK_SCAN_TIMEOUT,
    )
    .await?;
    Ok("Quick scan finished. Re-run the security scan to see any new detections.".to_string())
}

/// Start a Defender full scan and wait for it to finish.
///
/// **This blocks for as long as the scan runs — up to two hours.** Call it from
/// a background task, never from a UI thread, and expect it to be cancelled by
/// the user long before it times out.
#[cfg(windows)]
pub async fn full_scan() -> Result<String> {
    super::ps_mutate(
        "Start-MpScan -ScanType FullScan",
        "$ErrorActionPreference='Stop'; Start-MpScan -ScanType FullScan; 'ok'",
        FULL_SCAN_TIMEOUT,
    )
    .await?;
    Ok("Full scan finished. Re-run the security scan to see any new detections.".to_string())
}

/// Scan one file or directory with Defender.
#[cfg(windows)]
pub async fn scan_path(path: &str) -> Result<String> {
    validate_scan_path(path)?;
    let script = format!(
        "$ErrorActionPreference='Stop'; Start-MpScan -ScanType CustomScan -ScanPath {}; 'ok'",
        super::ps_quote(path)
    );
    super::ps_mutate(
        "Start-MpScan -ScanType CustomScan",
        &script,
        QUICK_SCAN_TIMEOUT,
    )
    .await?;
    Ok(format!("Defender scanned {path}."))
}

/// Pull down the latest definitions.
#[cfg(windows)]
pub async fn update_signatures() -> Result<String> {
    super::ps_mutate(
        "Update-MpSignature",
        "$ErrorActionPreference='Stop'; Update-MpSignature; 'ok'",
        Duration::from_secs(60 * 15),
    )
    .await?;
    Ok("Defender signatures updated.".to_string())
}

/// Ask Defender to remove a threat it has already identified.
#[cfg(windows)]
/// Ask Defender to remediate every threat it currently considers active.
///
/// `Remove-MpThreat` takes **no threat selector** — there is no `-ThreatID`
/// parameter, and passing one fails with `NamedParameterNotFound`. Defender
/// exposes no per-threat removal at all, so this is all-or-nothing by design of
/// the API, not by choice. `threat_id` is still validated and logged so the
/// caller's intent is recorded, and the returned message says plainly that the
/// action was not scoped to one threat.
pub async fn remove_threat(threat_id: &str) -> Result<String> {
    let id = validate_threat_id(threat_id)?;
    tracing::info!(
        threat_id = id,
        "asking Defender to remediate active threats"
    );

    let script = "$ErrorActionPreference='Stop'; Remove-MpThreat; 'ok'";
    super::ps_mutate("Remove-MpThreat", script, Duration::from_secs(60 * 20)).await?;

    Ok(
        "Asked Defender to remediate every threat it still considers active. \
         Defender offers no way to act on a single threat, so this covers all of \
         them. Re-run the audit to confirm."
            .to_string(),
    )
}

#[cfg(not(windows))]
pub async fn quick_scan() -> Result<String> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub async fn full_scan() -> Result<String> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub async fn scan_path(_path: &str) -> Result<String> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub async fn update_signatures() -> Result<String> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub async fn remove_threat(_threat_id: &str) -> Result<String> {
    Err(unsupported())
}

#[cfg(not(windows))]
fn unsupported() -> Error {
    Error::unavailable(
        "Microsoft Defender",
        "this action is only available on Windows",
    )
}

/// `Remove-MpThreat -ThreatID` takes an integer. Accepting anything else would
/// mean interpolating an attacker-influenced string into a script, so the ID is
/// parsed rather than quoted.
pub fn validate_threat_id(id: &str) -> Result<u64> {
    id.trim().parse::<u64>().map_err(|_| {
        Error::security(
            "Remove-MpThreat",
            format!("threat ID must be a positive integer, got {id:?}"),
        )
    })
}

/// Reject scan paths that are obviously not paths.
///
/// The value is single-quoted before interpolation regardless; this catches the
/// caller passing something nonsensical and turns it into a clear error.
pub fn validate_scan_path(path: &str) -> Result<()> {
    let p = path.trim();
    if p.is_empty() {
        return Err(Error::security("Start-MpScan", "scan path is empty"));
    }
    if p.contains('\0') || p.contains('\n') || p.contains('\r') {
        return Err(Error::security(
            "Start-MpScan",
            "scan path contains a control character",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> DefenderStatus {
        DefenderStatus {
            real_time_protection_enabled: Some(true),
            antivirus_enabled: Some(true),
            am_service_enabled: Some(true),
            behavior_monitor_enabled: Some(true),
            ioav_protection_enabled: Some(true),
            is_tamper_protected: Some(true),
            antivirus_signature_age: Some(0),
            antispyware_signature_age: Some(0),
            antivirus_signature_last_updated: Some("2026-07-21T04:00:00.0000000Z".into()),
            quick_scan_age: Some(1),
            full_scan_age: Some(9),
            antivirus_signature_version: Some("1.415.44.0".into()),
        }
    }

    #[test]
    fn a_healthy_defender_produces_no_findings() {
        assert!(analyze_status(&status()).is_empty());
    }

    #[test]
    fn real_time_protection_off_is_critical() {
        let s = DefenderStatus {
            real_time_protection_enabled: Some(false),
            ..status()
        };
        let f = analyze_status(&s);
        assert_eq!(f[0].id, "defender-realtime-off");
        assert_eq!(f[0].severity, Severity::Critical);
    }

    #[test]
    fn stale_signatures_escalate_with_age() {
        let week = DefenderStatus {
            antivirus_signature_age: Some(9),
            antispyware_signature_age: Some(3),
            ..status()
        };
        let f = analyze_status(&week);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Medium);
        assert!(f[0].title.contains("9 days old"));
        // Evidence must carry the version so a user can compare with Microsoft's.
        assert!(f[0].evidence.iter().any(|e| e.contains("1.415.44.0")));

        let month = DefenderStatus {
            antivirus_signature_age: Some(45),
            ..week
        };
        assert_eq!(analyze_status(&month)[0].severity, Severity::High);

        // Exactly at the threshold is still fine.
        let edge = DefenderStatus {
            antivirus_signature_age: Some(7),
            antispyware_signature_age: Some(7),
            ..status()
        };
        assert!(analyze_status(&edge).is_empty());
    }

    #[test]
    fn tamper_protection_off_is_reported_with_manual_instructions() {
        let s = DefenderStatus {
            is_tamper_protected: Some(false),
            ..status()
        };
        let f = analyze_status(&s);
        assert_eq!(f[0].id, "defender-tamper-protection-off");
        assert!(matches!(
            f[0].remediation,
            Some(super::super::Remediation::Manual { .. })
        ));
    }

    #[test]
    fn missing_fields_are_never_treated_as_a_problem() {
        // Third-party AV installed: most fields come back null. Silence is the
        // right answer — inventing findings from absent data trains users to
        // ignore the tool.
        assert!(analyze_status(&DefenderStatus::default()).is_empty());
    }

    #[test]
    fn an_active_threat_is_critical_and_offers_removal() {
        let snapshot = DefenderSnapshot {
            status: None,
            threats: vec![ThreatRow {
                threat_id: Some(2147735503),
                threat_name: Some("Trojan:Win32/Wacatac.B!ml".into()),
                severity_id: Some(3),
                is_active: Some(true),
                did_threat_execute: Some(true),
                resources: vec![r"file:_C:\Users\bob\AppData\Local\Temp\setup.exe".into()],
            }],
            detections: vec![DetectionRow {
                threat_id: Some(2147735503),
                detection_time: Some("2026-07-20T23:14:02.0000000Z".into()),
                action_success: Some(false),
                threat_status_id: Some(102),
                resources: vec![r"file:_C:\Users\bob\AppData\Local\Temp\setup.exe".into()],
            }],
        };

        let f = analyze(&snapshot);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "defender-threat:Trojan:Win32/Wacatac.B!ml");
        // SeverityID 3 is High, but still-active drags it to Critical.
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(f[0].category, "malware");
        assert!(f[0].evidence.iter().any(|e| e.contains("setup.exe")));
        assert!(f[0].evidence.iter().any(|e| e.contains("no action taken")));
        assert_eq!(
            f[0].remediation,
            Some(super::super::Remediation::RemoveDefenderThreat {
                threat_id: "2147735503".into()
            })
        );
        // The path must not be duplicated just because it appears in both rows.
        let hits = f[0]
            .evidence
            .iter()
            .filter(|e| e.contains("setup.exe"))
            .count();
        assert_eq!(hits, 1);
    }

    #[test]
    fn a_handled_threat_that_executed_warns_about_stolen_credentials() {
        let snapshot = DefenderSnapshot {
            status: None,
            threats: vec![ThreatRow {
                threat_id: Some(1),
                threat_name: Some("PWS:MSIL/Umbral".into()),
                severity_id: Some(4),
                is_active: Some(false),
                did_threat_execute: Some(true),
                resources: vec![],
            }],
            detections: vec![],
        };
        let f = analyze(&snapshot);
        assert_eq!(f[0].severity, Severity::Critical);
        assert!(f[0].detail.contains("already run"));
    }

    #[test]
    fn defender_severity_mapping() {
        // Handled and never executed: history, not a live threat, whatever
        // Defender's original severity was.
        assert_eq!(severity_from_defender(Some(1), Some(false)), Severity::Info);
        assert_eq!(severity_from_defender(Some(4), Some(false)), Severity::Info);

        // Unknown state is not assumed to be safe.
        assert_eq!(severity_from_defender(Some(5), None), Severity::Critical);

        // Handled, but it ran before being caught: keeps its full severity.
        // Quarantine does not undo credential theft.
        assert_eq!(
            severity_from_defender_row(Some(4), Some(false), Some(true)),
            Severity::Critical
        );
        // Unknown severity should not silently become "Low".
        assert_eq!(severity_from_defender(None, None), Severity::Medium);
        assert_eq!(
            severity_from_defender(Some(1), Some(true)),
            Severity::Critical
        );
    }

    /// Regression, from a real 58 KB `Get-MpThreat` payload.
    ///
    /// PowerShell emits a collection in three different shapes in the *same*
    /// response: `null` when empty, a normal array, and — once `ConvertTo-Json`
    /// hits its `-Depth` limit — `{"value":[…],"Count":n}`. Only the array was
    /// handled, so a single wrapped `Resources` 1,648 characters in failed the
    /// entire snapshot. The user was told Defender was "absent or superseded by
    /// a third-party AV" on a machine whose Defender was perfectly healthy, and
    /// the whole malware section of the audit went dark.
    #[test]
    fn every_powershell_collection_shape_parses() {
        let json = r#"{"Status":{"RealTimeProtectionEnabled":true},
          "Threats":[
            {"ThreatID":224054,"ThreatName":"PUA:Win32/IObit","SeverityID":1,
             "IsActive":false,"DidThreatExecute":false,"Resources":null},
            {"ThreatID":2147966054,"ThreatName":"TrojanDropper:Win64/Convagent.AHB!MTB",
             "SeverityID":5,"IsActive":true,"DidThreatExecute":true,
             "Resources":["file:_C:\\Users\\bob\\Downloads\\x.exe"]}],
          "Detections":[
            {"ThreatID":245560,"ThreatStatusID":106,
             "Resources":{"value":["file:_C:\\a.rar","webfile:_C:\\a.rar|https://x"],"Count":2}},
            {"ThreatID":224054,"ThreatStatusID":2,"Resources":null}]}"#;

        let snapshot: DefenderSnapshot =
            serde_json::from_str(json).expect("all three collection shapes must parse");

        assert_eq!(snapshot.threats.len(), 2);
        assert!(
            snapshot.threats[0].resources.is_empty(),
            "null becomes empty"
        );
        assert_eq!(
            snapshot.threats[1].resources.len(),
            1,
            "plain arrays survive"
        );
        assert_eq!(
            snapshot.detections[0].resources.len(),
            2,
            "the depth-limit {{value,Count}} wrapper is unwrapped"
        );
        assert!(snapshot.detections[1].resources.is_empty());

        // And the analysis still reaches the dangerous row behind the odd ones.
        let findings = analyze(&snapshot);
        assert!(
            findings
                .iter()
                .any(|f| f.title.contains("Convagent") && f.severity == Severity::Critical),
            "an active threat that executed must still surface"
        );
    }

    #[test]
    fn a_real_get_mpcomputerstatus_payload_deserializes() {
        // Trimmed but otherwise verbatim shape from PowerShell 5.1.
        let json = r#"{"Status":{"RealTimeProtectionEnabled":true,"AntivirusEnabled":true,
          "AMServiceEnabled":true,"BehaviorMonitorEnabled":false,"IoavProtectionEnabled":true,
          "IsTamperProtected":true,"AntivirusSignatureAge":12,"AntispywareSignatureAge":12,
          "QuickScanAge":2,"FullScanAge":null,"AntivirusSignatureVersion":"1.415.44.0",
          "AntivirusSignatureLastUpdated":"2026-07-09T04:00:00.0000000Z"},
          "Threats":[],"Detections":[]}"#;
        let snapshot: DefenderSnapshot = super::super::parse_ps_object(json).unwrap();
        let s = snapshot.status.as_ref().unwrap();
        assert_eq!(s.am_service_enabled, Some(true));
        assert_eq!(s.behavior_monitor_enabled, Some(false));
        assert_eq!(s.antivirus_signature_age, Some(12));

        let ids: Vec<String> = analyze(&snapshot).into_iter().map(|f| f.id).collect();
        assert!(ids.iter().any(|i| i == "defender-behavior-monitor-off"));
        assert!(ids.iter().any(|i| i == "defender-signatures-stale"));
    }

    #[test]
    fn a_single_threat_still_arrives_as_an_array() {
        // ConvertTo-Json would emit a bare object for one row; the script wraps
        // the pipeline in @() precisely so this stays an array.
        let json = r#"{"Status":null,"Threats":[{"ThreatID":2147735503,
          "ThreatName":"Trojan:Win32/Wacatac.B!ml","SeverityID":5,"IsActive":false,
          "DidThreatExecute":false,"Resources":["file:_C:\\x.exe"]}],"Detections":[]}"#;
        let snapshot: DefenderSnapshot = super::super::parse_ps_object(json).unwrap();
        assert_eq!(snapshot.threats.len(), 1);
        // Handled (`IsActive:false`) and never executed, so it is history and
        // belongs under "Watching" rather than on the live threat list.
        assert_eq!(analyze(&snapshot)[0].severity, Severity::Info);
    }

    #[test]
    fn threat_ids_must_be_integers() {
        assert_eq!(validate_threat_id(" 2147735503 ").unwrap(), 2147735503);
        for bad in ["1; Remove-Item C:\\", "-1", "abc", "", "1 2"] {
            assert!(
                validate_threat_id(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn scan_paths_reject_control_characters() {
        assert!(validate_scan_path(r"C:\Users\bob\Downloads").is_ok());
        assert!(validate_scan_path("").is_err());
        assert!(validate_scan_path("C:\\a\nStart-Process calc").is_err());
    }
}
