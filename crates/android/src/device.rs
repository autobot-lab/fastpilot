//! Android devices as fastpilot sees them: `host:devices-l` entries with their state and a
//! hint for the states that need the user (USB debugging not authorized, offline), resolved
//! from a `DeviceSpec` (tagged, serial or model) or the environment.

use fastpilot_core::{DeviceInfo, DeviceKind, DeviceSpec, Platform, select};

use crate::{Adb, Error};

/// One `host:devices-l` line: `serial state key:value...`.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub serial: String,
    pub state: String,
    pub model: String,
    pub product: String,
}

pub fn parse_devices_l(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let serial = it.next()?.to_owned();
            let mut state = it.next()?.to_owned();
            // `no permissions (user ...)`: the state is several words.
            if state == "no" {
                state = "no permissions".into();
            }
            let mut e = Entry { serial, state, model: String::new(), product: String::new() };
            for kv in it {
                match kv.split_once(':') {
                    Some(("model", v)) => e.model = v.replace('_', " "),
                    Some(("product", v)) => e.product = v.to_owned(),
                    _ => {}
                }
            }
            Some(e)
        })
        .collect()
}

/// What to do about a state other than `device`.
pub fn hint(state: &str) -> Option<&'static str> {
    Some(match state {
        "unauthorized" => "allow USB debugging on the phone (tick \"Always allow\"); if no prompt: Revoke USB debugging authorizations, replug",
        "offline" => "replug the cable, toggle USB debugging, or run `adb kill-server`",
        "no permissions" => "the USB device is not accessible (udev rules on Linux)",
        "authorizing" | "connecting" => "wait a moment and retry",
        "device" => return None,
        _ => "check the cable and USB debugging",
    })
}

pub fn info(e: &Entry) -> DeviceInfo {
    let kind = if e.serial.starts_with("emulator-") { DeviceKind::Emulator } else { DeviceKind::AndroidDevice };
    let name = if e.model.is_empty() { e.product.clone() } else { e.model.clone() };
    DeviceInfo { id: e.serial.clone(), kind, name, state: e.state.clone(), detail: hint(&e.state).map(str::to_owned).into_iter().collect() }
}

/// Attached Android devices, every state included.
pub async fn list() -> Result<Vec<DeviceInfo>, Error> {
    Ok(Adb::new().await?.devices_long().await?.iter().map(info).collect())
}

/// Serial for a session: `spec`, else `FASTPILOT_ANDROID_SERIAL`, else `ANDROID_SERIAL`, else
/// the only usable device. A device that needs the user fails with its hint.
pub async fn resolve(spec: Option<&str>) -> Result<String, Error> {
    let spec = spec
        .map(str::to_owned)
        .or_else(|| fastpilot_core::config::var("FASTPILOT_ANDROID_SERIAL").ok())
        .or_else(|| std::env::var("ANDROID_SERIAL").ok())
        .filter(|s| !s.is_empty());
    let spec = spec.as_deref().map(DeviceSpec::parse).transpose().map_err(Error::Adb)?;
    let all = list().await?;
    let d = select(&all, spec.as_ref(), Platform::Android).map_err(|e| Error::Adb(e.to_string()))?;
    match d.state.as_str() {
        "device" => Ok(d.id.clone()),
        "unauthorized" => Err(Error::Unauthorized(format!("{} is unauthorized: {}", d.id, hint("unauthorized").unwrap_or("")))),
        "offline" => Err(Error::Offline(format!("{} is offline: {}", d.id, hint("offline").unwrap_or("")))),
        other => Err(Error::Adb(format!("{} is {other}: {}", d.id, hint(other).unwrap_or("")))),
    }
}

/// OEM family from system properties, for OEM-specific hints (Xiaomi, Samsung, Pixel).
pub fn oem(manufacturer: &str, miui: &str, hyperos: &str) -> &'static str {
    let m = manufacturer.trim().to_ascii_lowercase();
    if !miui.trim().is_empty() || !hyperos.trim().is_empty() || ["xiaomi", "redmi", "poco"].contains(&m.as_str()) {
        "xiaomi"
    } else if m == "samsung" {
        "samsung"
    } else if m == "google" {
        "pixel"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_list_and_hints() {
        let text = "emulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\n\
                    R5CT1234ABC            unauthorized usb:1-1 transport_id:3\n\
                    8a3f2e91               device usb:1-2 product:ruby_global model:23021RAAEG device:ruby transport_id:4\n";
        let e = parse_devices_l(text);
        assert_eq!(e.len(), 3);
        assert_eq!((e[2].model.as_str(), e[1].state.as_str()), ("23021RAAEG", "unauthorized"));
        let infos: Vec<DeviceInfo> = e.iter().map(info).collect();
        assert_eq!(infos[0].kind, DeviceKind::Emulator);
        assert_eq!(infos[2].kind, DeviceKind::AndroidDevice);
        assert!(infos[1].detail[0].contains("Always allow"));
        // Two usable devices: no default.
        assert!(select(&infos, None, Platform::Android).is_err());
        assert_eq!(oem("Xiaomi", "", ""), "xiaomi");
        assert_eq!(oem("samsung", "", ""), "samsung");
        assert_eq!(oem("unknown", "V14", ""), "xiaomi");
        assert_eq!(oem("Google", "", ""), "pixel");
    }
}

