//! Configuration weaknesses: the settings that decide how much damage the next
//! mistake does.
//!
//! Nothing here detects an intrusion. It answers the other question — "if
//! something got in, what did it walk through, and what would it be able to do
//! next?" Firewall off, UAC lowered, SMBv1 still installed, no disk encryption:
//! none of these are compromises, all of them make one cheaper.
//!
//! Several of these queries need administrator rights (BitLocker status, the
//! SMBv1 feature state, some account properties). Unelevated they return
//! nothing, and nothing is reported as "unknown" rather than "fine" — a
//! security tool that reports an absent answer as a pass is worse than one that
//! stays quiet.

use serde::{Deserialize, Serialize};

use odysync_core::error::Result;

use super::{Finding, Remediation, Severity};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FirewallProfile {
    pub name: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalAccount {
    pub name: String,
    pub enabled: Option<bool>,
    /// `false` means the account may have a blank password.
    pub password_required: Option<bool>,
    pub is_administrator: Option<bool>,
    pub last_logon: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionPolicyScope {
    pub scope: String,
    pub policy: String,
}

/// The whole posture query, in one object.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PostureSnapshot {
    pub real_time_protection: Option<bool>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub firewall: Vec<FirewallProfile>,
    /// `EnableLUA`: 0 disables UAC entirely.
    pub enable_lua: Option<i64>,
    /// `ConsentPromptBehaviorAdmin`: 0 means elevate silently, 5 is the default.
    pub consent_prompt_admin: Option<i64>,
    /// `fDenyTSConnections`: 0 means Remote Desktop is allowed.
    pub deny_ts_connections: Option<i64>,
    /// `Enabled`, `Disabled`, or absent when the query needs elevation.
    pub smb1_state: Option<String>,
    pub smb1_server_enabled: Option<bool>,
    /// BitLocker `ProtectionStatus` for the system drive: 1 = protected.
    pub bitlocker_protection: Option<i64>,
    pub bitlocker_volume_status: Option<String>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub execution_policy: Vec<ExecutionPolicyScope>,
    pub pending_reboot: Option<bool>,
    #[serde(default, deserialize_with = "super::ps_collection")]
    pub accounts: Vec<LocalAccount>,
    pub secure_boot: Option<bool>,
}

#[cfg(windows)]
const SNAPSHOT_SCRIPT: &str = r#"$ErrorActionPreference='SilentlyContinue'
$rtp = $null
$mp = Get-MpComputerStatus
if ($mp) { $rtp = [bool]$mp.RealTimeProtectionEnabled }

$fw = @(Get-NetFirewallProfile | Select-Object @{n='name';e={[string]$_.Name}},@{n='enabled';e={[bool]$_.Enabled}})

$sys = Get-ItemProperty -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
$ts  = Get-ItemProperty -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Terminal Server'

$smbState = $null
$f = Get-WindowsOptionalFeature -Online -FeatureName SMB1Protocol
if ($f) { $smbState = [string]$f.State }
$smbServer = $null
$sc = Get-SmbServerConfiguration
if ($sc -ne $null) { $smbServer = [bool]$sc.EnableSMB1Protocol }

$blStatus = $null; $blVolume = $null
$bl = Get-BitLockerVolume -MountPoint $env:SystemDrive
if ($bl) { $blStatus = [int]$bl.ProtectionStatus; $blVolume = [string]$bl.VolumeStatus }

$ep = @(Get-ExecutionPolicy -List | Select-Object @{n='scope';e={[string]$_.Scope}},@{n='policy';e={[string]$_.ExecutionPolicy}})

$pending = $false
if (Test-Path 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending') { $pending = $true }
if (Test-Path 'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired') { $pending = $true }
if ((Get-ItemProperty -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Session Manager').PendingFileRenameOperations) { $pending = $true }

$admins = @()
foreach ($m in (Get-LocalGroupMember -Group 'Administrators')) { $admins += ($m.Name -split '\\')[-1] }
$accounts = @(Get-LocalUser | Select-Object `
  @{n='name';e={[string]$_.Name}}, `
  @{n='enabled';e={[bool]$_.Enabled}}, `
  @{n='passwordRequired';e={[bool]$_.PasswordRequired}}, `
  @{n='isAdministrator';e={[bool]($admins -contains $_.Name)}}, `
  @{n='lastLogon';e={if ($_.LastLogon) { $_.LastLogon.ToString('o') } else { $null }}})

$sb = $null
try { $sb = [bool](Confirm-SecureBootUEFI) } catch { $sb = $null }

[pscustomobject]@{
  realTimeProtection = $rtp
  firewall = $fw
  enableLua = $sys.EnableLUA
  consentPromptAdmin = $sys.ConsentPromptBehaviorAdmin
  denyTsConnections = $ts.fDenyTSConnections
  smb1State = $smbState
  smb1ServerEnabled = $smbServer
  bitlockerProtection = $blStatus
  bitlockerVolumeStatus = $blVolume
  executionPolicy = $ep
  pendingReboot = $pending
  accounts = $accounts
  secureBoot = $sb
} | ConvertTo-Json -Depth 4 -Compress"#;

/// Read the machine's security configuration.
#[cfg(windows)]
pub async fn scan() -> Result<Vec<Finding>> {
    use std::time::Duration;

    let stdout = super::ps_query(SNAPSHOT_SCRIPT, Duration::from_secs(150)).await?;
    let snapshot: PostureSnapshot = super::parse_ps_object(&stdout).unwrap_or_default();
    Ok(analyze(&snapshot))
}

/// Non-Windows stub.
#[cfg(not(windows))]
pub async fn scan() -> Result<Vec<Finding>> {
    Ok(Vec::new())
}

/// Turn configuration into findings. Pure.
pub fn analyze(s: &PostureSnapshot) -> Vec<Finding> {
    let mut out = Vec::new();

    // Defender real-time protection is also checked by the defender section;
    // it is repeated here with a distinct ID so that a machine where the
    // Defender module is missing entirely still gets the warning.
    if s.real_time_protection == Some(false) {
        out.push(Finding::new(
            "posture-realtime-off",
            Severity::Critical,
            "posture",
            "Real-time antivirus protection is off",
            "Files are not being checked as they are created or run.",
        ));
    }

    out.extend(analyze_firewall(&s.firewall));
    out.extend(analyze_uac(s.enable_lua, s.consent_prompt_admin));

    if s.deny_ts_connections == Some(0) {
        out.push(
            Finding::new(
                "posture-rdp-enabled",
                Severity::Medium,
                "posture",
                "Remote Desktop is enabled",
                "This machine accepts Remote Desktop connections. If you do not use \
                 Remote Desktop, turning it off removes an entire class of attack — \
                 exposed RDP is relentlessly scanned for and brute-forced.",
            )
            .with_remediation(Remediation::Manual {
                instructions: "Settings > System > Remote Desktop > Off.".into(),
            }),
        );
    }

    out.extend(analyze_smb1(s.smb1_state.as_deref(), s.smb1_server_enabled));
    out.extend(analyze_bitlocker(
        s.bitlocker_protection,
        s.bitlocker_volume_status.as_deref(),
    ));
    out.extend(analyze_execution_policy(&s.execution_policy));

    if s.pending_reboot == Some(true) {
        out.push(Finding::new(
            "posture-pending-reboot",
            Severity::Low,
            "posture",
            "A restart is pending",
            "Updates have been installed but are not fully in effect until the machine \
             restarts. Security fixes in particular often do nothing until then.",
        ));
    }

    if s.secure_boot == Some(false) {
        out.push(Finding::new(
            "posture-secure-boot-off",
            Severity::Low,
            "posture",
            "Secure Boot is off",
            "Secure Boot stops unsigned code from loading before Windows does. It is \
             commonly turned off for dual-booting or older hardware, so this is only \
             worth acting on if you did not do that deliberately.",
        ));
    }

    out.extend(analyze_accounts(&s.accounts));

    out
}

pub fn analyze_firewall(profiles: &[FirewallProfile]) -> Vec<Finding> {
    let off: Vec<&FirewallProfile> = profiles
        .iter()
        .filter(|p| p.enabled == Some(false))
        .collect();
    if off.is_empty() {
        return Vec::new();
    }
    let names: Vec<String> = off.iter().map(|p| p.name.clone()).collect();
    vec![Finding::new(
        "posture-firewall-disabled",
        Severity::High,
        "posture",
        format!(
            "Windows Firewall is off for the {} profile(s)",
            names.join(", ")
        ),
        "With the firewall off, anything listening on this machine is reachable from \
         the network it is attached to. Malware routinely disables the firewall as a \
         first step, so if you did not turn this off yourself, that is worth taking \
         seriously on its own.",
    )
    .with_evidence(
        profiles
            .iter()
            .map(|p| {
                format!(
                    "{}: {}",
                    p.name,
                    match p.enabled {
                        Some(true) => "enabled",
                        Some(false) => "DISABLED",
                        None => "unknown",
                    }
                )
            })
            .collect::<Vec<_>>(),
    )
    .with_remediation(Remediation::Manual {
        instructions: "Windows Security > Firewall & network protection > turn the \
             firewall on for every profile."
            .into(),
    })]
}

/// UAC: `EnableLUA=0` switches it off entirely, and
/// `ConsentPromptBehaviorAdmin=0` elevates without asking, which means any
/// program you run can silently become administrator.
pub fn analyze_uac(enable_lua: Option<i64>, consent_prompt_admin: Option<i64>) -> Vec<Finding> {
    let mut out = Vec::new();

    if enable_lua == Some(0) {
        out.push(
            Finding::new(
                "posture-uac-disabled",
                Severity::High,
                "posture",
                "User Account Control is turned off",
                "With UAC off, every program you run as an administrator account gets \
                 full administrator rights immediately, with no prompt. Nothing stands \
                 between a double-clicked file and the whole system.",
            )
            .with_remediation(Remediation::Manual {
                instructions: "Search for \"Change User Account Control settings\" and set \
                     the slider to the default (second from top). A restart is required."
                    .into(),
            }),
        );
    } else if consent_prompt_admin == Some(0) {
        out.push(Finding::new(
            "posture-uac-no-prompt",
            Severity::High,
            "posture",
            "UAC elevates without prompting",
            "UAC is enabled, but administrators are elevated silently. In practice this \
             is close to having UAC off: a program that asks for administrator rights \
             simply gets them, and you never see it happen.",
        ));
    }
    // Values 1 and 3 prompt for credentials and 2/4/5 prompt for consent: all
    // are at least as strict as the Windows default, so none are findings.

    out
}

pub fn analyze_smb1(state: Option<&str>, server_enabled: Option<bool>) -> Vec<Finding> {
    let installed = state.is_some_and(|s| s.eq_ignore_ascii_case("Enabled"));
    if !installed && server_enabled != Some(true) {
        return Vec::new();
    }
    let mut evidence = Vec::new();
    if let Some(s) = state {
        evidence.push(format!("SMB1Protocol feature: {s}"));
    }
    if let Some(e) = server_enabled {
        evidence.push(format!("SMB1 server protocol enabled: {e}"));
    }
    vec![Finding::new(
        "posture-smb1-enabled",
        Severity::High,
        "posture",
        "SMBv1 is installed and enabled",
        "SMBv1 is the thirty-year-old file sharing protocol that WannaCry and NotPetya \
         spread through. Microsoft removed it from default installs for that reason. \
         Unless you have a device on your network old enough to require it — some NAS \
         boxes and printers do — turning it off costs nothing.",
    )
    .with_evidence(evidence)
    .with_remediation(Remediation::Manual {
        instructions: "Windows Features > untick \"SMB 1.0/CIFS File Sharing Support\", \
             then restart."
            .into(),
    })]
}

pub fn analyze_bitlocker(protection: Option<i64>, volume_status: Option<&str>) -> Vec<Finding> {
    // Absent means the query could not run (not elevated, or Home edition).
    let Some(protection) = protection else {
        return Vec::new();
    };
    if protection == 1 {
        return Vec::new();
    }
    let mut evidence = vec![format!("ProtectionStatus: {protection}")];
    if let Some(v) = volume_status {
        evidence.push(format!("VolumeStatus: {v}"));
    }
    vec![Finding::new(
        "posture-bitlocker-off",
        Severity::Medium,
        "posture",
        "The system drive is not encrypted",
        "Without disk encryption, anyone who can boot this machine from a USB stick, or \
         who takes the drive out, reads everything on it — including saved passwords and \
         session tokens — without needing your Windows password at all. This matters for \
         a laptop far more than a desktop that never leaves the house.",
    )
    .with_evidence(evidence)
    .with_remediation(Remediation::Manual {
        instructions: "Settings > Privacy & security > Device encryption, or Control \
             Panel > BitLocker Drive Encryption. Save the recovery key somewhere that is \
             not this machine."
            .into(),
    })]
}

pub fn analyze_execution_policy(scopes: &[ExecutionPolicyScope]) -> Vec<Finding> {
    let unrestricted: Vec<&ExecutionPolicyScope> = scopes
        .iter()
        .filter(|s| {
            s.policy.eq_ignore_ascii_case("Unrestricted") || s.policy.eq_ignore_ascii_case("Bypass")
        })
        .collect();
    if unrestricted.is_empty() {
        return Vec::new();
    }
    vec![Finding::new(
        "posture-execution-policy",
        Severity::Low,
        "posture",
        "PowerShell execution policy is unrestricted",
        "The execution policy is a guard rail against accidentally running a downloaded \
         script, not a security boundary — anyone who wants to bypass it can, in one \
         flag. Still, leaving it unrestricted removes the last prompt between a \
         double-clicked .ps1 and it running.",
    )
    .with_evidence(
        unrestricted
            .iter()
            .map(|s| format!("{}: {}", s.scope, s.policy))
            .collect::<Vec<_>>(),
    )]
}

/// Windows' own accounts, which exist on every install and are disabled by
/// default.
const BUILTIN_ACCOUNTS: &[&str] = &[
    "administrator",
    "guest",
    "defaultaccount",
    "wdagutilityaccount",
];

pub fn analyze_accounts(accounts: &[LocalAccount]) -> Vec<Finding> {
    let mut out = Vec::new();

    // Blank passwords: an enabled account with no password required is a free
    // login, and for an administrator account it is a free administrator login.
    let blank: Vec<&LocalAccount> = accounts
        .iter()
        .filter(|a| a.enabled == Some(true) && a.password_required == Some(false))
        .collect();
    if !blank.is_empty() {
        let any_admin = blank.iter().any(|a| a.is_administrator == Some(true));
        out.push(
            Finding::new(
                "account-blank-password",
                if any_admin {
                    Severity::Critical
                } else {
                    Severity::High
                },
                "account",
                format!("{} enabled account(s) may have no password", blank.len()),
                "An account that does not require a password can be logged into by \
                 anyone at the keyboard, and — depending on network settings — \
                 potentially from elsewhere. When such an account is also an \
                 administrator, it is the whole machine.",
            )
            .with_evidence(
                blank
                    .iter()
                    .map(|a| {
                        format!(
                            "{}{}",
                            a.name,
                            if a.is_administrator == Some(true) {
                                " (administrator)"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .with_remediation(Remediation::Manual {
                instructions: "Set a password for the account, or disable it: \
                     Settings > Accounts > Other users."
                    .into(),
            }),
        );
    }

    // The built-in Administrator account being enabled is unusual on a personal
    // machine and is a common post-compromise foothold.
    for a in accounts {
        if a.name.eq_ignore_ascii_case("Administrator") && a.enabled == Some(true) {
            out.push(Finding::new(
                "account-builtin-administrator-enabled",
                Severity::Medium,
                "account",
                "The built-in Administrator account is enabled",
                "Windows disables this account by default. It being enabled is either \
                 something you or an IT department did deliberately, or something an \
                 attacker did to keep a way back in.",
            ));
        }
    }

    // Unexpected administrators.
    let admins: Vec<&LocalAccount> = accounts
        .iter()
        .filter(|a| a.is_administrator == Some(true))
        .collect();
    if !admins.is_empty() {
        let unusual: Vec<&&LocalAccount> = admins
            .iter()
            .filter(|a| !BUILTIN_ACCOUNTS.contains(&a.name.to_ascii_lowercase().as_str()))
            .collect();
        out.push(
            Finding::new(
                "account-administrators",
                Severity::Info,
                "account",
                format!(
                    "{} local account(s) have administrator rights",
                    admins.len()
                ),
                format!(
                    "Every account listed here can change anything on this machine. \
                     {} Any name you do not recognise is a serious finding — creating a \
                     second administrator is how an intruder keeps access after the \
                     original hole is closed.",
                    if unusual.len() > 1 {
                        "More than one non-built-in administrator exists."
                    } else {
                        ""
                    }
                ),
            )
            .with_evidence(
                admins
                    .iter()
                    .map(|a| {
                        format!(
                            "{} ({})",
                            a.name,
                            match a.enabled {
                                Some(true) => "enabled",
                                Some(false) => "disabled",
                                None => "unknown",
                            }
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> PostureSnapshot {
        PostureSnapshot {
            real_time_protection: Some(true),
            firewall: vec![
                FirewallProfile {
                    name: "Domain".into(),
                    enabled: Some(true),
                },
                FirewallProfile {
                    name: "Private".into(),
                    enabled: Some(true),
                },
                FirewallProfile {
                    name: "Public".into(),
                    enabled: Some(true),
                },
            ],
            enable_lua: Some(1),
            consent_prompt_admin: Some(5),
            deny_ts_connections: Some(1),
            smb1_state: Some("Disabled".into()),
            smb1_server_enabled: Some(false),
            bitlocker_protection: Some(1),
            bitlocker_volume_status: Some("FullyEncrypted".into()),
            execution_policy: vec![ExecutionPolicyScope {
                scope: "LocalMachine".into(),
                policy: "RemoteSigned".into(),
            }],
            pending_reboot: Some(false),
            accounts: vec![
                LocalAccount {
                    name: "bob".into(),
                    enabled: Some(true),
                    password_required: Some(true),
                    is_administrator: Some(true),
                    last_logon: None,
                },
                LocalAccount {
                    name: "Administrator".into(),
                    enabled: Some(false),
                    password_required: Some(true),
                    is_administrator: Some(true),
                    last_logon: None,
                },
            ],
            secure_boot: Some(true),
        }
    }

    #[test]
    fn a_healthy_machine_reports_only_the_administrator_inventory() {
        let f = analyze(&healthy());
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "account-administrators");
        assert_eq!(f[0].severity, Severity::Info);
    }

    #[test]
    fn an_empty_snapshot_reports_nothing() {
        // Every query failed. Silence, not a clean bill of health — the scan
        // report's own `incomplete` flag is what tells the user.
        assert!(analyze(&PostureSnapshot::default()).is_empty());
    }

    #[test]
    fn disabled_firewall_profiles_are_named() {
        let profiles = vec![
            FirewallProfile {
                name: "Domain".into(),
                enabled: Some(true),
            },
            FirewallProfile {
                name: "Public".into(),
                enabled: Some(false),
            },
        ];
        let f = analyze_firewall(&profiles);
        assert_eq!(f[0].severity, Severity::High);
        assert!(f[0].title.contains("Public"));
        assert!(!f[0].title.contains("Domain"));
        // Evidence still lists every profile so the user sees the whole picture.
        assert_eq!(f[0].evidence.len(), 2);
    }

    #[test]
    fn uac_disabled_and_silent_elevation_are_distinguished() {
        assert_eq!(analyze_uac(Some(0), Some(5))[0].id, "posture-uac-disabled");
        assert_eq!(analyze_uac(Some(1), Some(0))[0].id, "posture-uac-no-prompt");
        // Default and stricter-than-default configurations are silent.
        assert!(analyze_uac(Some(1), Some(5)).is_empty());
        assert!(analyze_uac(Some(1), Some(2)).is_empty());
        assert!(analyze_uac(Some(1), Some(1)).is_empty());
        assert!(analyze_uac(None, None).is_empty());
    }

    #[test]
    fn smb1_is_reported_from_either_signal() {
        assert_eq!(analyze_smb1(Some("Enabled"), None).len(), 1);
        assert_eq!(analyze_smb1(None, Some(true)).len(), 1);
        assert!(analyze_smb1(Some("Disabled"), Some(false)).is_empty());
        // Query failed entirely: report nothing rather than a false pass.
        assert!(analyze_smb1(None, None).is_empty());
    }

    #[test]
    fn bitlocker_off_is_reported_but_an_unreadable_status_is_not() {
        assert_eq!(analyze_bitlocker(Some(0), Some("FullyDecrypted")).len(), 1);
        assert!(analyze_bitlocker(Some(1), Some("FullyEncrypted")).is_empty());
        // Unelevated / Home edition: no answer, no finding, no false comfort.
        assert!(analyze_bitlocker(None, None).is_empty());
    }

    #[test]
    fn unrestricted_execution_policy_is_low() {
        let scopes = vec![
            ExecutionPolicyScope {
                scope: "MachinePolicy".into(),
                policy: "Undefined".into(),
            },
            ExecutionPolicyScope {
                scope: "CurrentUser".into(),
                policy: "Bypass".into(),
            },
        ];
        let f = analyze_execution_policy(&scopes);
        assert_eq!(f[0].severity, Severity::Low);
        assert_eq!(f[0].evidence, vec!["CurrentUser: Bypass"]);
    }

    #[test]
    fn a_blank_password_on_an_admin_account_is_critical() {
        let accounts = vec![LocalAccount {
            name: "test".into(),
            enabled: Some(true),
            password_required: Some(false),
            is_administrator: Some(true),
            last_logon: None,
        }];
        let f = analyze_accounts(&accounts);
        assert_eq!(f[0].id, "account-blank-password");
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(f[0].category, "account");
        assert!(f[0].evidence[0].contains("administrator"));
    }

    #[test]
    fn a_blank_password_on_a_standard_account_is_high() {
        let accounts = vec![LocalAccount {
            name: "kiosk".into(),
            enabled: Some(true),
            password_required: Some(false),
            is_administrator: Some(false),
            last_logon: None,
        }];
        assert_eq!(analyze_accounts(&accounts)[0].severity, Severity::High);
    }

    #[test]
    fn a_disabled_account_without_a_password_is_not_reported() {
        let accounts = vec![LocalAccount {
            name: "Guest".into(),
            enabled: Some(false),
            password_required: Some(false),
            is_administrator: Some(false),
            last_logon: None,
        }];
        assert!(analyze_accounts(&accounts).is_empty());
    }

    #[test]
    fn the_enabled_builtin_administrator_is_reported() {
        let accounts = vec![LocalAccount {
            name: "Administrator".into(),
            enabled: Some(true),
            password_required: Some(true),
            is_administrator: Some(true),
            last_logon: None,
        }];
        let ids: Vec<String> = analyze_accounts(&accounts)
            .into_iter()
            .map(|f| f.id)
            .collect();
        assert!(ids
            .iter()
            .any(|i| i == "account-builtin-administrator-enabled"));
        assert!(ids.iter().any(|i| i == "account-administrators"));
    }

    #[test]
    fn a_realistic_posture_payload_deserializes_and_classifies() {
        let json = r#"{"realTimeProtection":true,
          "firewall":[{"name":"Domain","enabled":true},{"name":"Private","enabled":true},{"name":"Public","enabled":false}],
          "enableLua":1,"consentPromptAdmin":5,"denyTsConnections":0,
          "smb1State":"Enabled","smb1ServerEnabled":true,
          "bitlockerProtection":0,"bitlockerVolumeStatus":"FullyDecrypted",
          "executionPolicy":[{"scope":"LocalMachine","policy":"Undefined"}],
          "pendingReboot":true,
          "accounts":[{"name":"bob","enabled":true,"passwordRequired":true,"isAdministrator":true,"lastLogon":"2026-07-21T08:00:00.0000000+02:00"}],
          "secureBoot":true}"#;
        let s: PostureSnapshot = crate::security::parse_ps_object(json).unwrap();
        let ids: Vec<String> = analyze(&s).iter().map(|f| f.id.clone()).collect();
        for expected in [
            "posture-firewall-disabled",
            "posture-rdp-enabled",
            "posture-smb1-enabled",
            "posture-bitlocker-off",
            "posture-pending-reboot",
            "account-administrators",
        ] {
            assert!(ids.contains(&expected.to_string()), "missing {expected}");
        }
        assert!(!ids.contains(&"posture-uac-disabled".to_string()));
    }
}
