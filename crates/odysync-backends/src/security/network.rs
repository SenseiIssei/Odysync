//! What is listening, what is connected, and which program owns it.
//!
//! A backdoor has to be reachable, and a stealer has to send its haul
//! somewhere, so the socket table is one of the few places where both leave a
//! mark regardless of how they persist. `Get-NetTCPConnection` is joined to the
//! owning process and then to Authenticode, because "port 4444 is open" is
//! useless on its own and "port 4444 is open, owned by an unsigned executable
//! in `%TEMP%`" is not.
//!
//! Limits: this is a snapshot, not a capture. Malware that connects once a day
//! will not be here unless the scan happens to run at the right moment, and UDP
//! is not covered at all (`Get-NetUDPEndpoint` has no state to filter on, so it
//! is nearly all noise). Absence of findings here means very little.

use serde::{Deserialize, Serialize};

use odysync_core::error::Result;

use super::{Finding, Remediation, Severity, TrustMap};

/// Cap on inventory evidence lines.
const MAX_INVENTORY_EVIDENCE: usize = 40;

/// One TCP endpoint, joined to its owning process.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Connection {
    pub local_address: String,
    pub local_port: u32,
    pub remote_address: Option<String>,
    pub remote_port: Option<u32>,
    /// `Listen` or `Established`.
    pub state: String,
    pub owning_process: Option<u32>,
    pub process_name: Option<String>,
    pub process_path: Option<String>,
}

impl Connection {
    pub fn is_listener(&self) -> bool {
        self.state.eq_ignore_ascii_case("Listen")
    }

    pub fn is_established(&self) -> bool {
        self.state.eq_ignore_ascii_case("Established")
    }

    /// How the endpoint should read in a report.
    pub fn describe(&self) -> String {
        let owner = match (&self.process_name, self.owning_process) {
            (Some(n), Some(pid)) => format!("{n} (pid {pid})"),
            (Some(n), None) => n.clone(),
            (None, Some(pid)) => format!("pid {pid}"),
            (None, None) => "unknown process".to_string(),
        };
        if self.is_listener() {
            format!(
                "listening on {}:{} — {owner}",
                self.local_address, self.local_port
            )
        } else {
            format!(
                "{}:{} -> {}:{} — {owner}",
                self.local_address,
                self.local_port,
                self.remote_address.as_deref().unwrap_or("?"),
                self.remote_port.unwrap_or(0),
            )
        }
    }
}

/// The RDP port. Called out by name because an exposed RDP service is the
/// single most common way a Windows machine gets taken over remotely.
pub const RDP_PORT: u32 = 3389;

/// Ports where a listener is expected on an ordinary Windows machine.
const EXPECTED_PORTS: &[u32] = &[
    80,    // http
    135,   // RPC endpoint mapper
    139,   // NetBIOS session
    443,   // https
    445,   // SMB
    500,   // IKE
    515,   // printing
    554,   // RTSP (media sharing)
    902,   // VMware
    1900,  // SSDP
    2179,  // Hyper-V VMConnect
    2869,  // UPnP
    3587,  // peer networking
    4500,  // IPsec NAT-T
    5040,  // Windows connected devices
    5050,  // Windows connected devices
    5353,  // mDNS
    5355,  // LLMNR
    5357,  // WSD
    5985,  // WinRM http
    5986,  // WinRM https
    7680,  // Delivery Optimization
    8080,  // common dev server
    27036, // Steam
];

/// Everything at or above this is a dynamic/ephemeral port. Windows allocates
/// RPC services here on every boot, so a listener in this range is unremarkable.
const EPHEMERAL_START: u32 = 49152;

/// One PowerShell call for the whole table; `Get-Process` is snapshotted into a
/// hash table first so the join is O(n) rather than a lookup per connection.
#[cfg(windows)]
const SNAPSHOT_SCRIPT: &str = r#"$ErrorActionPreference='SilentlyContinue'
$procs = @{}
foreach ($p in (Get-Process)) { $procs[[int]$p.Id] = $p }
$out = foreach ($c in (Get-NetTCPConnection)) {
  $st = [string]$c.State
  if ($st -eq 'Listen' -or $st -eq 'Established') {
    $p = $procs[[int]$c.OwningProcess]
    [pscustomobject]@{
      localAddress = [string]$c.LocalAddress
      localPort = [int]$c.LocalPort
      remoteAddress = [string]$c.RemoteAddress
      remotePort = [int]$c.RemotePort
      state = $st
      owningProcess = [int]$c.OwningProcess
      processName = $(if ($p) { [string]$p.ProcessName } else { $null })
      processPath = $(if ($p) { [string]$p.Path } else { $null })
    }
  }
}
@($out) | ConvertTo-Json -Depth 3 -Compress"#;