/// Backup of the settings fastpilot changes on a device, restored on close.
pub const BACKUP: &str = "/data/local/tmp/fastpilot-anim.bak";
const SCALES: [&str; 3] = ["window_animation_scale", "transition_animation_scale", "animator_duration_scale"];

/// Saves the animation scales and stay-awake setting (only if no backup exists yet, so a
/// crashed earlier session's originals are kept), then turns animations off and keeps the
/// screen on while plugged in. Runs under `sh -c` (adb shell service).
pub fn prepare_script() -> String {
    let save: Vec<String> = SCALES.iter().map(|k| format!("echo \"{k}=$(settings get global {k})\"")).collect();
    let zero: Vec<String> = SCALES.iter().map(|k| format!("settings put global {k} 0")).collect();
    format!(
        "f={BACKUP}; [ -f $f ] || {{ {{ {}; echo \"stay_on_while_plugged_in=$(settings get global stay_on_while_plugged_in)\"; }} > $f; }}; {}; svc power stayon usb",
        save.join("; "),
        zero.join("; ")
    )
}

/// Puts every saved value back (`null` = the setting did not exist) and removes the backup.
pub fn restore_script() -> String {
    format!(
        "f={BACKUP}; [ -f $f ] && {{ while IFS='=' read -r k v; do [ -n \"$k\" ] || continue; if [ \"$v\" = null ]; then settings delete global \"$k\"; else settings put global \"$k\" \"$v\"; fi; done < $f; rm -f $f; }}; true"
    )
}

/// `mWakefulness=Awake` in `dumpsys power`.
pub fn awake(dumpsys_power: &str) -> bool {
    dumpsys_power.lines().any(|l| l.trim() == "mWakefulness=Awake")
}

/// Keyguard still showing, per `dumpsys window` (field names vary by version/OEM).
pub fn locked(dumpsys_window: &str) -> bool {
    ["mDreamingLockscreen=true", "isKeyguardShowing=true", "mShowingLockscreen=true", "keyguardShowing=true", "mKeyguardShowing=true"]
        .iter()
        .any(|k| dumpsys_window.contains(k))
}

/// What an install failure means, with the OEM fix when known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallFailure {
    /// Signed with another key (another machine's debug keystore) or newer: uninstall and retry.
    Incompatible,
    /// MIUI/HyperOS blocks USB installs until "Install via USB" is on.
    UserRestricted,
    /// Waiting for an on-phone confirmation.
    Confirm,
    Other,
}

pub fn install_failure(out: &str) -> InstallFailure {
    if out.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") || out.contains("INSTALL_FAILED_VERSION_DOWNGRADE") {
        InstallFailure::Incompatible
    } else if out.contains("INSTALL_FAILED_USER_RESTRICTED") {
        InstallFailure::UserRestricted
    } else if out.contains("AdbInstallActivity") || out.contains("packageinstaller") {
        InstallFailure::Confirm
    } else {
        InstallFailure::Other
    }
}

pub const XIAOMI_HINT: &str =
    "Xiaomi/Redmi/POCO: Developer options → turn on \"Install via USB\" and \"USB debugging (Security settings)\" (needs a Mi account and SIM)";
pub const SAMSUNG_HINT: &str = "Samsung: turn off Auto Blocker (Settings → Security and privacy)";

#[cfg(test)]
mod hardening_tests {
    use super::*;

    #[test]
    fn scripts_and_state_parsers() {
        let p = prepare_script();
        assert!(p.starts_with("f=/data/local/tmp/fastpilot-anim.bak; [ -f $f ] ||") && p.contains("settings put global animator_duration_scale 0"));
        assert!(p.contains("svc power stayon usb") && p.contains("stay_on_while_plugged_in=$(settings get global"));
        let r = restore_script();
        assert!(r.contains("settings delete global") && r.contains("rm -f $f"));
        assert!(awake("  mWakefulness=Awake\n") && !awake("  mWakefulness=Asleep\n"));
        assert!(locked("    mDreamingLockscreen=true mDreamingSleepToken=null") && !locked("isKeyguardShowing=false"));
        assert_eq!(install_failure("Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Package ...]"), InstallFailure::Incompatible);
        assert_eq!(install_failure("Failure [INSTALL_FAILED_USER_RESTRICTED: Install canceled by user]"), InstallFailure::UserRestricted);
        assert_eq!(install_failure("Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE]"), InstallFailure::Other);
    }
}
