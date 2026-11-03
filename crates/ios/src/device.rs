//! Physical iPhones/iPads through `devicectl` (Xcode 15+), the runner's signing profile, and
//! the preflight behind `fastpilot ios doctor`: every reason a device session cannot start,
//! each with the fix. Parsing is pure (tested against a real `devicectl` dump); I/O is thin.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::process::Command;

/// What `devicectl list devices` reports for one device (fields absent while disconnected
/// are `None`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceFacts {
    /// `hardwareProperties.udid`: the canonical id (`xcodebuild -destination id=` takes it).
    pub udid: String,
    pub name: String,
    pub os_version: String,
    pub pairing: String,
    /// `connected` when reachable; `unavailable` when not plugged in / not unlocked yet.
    pub tunnel: String,
    pub developer_mode: Option<String>,
    pub ddi: Option<bool>,
    pub locked: Option<bool>,
}

impl DeviceFacts {
    pub fn reachable(&self) -> bool {
        self.tunnel != "unavailable" && !self.tunnel.is_empty()
    }
}

/// `devicectl list devices -j` (jsonVersion 2): iOS/iPadOS devices only.
pub fn parse_devices(json: &Value) -> Vec<DeviceFacts> {
    json["result"]["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["hardwareProperties"]["platform"].as_str().is_none_or(|p| p == "iOS"))
        .filter_map(|d| {
            let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
            let udid = d["hardwareProperties"]["udid"].as_str()?.to_owned();
            Some(DeviceFacts {
                udid,
                name: s(&d["deviceProperties"]["name"]),
                os_version: s(&d["deviceProperties"]["osVersionNumber"]),
                pairing: s(&d["connectionProperties"]["pairingState"]),
                tunnel: s(&d["connectionProperties"]["tunnelState"]),
                developer_mode: d["deviceProperties"]["developerModeStatus"].as_str().map(str::to_owned),
                ddi: d["deviceProperties"]["ddiServicesAvailable"].as_bool(),
                locked: None,
            })
        })
        .collect()
}

fn major(version: &str) -> Option<u32> {
    version.split('.').next()?.trim().parse().ok()
}

/// The runner's provisioning profile (`embedded.mobileprovision`).
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub expires: SystemTime,
    pub devices: Vec<String>,
    pub team: String,
    pub app: String,
}

impl Profile {
    pub fn left(&self, now: SystemTime) -> Duration {
        self.expires.duration_since(now).unwrap_or(Duration::ZERO)
    }
}

/// The plist inside a CMS-signed `.mobileprovision`.
pub fn parse_profile(bytes: &[u8], app: &str) -> Option<Profile> {
    let start = bytes.windows(5).position(|w| w == b"<?xml")?;
    let end = bytes.windows(8).rposition(|w| w == b"</plist>")? + 8;
    let v: plist::Value = plist::from_bytes(&bytes[start..end]).ok()?;
    let d = v.as_dictionary()?;
    let expires: SystemTime = d.get("ExpirationDate")?.as_date()?.into();
    let devices = d
        .get("ProvisionedDevices")
        .and_then(plist::Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_string().map(str::to_owned)).collect())
        .unwrap_or_default();
    let team = d
        .get("TeamIdentifier")
        .and_then(plist::Value::as_array)
        .and_then(|a| a.first())
        .and_then(plist::Value::as_string)
        .unwrap_or("")
        .to_owned();
    Some(Profile { expires, devices, team, app: app.to_owned() })
}

/// Device build of the runner (`ios-runner/build.sh --device`), overridable with
/// `FASTPILOT_IOS_RUNNER_DEVICE`.
pub fn device_build_dir() -> PathBuf {
    std::env::var_os("FASTPILOT_IOS_RUNNER_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ios-runner/build-device")))
}