/// Read the TCP table and classify it.
#[cfg(windows)]
pub async fn scan() -> Result<Vec<Finding>> {
    use std::time::Duration;

    let stdout = super::ps_query(SNAPSHOT_SCRIPT, Duration::from_secs(120)).await?;
    let conns: Vec<Connection> = super::parse_ps_json(&stdout);

    let paths: Vec<String> = conns
        .iter()
        .filter_map(|c| c.process_path.clone())
        .filter(|p| !p.trim().is_empty())
        .collect();
    let trust = super::query_file_trust(&paths).await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "signature check failed; continuing without it");
        TrustMap::new()
    });

    Ok(analyze(&conns, &trust))
}

/// Non-Windows stub.
#[cfg(not(windows))]
pub async fn scan() -> Result<Vec<Finding>> {
    Ok(Vec::new())
}

/// True for a bind to every interface, i.e. reachable from the network rather
/// than only from this machine.
pub fn is_wildcard_address(addr: &str) -> bool {
    matches!(addr.trim(), "0.0.0.0" | "::" | "[::]" | "*")
}

pub fn is_loopback(addr: &str) -> bool {
    let a = addr.trim().trim_matches(['[', ']']);
    a == "::1" || a.starts_with("127.")
}

/// True for addresses that cannot be routed across the internet.
pub fn is_private_address(addr: &str) -> bool {
    let a = addr.trim().trim_matches(['[', ']']).to_ascii_lowercase();
    if is_loopback(&a) || a == "0.0.0.0" || a == "::" {
        return true;
    }
    if a.starts_with("10.") || a.starts_with("192.168.") || a.starts_with("169.254.") {
        return true;
    }
    if a.starts_with("fe80:") || a.starts_with("fc") || a.starts_with("fd") {
        return true;
    }
    // 172.16.0.0/12
    if let Some(rest) = a.strip_prefix("172.") {
        if let Some(second) = rest.split('.').next() {
            if let Ok(n) = second.parse::<u32>() {
                return (16..=31).contains(&n);
            }
        }
    }
    false
}

/// True when a listener on this port is unremarkable.
pub fn is_expected_port(port: u32) -> bool {
    EXPECTED_PORTS.contains(&port) || port >= EPHEMERAL_START
}

/// Whether the owning process is one to worry about, and why.
fn process_concerns(conn: &Connection, trust: &TrustMap) -> Vec<String> {
    let mut reasons = Vec::new();
    let Some(path) = conn
        .process_path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
    else {
        // A missing path usually just means the process is protected and this
        // scan is not elevated, so it is not evidence of anything.
        return reasons;
    };
    if let Some(t) = super::trust_of(trust, path) {
        if t.unsigned() {
            reasons.push("the program that owns it is not digitally signed".to_string());
        }
    }
    if super::is_user_writable_dir(path) {
        reasons.push(
            "it runs from a directory that any program can write to, so the \
             executable can be swapped without administrator rights"
                .to_string(),
        );
    }
    reasons
}

