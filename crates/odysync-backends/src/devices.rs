//! Devices Windows reports as not working, explained in plain language.
//!
//! Driver Booster sells "fix no sound / network failure / bad resolution"
//! buttons that reinstall a driver and hope. Most of the value is in the part
//! that comes before any fix: *which* device is unhappy and *why*, which
//! Device Manager knows but hides behind a numeric code. This reports exactly
//! that and suggests the next step. It changes nothing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceProblem {
    pub name: String,
    /// Device class, e.g. `USB`, `Display`, `Net`.
    pub class: String,
    pub device_id: String,
    /// Device Manager's problem code.
    pub code: u32,
    /// What the code means.
    pub meaning: String,
    /// What to try first.
    pub suggestion: String,
}

/// Device Manager problem codes, in words.
///
/// Microsoft documents every code; these are the ones a user plausibly sees.
/// Unknown codes still get a sentence rather than a bare number.
pub fn explain(code: u32) -> (&'static str, &'static str) {
    match code {
        1 | 3 | 18 | 19 => (
            "The driver is not configured correctly or is damaged.",
            "Run a driver scan; if Windows Update offers a driver for this device, install it.",
        ),
        10 => (
            "The device cannot start.",
            "Unplug and reconnect it if you can, then restart. If it persists, look for a newer driver.",
        ),
        12 => (
            "The device cannot find enough free resources.",
            "Remove recently added hardware, or update the BIOS/UEFI firmware.",
        ),
        14 => (
            "The device needs a restart to work.",
            "Restart the computer.",
        ),
        22 => (
            "The device is disabled.",
            "Enable it in Device Manager if you did not disable it on purpose.",
        ),
        24 => (
            "The device is not present or not working properly.",
            "Reconnect it; if it was removed for good, this entry can be ignored.",
        ),
        28 => (
            "No driver is installed for this device.",
            "Run a driver scan — Windows Update often has one. Otherwise get it from the manufacturer.",
        ),
        31 => (
            "Windows cannot load the driver for this device.",
            "Look for a newer driver, or roll back to the previous one in Device Manager.",
        ),
        39 => (
            "The driver is missing or corrupted.",
            "Reinstall the driver from Windows Update or the manufacturer.",
        ),
        43 => (
            "Windows stopped the device because it reported a problem.",
            "For USB devices: try another port or cable. Otherwise update or reinstall the driver.",
        ),
        45 => (
            "The device is not connected right now.",
            "Nothing to do unless you expected it to be connected.",
        ),
        48 => (
            "The driver was blocked because it is known to cause problems.",
            "Get an updated driver from the manufacturer; do not force the old one.",
        ),
        52 => (
            "Windows cannot verify the driver's digital signature.",
            "Install a signed driver from Windows Update or the manufacturer.",
        ),
        _ => (
            "Windows reports a problem with this device.",
            "Open Device Manager for details, and look for a newer driver.",
        ),
    }
}

/// Parse `Win32_PnPEntity` rows emitted by `ConvertTo-Json`.
///
/// Codes 0 (working) and 45 (simply not connected right now — Windows keeps
/// an entry for every device ever plugged in) are not problems worth a line.
pub fn parse(json: &str) -> Vec<DeviceProblem> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(rename = "Name")]
        name: Option<String>,
        #[serde(rename = "PNPClass")]
        class: Option<String>,
        #[serde(rename = "PNPDeviceID")]
        device_id: Option<String>,
        #[serde(rename = "ConfigManagerErrorCode")]
        code: Option<u32>,
    }

    let json = json.trim();
    if json.is_empty() {
        return Vec::new();
    }
    let rows: Vec<Row> = if json.starts_with('[') {
        serde_json::from_str(json).unwrap_or_default()
    } else {
        serde_json::from_str::<Row>(json)
            .map(|r| vec![r])
            .unwrap_or_default()
    };

    let mut out: Vec<DeviceProblem> = rows
        .into_iter()
        .filter_map(|r| {
            let code = r.code?;
            if code == 0 || code == 45 {
                return None;
            }
            let (meaning, suggestion) = explain(code);
            Some(DeviceProblem {
                name: r.name.unwrap_or_else(|| "Unnamed device".into()),
                class: r.class.unwrap_or_default(),
                device_id: r.device_id.unwrap_or_default(),
                code,
                meaning: meaning.into(),
                suggestion: suggestion.into(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.class.cmp(&b.class).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Every present device with a problem code.
pub async fn problems() -> Vec<DeviceProblem> {
    #[cfg(windows)]
    {
        let script = "Get-CimInstance Win32_PnPEntity | \
                      Where-Object { $_.ConfigManagerErrorCode -ne 0 } | \
                      Select-Object Name, PNPClass, PNPDeviceID, ConfigManagerErrorCode | \
                      ConvertTo-Json -Compress";
        match odysync_core::proc::powershell(script, std::time::Duration::from_secs(60)).await {
            Ok(out) => parse(&out.stdout),
            Err(e) => {
                tracing::warn!(error = %e, "could not list device problems");
                Vec::new()
            }
        }
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from a German Windows 11 machine: the device name is
    /// localised, the code is not — which is why the code is what matters.
    const REAL: &str = r#"{"Name":"Unbekanntes USB-Gerät (Fehler beim Anfordern einer Konfigurationsbeschreibung.)","PNPClass":"USB","ConfigManagerErrorCode":43,"PNPDeviceID":"USB\\VID_0000&PID_0003\\8&EF6451E&0&10"}"#;

    #[test]
    fn a_single_problem_device_is_explained() {
        let found = parse(REAL);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].code, 43);
        assert_eq!(found[0].class, "USB");
        assert!(found[0].meaning.contains("stopped the device"));
        assert!(found[0].suggestion.contains("another port"));
    }

    #[test]
    fn working_and_disconnected_devices_are_not_problems() {
        let json = r#"[
            {"Name":"Working","PNPClass":"Net","ConfigManagerErrorCode":0,"PNPDeviceID":"A"},
            {"Name":"Unplugged","PNPClass":"USB","ConfigManagerErrorCode":45,"PNPDeviceID":"B"},
            {"Name":"No driver","PNPClass":"Unknown","ConfigManagerErrorCode":28,"PNPDeviceID":"C"}
        ]"#;
        let found = parse(json);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "No driver");
        assert!(found[0].meaning.contains("No driver"));
    }

    #[test]
    fn every_code_gets_words_not_just_a_number() {
        for code in [1, 10, 12, 14, 22, 24, 28, 31, 39, 43, 48, 52, 999] {
            let (meaning, suggestion) = explain(code);
            assert!(meaning.len() > 10 && suggestion.len() > 10, "code {code}");
        }
    }

    #[test]
    fn garbage_is_not_a_device_list() {
        assert!(parse("").is_empty());
        assert!(parse("not json").is_empty());
    }
}