/// Profiles of the built runner apps (the demo is never part of a device build).
pub fn runner_profiles(build: &Path) -> Vec<Profile> {
    let products = build.join("Build/Products/Debug-iphoneos");
    std::fs::read_dir(products)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "app") && !p.to_string_lossy().contains("Demo"))
        .filter_map(|p| {
            let bytes = std::fs::read(p.join("embedded.mobileprovision")).ok()?;
            parse_profile(&bytes, &p.file_name()?.to_string_lossy())
        })
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostFacts {
    pub devicectl: bool,
    pub xcodebuild: bool,
    /// `xcrun --sdk iphoneos --show-sdk-version`.
    pub sdk: Option<String>,
    pub identities: usize,
    pub runner_built: bool,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub what: String,
    pub fix: String,
    /// A session cannot start until this is fixed (others are warnings).
    pub blocking: bool,
}

fn problem(what: impl Into<String>, fix: impl Into<String>, blocking: bool) -> Problem {
    Problem { what: what.into(), fix: fix.into(), blocking }
}

/// Every reason a session on `d` would fail, in the order a user fixes them.
pub fn preflight(d: &DeviceFacts, h: &HostFacts, now: SystemTime) -> Vec<Problem> {
    let mut out = Vec::new();
    if !h.devicectl || !h.xcodebuild {
        out.push(problem(
            "Xcode command line tools not found (devicectl / xcodebuild)",
            "install Xcode, then `sudo xcode-select -s /Applications/Xcode.app`",
            true,
        ));
    }
    if let (Some(sdk), Some(os)) = (h.sdk.as_deref().and_then(major), major(&d.os_version))
        && sdk < os
    {
        out.push(problem(
            format!("Xcode's iOS SDK {} cannot run tests on iOS {}", h.sdk.as_deref().unwrap_or(""), d.os_version),
            format!("install Xcode {os} or newer and select it with xcode-select"),
            true,
        ));
    }
    if d.pairing != "paired" {
        out.push(problem(format!("{} is not paired ({})", d.name, d.pairing), "connect it by USB, unlock it and tap Trust This Computer", true));
    }
    if !d.reachable() {
        let tunnel = if d.tunnel.is_empty() { "unknown" } else { d.tunnel.as_str() };
        out.push(problem(
            format!("{} is not reachable (tunnel {tunnel})", d.name),
            "connect it by USB (or the same Wi-Fi after pairing) and unlock it",
            true,
        ));
    }
    match d.developer_mode.as_deref() {
        Some("enabled") => {}
        Some(other) => out.push(problem(
            format!("Developer Mode is {other}"),
            "Settings → Privacy & Security → Developer Mode → on, then restart the phone",
            true,
        )),
        None if d.reachable() => {
            out.push(problem("Developer Mode status unknown", "check Settings → Privacy & Security → Developer Mode", false));
        }
        None => {}
    }
    if d.reachable() && d.ddi == Some(false) {
        out.push(problem(
            "developer disk image services not available",
            "open Xcode → Window → Devices and Simulators once with the phone connected",
            true,
        ));
    }
    if d.locked == Some(true) {
        out.push(problem(format!("{} is locked", d.name), "unlock it; set Auto-Lock to Never while testing", true));
    }
    if h.identities == 0 {
        out.push(problem(
            "no code-signing identity on this Mac",
            "Xcode → Settings → Accounts: add your Apple ID (a free personal team works), then build the runner",
            true,
        ));
    }
    if !h.runner_built {
        out.push(problem(
            "runner not built for devices",
            format!("FASTPILOT_IOS_TEAM=<team> FASTPILOT_IOS_BUNDLE_PREFIX=com.<you>.fastpilot ios-runner/build.sh --device {}", d.udid),
            true,
        ));
    }
    for p in &h.profiles {
        let left = p.left(now);
        if left.is_zero() {
            out.push(problem(
                format!("{}: signing profile expired", p.app),
                format!("rebuild: ios-runner/build.sh --device {} (free profiles last 7 days)", d.udid),
                true,
            ));
        } else if left < Duration::from_secs(24 * 3600) {
            out.push(problem(
                format!("{}: signing profile expires in {}h", p.app, left.as_secs() / 3600),
                format!("rebuild soon: ios-runner/build.sh --device {}", d.udid),
                false,
            ));
        }
        if !p.devices.is_empty() && !p.devices.contains(&d.udid) {
            out.push(problem(
                format!("{}: profile does not include this device", p.app),
                format!("rebuild for it: ios-runner/build.sh --device {}", d.udid),
                true,
            ));
        }
    }
    if major(&d.os_version).is_some_and(|m| m >= 26) {
        out.push(problem(
            "iOS 26 may ask to allow UI automation on the first run",
            "if tests hang at start: Settings → Developer → Enable UI Automation",
            false,
        ));
    }
    out
}

async fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `devicectl ... -j <tmp>`: its JSON goes to a file, not stdout.
pub async fn devicectl_json(args: &[&str]) -> Option<Value> {
    let tag: String = args.join("-").chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let path = std::env::temp_dir().join(format!("fastpilot-devicectl-{}-{tag}.json", std::process::id()));
    let p = path.display().to_string();
    let mut full: Vec<&str> = vec!["devicectl"];
    full.extend_from_slice(args);
    full.extend_from_slice(&["-j", &p]);
    let _ = Command::new("xcrun").args(&full).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await;
    let v = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let _ = std::fs::remove_file(&path);
    v
}

/// Paired physical devices, with the lock state of reachable ones.
pub async fn list() -> Vec<DeviceFacts> {
    let Some(v) = devicectl_json(&["list", "devices"]).await else { return Vec::new() };
    let mut devices = parse_devices(&v);
    for d in devices.iter_mut().filter(|d| d.reachable()) {
        if let Some(l) = devicectl_json(&["device", "info", "lockState", "--device", &d.udid]).await {
            d.locked = l["result"]["passcodeRequired"].as_bool().or_else(|| l["result"]["isLocked"].as_bool());
        }
    }
    devices
}

pub async fn host() -> HostFacts {
    let devicectl = run("xcrun", &["devicectl", "--version"]).await.is_some();
    let xcodebuild = run("xcodebuild", &["-version"]).await.is_some();
    let sdk = run("xcrun", &["--sdk", "iphoneos", "--show-sdk-version"]).await.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let identities = run("security", &["find-identity", "-v", "-p", "codesigning"])
        .await
        .map(|s| s.lines().filter(|l| l.trim_start().chars().next().is_some_and(|c| c.is_ascii_digit()) && l.contains('"')).count())
        .unwrap_or(0);
    let profiles = runner_profiles(&device_build_dir());
    HostFacts { devicectl, xcodebuild, sdk, identities, runner_built: !profiles.is_empty(), profiles }
}

pub fn days_left(p: &Profile) -> u64 {
    p.left(SystemTime::now()).as_secs() / 86_400
}

/// `fastpilot ios doctor [udid]`: host facts, each device and what blocks it. Err when a
/// blocking problem remains.
pub async fn doctor(udid: Option<&str>) -> Result<String, String> {
    let h = host().await;
    let devices: Vec<DeviceFacts> = list().await.into_iter().filter(|d| udid.is_none_or(|u| d.udid == u || d.name == u)).collect();
    let mut out = vec![format!(
        "host: devicectl {}, xcodebuild {}, iOS SDK {}, {} signing identities, runner for devices {}",
        if h.devicectl { "ok" } else { "missing" },
        if h.xcodebuild { "ok" } else { "missing" },
        h.sdk.as_deref().unwrap_or("none"),
        h.identities,
        if h.runner_built { "built" } else { "not built" }
    )];
    for p in &h.profiles {
        out.push(format!("  profile {} (team {}): {} days left, {} devices", p.app, p.team, days_left(p), p.devices.len()));
    }
    if devices.is_empty() {
        out.push(match udid {
            Some(u) => format!("no paired device `{u}`: connect it by USB, unlock it and tap Trust"),
            None => "no paired iPhone/iPad: connect one by USB, unlock it and tap Trust".into(),
        });
        return Err(out.join("\n"));
    }
    let now = SystemTime::now();
    let mut blocked = false;
    for d in &devices {
        out.push(format!("\nios-device:{} \"{}\" iOS {} (pairing {}, tunnel {})", d.udid, d.name, d.os_version, d.pairing, d.tunnel));
        let problems = preflight(d, &h, now);
        if problems.iter().all(|p| !p.blocking) {
            out.push("  ready".into());
        }
        for p in problems {
            blocked |= p.blocking;
            out.push(format!("  {} {}\n      fix: {}", if p.blocking { "✗" } else { "!" }, p.what, p.fix));
        }
    }
    let text = out.join("\n");
    if blocked { Err(text) } else { Ok(text) }
}