/// Classify the TCP table.
pub fn analyze(conns: &[Connection], trust: &TrustMap) -> Vec<Finding> {
    let mut out = Vec::new();

    // --- RDP -------------------------------------------------------------
    if let Some(rdp) = conns
        .iter()
        .find(|c| c.is_listener() && c.local_port == RDP_PORT)
    {
        let exposed = is_wildcard_address(&rdp.local_address);
        out.push(
            Finding::new(
                "network-rdp-listening",
                if exposed {
                    Severity::High
                } else {
                    Severity::Medium
                },
                "network",
                "Remote Desktop is accepting connections",
                if exposed {
                    "RDP is listening on every network interface. Internet-exposed RDP is \
                     the most heavily brute-forced service on Windows, and a working \
                     password is all an attacker needs — no malware involved. If you do \
                     not use Remote Desktop, turn it off."
                } else {
                    "Remote Desktop is enabled. That is fine if you use it, but it is \
                     worth confirming that you meant to, and that the firewall does not \
                     allow it from outside your own network."
                },
            )
            .with_evidence(vec![rdp.describe()])
            .with_remediation(Remediation::Manual {
                instructions: "Settings > System > Remote Desktop > Off, if you do not use \
                     it. If you do, restrict it to your local network in Windows Firewall \
                     and require Network Level Authentication."
                    .into(),
            }),
        );
    }

    // --- Unexpected listeners --------------------------------------------
    let mut seen_listeners = std::collections::HashSet::new();
    for c in conns.iter().filter(|c| c.is_listener()) {
        if c.local_port == RDP_PORT {
            continue; // already reported above
        }
        let key = (
            c.local_port,
            c.process_name.clone().unwrap_or_default(),
            is_wildcard_address(&c.local_address),
        );
        if !seen_listeners.insert(key) {
            // IPv4 and IPv6 bindings of the same service are one finding.
            continue;
        }

        let wildcard = is_wildcard_address(&c.local_address);
        let unexpected = !is_expected_port(c.local_port);
        let concerns = process_concerns(c, trust);

        if !(wildcard && unexpected) && concerns.is_empty() {
            continue;
        }

        let mut severity = Severity::Low;
        let mut reasons = Vec::new();
        if wildcard && unexpected {
            severity = Severity::Medium;
            reasons.push(format!(
                "it accepts connections from any network interface on port {}, which \
                 is not a port Windows or common software normally uses",
                c.local_port
            ));
        } else if unexpected {
            reasons.push(format!(
                "it listens on the unusual port {} (local connections only)",
                c.local_port
            ));
        }
        if !concerns.is_empty() {
            severity = severity.escalate(if wildcard {
                Severity::High
            } else {
                Severity::Medium
            });
            reasons.extend(concerns);
        }

        let mut evidence = vec![c.describe()];
        if let Some(p) = &c.process_path {
            evidence.push(p.clone());
            if let Some(t) = super::trust_of(trust, p) {
                evidence.push(t.describe());
            }
        }

        out.push(
            Finding::new(
                format!(
                    "network-listener:{}:{}",
                    c.local_port,
                    c.process_name.clone().unwrap_or_else(|| "unknown".into())
                ),
                severity,
                "network",
                format!(
                    "Unexpected listening port {} ({})",
                    c.local_port,
                    c.process_name.as_deref().unwrap_or("unknown process")
                ),
                format!(
                    "Something on this machine is waiting for incoming connections. It was \
                     flagged because {}. Plenty of legitimate software listens — game \
                     launchers, sync clients, dev servers — so identify the program \
                     before assuming the worst.",
                    reasons.join("; and ")
                ),
            )
            .with_evidence(evidence),
        );
    }

    // --- Outbound connections from untrusted programs --------------------
    let mut seen_out = std::collections::HashSet::new();
    for c in conns.iter().filter(|c| c.is_established()) {
        let remote = c.remote_address.clone().unwrap_or_default();
        if is_private_address(&remote) {
            continue; // LAN chatter is not exfiltration
        }
        let concerns = process_concerns(c, trust);
        if concerns.is_empty() {
            continue;
        }
        let key = (
            c.process_name.clone().unwrap_or_default(),
            c.process_path.clone().unwrap_or_default(),
        );
        if !seen_out.insert(key) {
            continue;
        }

        let mut evidence = vec![c.describe()];
        if let Some(p) = &c.process_path {
            evidence.push(p.clone());
            if let Some(t) = super::trust_of(trust, p) {
                evidence.push(t.describe());
            }
        }

        out.push(
            Finding::new(
                format!(
                    "network-outbound:{}",
                    c.process_path
                        .as_deref()
                        .map(super::normalize_path)
                        .unwrap_or_else(|| c.process_name.clone().unwrap_or_default())
                ),
                Severity::Medium,
                "network",
                format!(
                    "{} is talking to the internet",
                    c.process_name
                        .as_deref()
                        .unwrap_or("An unidentified program")
                ),
                format!(
                    "This program has an open connection to an address outside your \
                     network, and {}. That describes most indie and Electron software as \
                     well as most malware, so the question to answer is simply whether \
                     you know what this program is.",
                    concerns.join("; and ")
                ),
            )
            .with_evidence(evidence),
        );
    }

    // --- Inventory --------------------------------------------------------
    let listeners: Vec<&Connection> = conns.iter().filter(|c| c.is_listener()).collect();
    if !listeners.is_empty() {
        let mut lines: Vec<String> = listeners.iter().map(|c| c.describe()).collect();
        lines.sort();
        lines.dedup();
        let total = lines.len();
        lines.truncate(MAX_INVENTORY_EVIDENCE);
        out.push(
            Finding::new(
                "network-listener-inventory",
                Severity::Info,
                "network",
                format!("{total} listening TCP ports"),
                "The full list of programs accepting incoming connections, for reference.",
            )
            .with_evidence(lines),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{trust_map, FileTrust};

    fn listener(port: u32, addr: &str, name: &str, path: &str) -> Connection {
        Connection {
            local_address: addr.into(),
            local_port: port,
            remote_address: Some("0.0.0.0".into()),
            remote_port: Some(0),
            state: "Listen".into(),
            owning_process: Some(1234),
            process_name: Some(name.into()),
            process_path: (!path.is_empty()).then(|| path.to_string()),
        }
    }

    fn established(remote: &str, name: &str, path: &str) -> Connection {
        Connection {
            local_address: "192.168.1.10".into(),
            local_port: 51000,
            remote_address: Some(remote.into()),
            remote_port: Some(443),
            state: "Established".into(),
            owning_process: Some(4321),
            process_name: Some(name.into()),
            process_path: Some(path.into()),
        }
    }

    fn unsigned(path: &str) -> FileTrust {
        FileTrust {
            path: path.into(),
            exists: true,
            status: "NotSigned".into(),
            signer: None,
        }
    }

    fn signed(path: &str) -> FileTrust {
        FileTrust {
            path: path.into(),
            exists: true,
            status: "Valid".into(),
            signer: Some("CN=Example Ltd".into()),
        }
    }

    #[test]
    fn wildcard_and_loopback_addresses_are_distinguished() {
        assert!(is_wildcard_address("0.0.0.0"));
        assert!(is_wildcard_address("::"));
        assert!(!is_wildcard_address("127.0.0.1"));
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("::1"));
        assert!(!is_loopback("10.0.0.1"));
    }

    #[test]
    fn private_ranges_are_recognised() {
        for a in [
            "10.1.2.3",
            "192.168.0.5",
            "172.16.0.1",
            "172.31.255.255",
            "169.254.1.1",
            "fe80::1",
            "::1",
            "127.0.0.1",
        ] {
            assert!(is_private_address(a), "{a} should be private");
        }
        for a in [
            "8.8.8.8",
            "172.15.0.1",
            "172.32.0.1",
            "1.1.1.1",
            "2606:4700::1",
        ] {
            assert!(!is_private_address(a), "{a} should be public");
        }
    }

    #[test]
    fn expected_and_ephemeral_ports_are_not_flagged() {
        assert!(is_expected_port(445));
        assert!(is_expected_port(135));
        assert!(is_expected_port(49670));
        assert!(!is_expected_port(4444));
        assert!(!is_expected_port(1337));
    }

    #[test]
    fn rdp_listening_on_all_interfaces_is_high() {
        let conns = vec![listener(
            3389,
            "0.0.0.0",
            "svchost",
            r"C:\Windows\System32\svchost.exe",
        )];
        let f = analyze(&conns, &TrustMap::new());
        assert_eq!(f[0].id, "network-rdp-listening");
        assert_eq!(f[0].severity, Severity::High);
    }

    #[test]
    fn rdp_bound_to_loopback_only_is_medium() {
        let conns = vec![listener(
            3389,
            "127.0.0.1",
            "svchost",
            r"C:\Windows\System32\svchost.exe",
        )];
        assert_eq!(
            analyze(&conns, &TrustMap::new())[0].severity,
            Severity::Medium
        );
    }

    #[test]
    fn an_unsigned_listener_on_an_odd_port_is_high() {
        let path = r"C:\Users\bob\AppData\Local\Temp\svc.exe";
        let conns = vec![listener(4444, "0.0.0.0", "svc", path)];
        let trust = trust_map(vec![unsigned(path)]);
        let f = analyze(&conns, &trust);
        assert_eq!(f[0].severity, Severity::High);
        assert!(f[0].detail.contains("not a port Windows"));
        assert!(f[0].detail.contains("not digitally signed"));
        assert!(f[0].evidence.iter().any(|e| e.contains("svc.exe")));
        // Last finding is the inventory.
        assert_eq!(f[1].id, "network-listener-inventory");
    }

    #[test]
    fn ordinary_system_listeners_produce_only_an_inventory() {
        let conns = vec![
            listener(445, "0.0.0.0", "System", ""),
            listener(
                135,
                "0.0.0.0",
                "svchost",
                r"C:\Windows\System32\svchost.exe",
            ),
            listener(
                49670,
                "0.0.0.0",
                "services",
                r"C:\Windows\System32\services.exe",
            ),
        ];
        let trust = trust_map(vec![
            signed(r"C:\Windows\System32\svchost.exe"),
            signed(r"C:\Windows\System32\services.exe"),
        ]);
        let f = analyze(&conns, &trust);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "network-listener-inventory");
        assert_eq!(f[0].severity, Severity::Info);
    }

    #[test]
    fn the_same_service_on_ipv4_and_ipv6_is_one_finding() {
        let path = r"C:\Users\bob\AppData\Local\Temp\svc.exe";
        let conns = vec![
            listener(4444, "0.0.0.0", "svc", path),
            listener(4444, "::", "svc", path),
        ];
        let trust = trust_map(vec![unsigned(path)]);
        let f = analyze(&conns, &trust);
        assert_eq!(
            f.iter()
                .filter(|x| x.id.starts_with("network-listener:"))
                .count(),
            1
        );
    }

    #[test]
    fn an_unsigned_program_talking_to_the_internet_is_reported_once() {
        let path = r"C:\Users\bob\AppData\Roaming\updater.exe";
        let conns = vec![
            established("104.18.0.1", "updater", path),
            established("104.18.0.2", "updater", path),
        ];
        let trust = trust_map(vec![unsigned(path)]);
        let f = analyze(&conns, &trust);
        let outbound: Vec<_> = f
            .iter()
            .filter(|x| x.id.starts_with("network-outbound:"))
            .collect();
        assert_eq!(outbound.len(), 1);
        assert_eq!(outbound[0].severity, Severity::Medium);
    }

    #[test]
    fn lan_connections_and_signed_programs_are_ignored() {
        let signed_path = r"C:\Program Files\App\app.exe";
        let conns = vec![
            established("192.168.1.20", "app", signed_path),
            established("8.8.8.8", "app", signed_path),
        ];
        let trust = trust_map(vec![signed(signed_path)]);
        let f = analyze(&conns, &trust);
        assert!(f.iter().all(|x| !x.id.starts_with("network-outbound:")));
    }

    #[test]
    fn a_process_without_a_readable_path_is_not_accused() {
        // Unelevated scans cannot read the path of protected processes; that
        // must not turn into "unsigned".
        let conns = vec![listener(4444, "0.0.0.0", "System", "")];
        let f = analyze(&conns, &TrustMap::new());
        let listener_finding = f
            .iter()
            .find(|x| x.id.starts_with("network-listener:"))
            .unwrap();
        assert_eq!(listener_finding.severity, Severity::Medium);
        assert!(!listener_finding.detail.contains("not digitally signed"));
    }

    #[test]
    fn a_real_get_nettcpconnection_row_deserializes() {
        let json = r#"[{"localAddress":"0.0.0.0","localPort":445,"remoteAddress":"0.0.0.0",
          "remotePort":0,"state":"Listen","owningProcess":4,"processName":"System",
          "processPath":null}]"#;
        let conns: Vec<Connection> = crate::security::parse_ps_json(json);
        assert_eq!(conns.len(), 1);
        assert!(conns[0].is_listener());
        assert_eq!(conns[0].process_path, None);
        assert_eq!(
            conns[0].describe(),
            "listening on 0.0.0.0:445 — System (pid 4)"
        );
    }
}