/// A point in time `secs` after the epoch.
pub fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFLINE: &str = include_str!("../tests/fixtures/devicectl-offline.json");

    #[test]
    fn parses_devicectl_and_runs_preflight() {
        let devices = parse_devices(&serde_json::from_str(OFFLINE).expect("json"));
        assert_eq!(devices.len(), 1);
        let d = &devices[0];
        assert_eq!((d.udid.as_str(), d.os_version.as_str(), d.pairing.as_str()), ("00008110-001C5C9E0AC0A01E", "26.6.1", "paired"));
        assert!(!d.reachable() && d.ddi == Some(false) && d.developer_mode.is_none());

        // Today's machine: Xcode 16.1 (SDK 18.1), no identities, nothing built, phone offline.
        let today = HostFacts { devicectl: true, xcodebuild: true, sdk: Some("18.1".into()), ..HostFacts::default() };
        let p = preflight(d, &today, at(0));
        let what: Vec<&str> = p.iter().filter(|p| p.blocking).map(|p| p.what.as_str()).collect();
        assert!(what.iter().any(|w| w.contains("SDK 18.1 cannot run tests on iOS 26.6.1")), "{what:?}");
        assert!(what.iter().any(|w| w.contains("not reachable")) && what.iter().any(|w| w.contains("no code-signing identity")), "{what:?}");
        assert!(what.iter().any(|w| w.contains("runner not built")), "{what:?}");
        assert!(!what.iter().any(|w| w.contains("disk image")), "DDI is only checked once reachable");

        // Ready: connected, Developer Mode on, Xcode 26, signed and in date.
        let ready = DeviceFacts {
            tunnel: "connected".into(),
            developer_mode: Some("enabled".into()),
            ddi: Some(true),
            locked: Some(false),
            ..d.clone()
        };
        let profile = Profile { expires: at(10 * 86_400), devices: vec![d.udid.clone()], team: "T".into(), app: "FastpilotHost.app".into() };
        let host = HostFacts { sdk: Some("26.0".into()), identities: 1, runner_built: true, profiles: vec![profile.clone()], ..today.clone() };
        assert!(preflight(&ready, &host, at(0)).iter().all(|p| !p.blocking));
        assert!(preflight(&ready, &host, at(11 * 86_400)).iter().any(|p| p.blocking && p.what.contains("expired")));
        assert!(preflight(&ready, &host, at(10 * 86_400 - 3600)).iter().any(|p| !p.blocking && p.what.contains("expires in")));
        let other = HostFacts { profiles: vec![Profile { devices: vec!["OTHER".into()], ..profile }], ..host };
        assert!(preflight(&ready, &other, at(0)).iter().any(|p| p.what.contains("does not include this device")));
    }

    #[test]
    fn reads_a_signed_profile() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>
<key>ExpirationDate</key><date>2026-11-10T00:00:00Z</date>
<key>ProvisionedDevices</key><array><string>00008110-001C5C9E0AC0A01E</string></array>
<key>TeamIdentifier</key><array><string>ABCDE12345</string></array></dict></plist>"#;
        let mut cms = b"\x30\x82junk".to_vec();
        cms.extend_from_slice(xml);
        cms.extend_from_slice(b"\x00signature");
        let p = parse_profile(&cms, "FastpilotHost.app").expect("profile");
        assert_eq!((p.team.as_str(), p.devices.len()), ("ABCDE12345", 1));
        assert_eq!(p.expires, at(1_794_268_800));
    }
}
