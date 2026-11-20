//! Which device a session runs on: tagged ids (`ios-device:<udid>`, `simulator:<udid>`,
//! `emulator:<serial>`, `android-device:<serial>`) or plain ids / names. Without a spec the old
//! behaviour holds (iOS: the booted simulator; Android: the only device); a physical iPhone is
//! only used when named, so its few free-signing app slots are never used by accident.

use std::fmt;

use crate::Platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Simulator,
    IosDevice,
    Emulator,
    AndroidDevice,
}

impl DeviceKind {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Simulator => "simulator",
            Self::IosDevice => "ios-device",
            Self::Emulator => "emulator",
            Self::AndroidDevice => "android-device",
        }
    }

    /// What a person calls it (QA B9: the Device page showed only raw ids).
    pub fn label(self) -> &'static str {
        match self {
            Self::Simulator => "iPhone simulator",
            Self::IosDevice => "iPhone (physical)",
            Self::Emulator => "Android emulator",
            Self::AndroidDevice => "Android phone (physical)",
        }
    }

    /// The kind of an adb serial: `emulator-5556` is an emulator, anything else a phone.
    pub fn of_android_serial(serial: &str) -> Self {
        if serial.starts_with("emulator-") { Self::Emulator } else { Self::AndroidDevice }
    }

    pub fn platform(self) -> Platform {
        match self {
            Self::Simulator | Self::IosDevice => Platform::Ios,
            Self::Emulator | Self::AndroidDevice => Platform::Android,
        }
    }

    fn parse(tag: &str) -> Option<Self> {
        [Self::Simulator, Self::IosDevice, Self::Emulator, Self::AndroidDevice].into_iter().find(|k| k.tag() == tag)
    }

    pub fn physical(self) -> bool {
        matches!(self, Self::IosDevice | Self::AndroidDevice)
    }
}

/// `ios-device:0000…`, or a plain id / name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSpec {
    pub kind: Option<DeviceKind>,
    pub id: String,
}

impl DeviceSpec {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty device".into());
        }
        // `emulator-5554` is a serial, not a tag: only `tag:` with a known tag counts.
        if let Some((tag, id)) = s.split_once(':')
            && let Some(kind) = DeviceKind::parse(tag)
        {
            if id.is_empty() {
                return Err(format!("`{s}`: missing id after `{tag}:`"));
            }
            return Ok(Self { kind: Some(kind), id: id.to_owned() });
        }
        Ok(Self { kind: None, id: s.to_owned() })
    }

    /// The platform the tag implies conflicts with the session's.
    pub fn check_platform(&self, platform: Platform) -> Result<(), String> {
        match self.kind {
            Some(k) if k.platform() != platform => Err(format!("device `{self}` is {}, the session is {platform}", k.platform())),
            _ => Ok(()),
        }
    }
}

impl fmt::Display for DeviceSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            Some(k) => write!(f, "{}:{}", k.tag(), self.id),
            None => f.write_str(&self.id),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub id: String,
    pub kind: DeviceKind,
    pub name: String,
    /// `Booted`, `device`, `unauthorized`, `offline`, `paired`, ...
    pub state: String,
    /// Extra facts and hints shown in `devices` (OS version, profile days left, ...).
    pub detail: Vec<String>,
}

impl DeviceInfo {
    pub fn tagged(&self) -> String {
        format!("{}:{}", self.kind.tag(), self.id)
    }

    /// Ready to drive without user action.
    pub fn usable(&self) -> bool {
        match self.kind {
            DeviceKind::Simulator => self.state == "Booted",
            DeviceKind::Emulator | DeviceKind::AndroidDevice => self.state == "device",
            DeviceKind::IosDevice => true,
        }
    }

    pub fn line(&self) -> String {
        let detail = if self.detail.is_empty() { String::new() } else { format!("; {}", self.detail.join("; ")) };
        format!("{}: {} {:?} ({}{detail})", self.kind.platform(), self.tagged(), self.name, self.state)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectError {
    /// Nothing matches the spec.
    NotFound { spec: String, candidates: Vec<String> },
    Ambiguous { spec: String, matches: Vec<String> },
    WrongPlatform(String),
    /// No spec, and no usable device of the platform.
    NoDevice { platform: Platform, candidates: Vec<String> },
    /// No spec, and several usable devices.
    Several { platform: Platform, candidates: Vec<String> },
}

impl fmt::Display for SelectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |v: &[String]| if v.is_empty() { "none".to_owned() } else { v.join(", ") };
        match self {
            Self::NotFound { spec, candidates } => write!(f, "no device matches `{spec}` (have: {})", list(candidates)),
            Self::Ambiguous { spec, matches } => write!(f, "`{spec}` matches several devices: {}; use a full id", list(matches)),
            Self::WrongPlatform(m) => f.write_str(m),
            Self::NoDevice { platform, candidates } => match platform {
                Platform::Ios => write!(f, "no booted simulator (physical iPhones are only used when named: device: ios-device:<udid>); have: {}", list(candidates)),
                _ => write!(f, "no usable {platform} device; connect one or boot an emulator (have: {})", list(candidates)),
            },
            Self::Several { platform, candidates } => write!(f, "several {platform} devices; pick one with `device`: {}", list(candidates)),
        }
    }
}

/// Pick the device for a session: id exactly → name exactly → unique case-insensitive prefix
/// of the id or name. Without a spec: the only usable device of the platform (never a
/// physical iPhone).
pub fn select<'a>(all: &'a [DeviceInfo], spec: Option<&DeviceSpec>, platform: Platform) -> Result<&'a DeviceInfo, SelectError> {
    let mine: Vec<&DeviceInfo> = all.iter().filter(|d| d.kind.platform() == platform).collect();
    let names = |v: &[&DeviceInfo]| v.iter().map(|d| d.line()).collect::<Vec<_>>();
    let Some(spec) = spec else {
        let usable: Vec<&DeviceInfo> =
            mine.iter().copied().filter(|d| d.usable() && !(platform == Platform::Ios && d.kind == DeviceKind::IosDevice)).collect();
        return match usable.as_slice() {
            [one] => Ok(one),
            [] => Err(SelectError::NoDevice { platform, candidates: names(&mine) }),
            many => Err(SelectError::Several { platform, candidates: names(many) }),
        };
    };
    spec.check_platform(platform).map_err(SelectError::WrongPlatform)?;
    let pool: Vec<&DeviceInfo> = mine.iter().copied().filter(|d| spec.kind.is_none_or(|k| k == d.kind)).collect();
    if let Some(d) = pool.iter().find(|d| d.id == spec.id) {
        return Ok(d);
    }
    if let Some(d) = pool.iter().find(|d| d.name == spec.id) {
        return Ok(d);
    }
    let want = spec.id.to_lowercase();
    let prefixed: Vec<&DeviceInfo> =
        pool.iter().copied().filter(|d| d.id.to_lowercase().starts_with(&want) || d.name.to_lowercase().starts_with(&want)).collect();
    match prefixed.as_slice() {
        [one] => Ok(one),
        [] => Err(SelectError::NotFound { spec: spec.to_string(), candidates: names(&mine) }),
        many => Err(SelectError::Ambiguous { spec: spec.to_string(), matches: names(many) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(id: &str, kind: DeviceKind, name: &str, state: &str) -> DeviceInfo {
        DeviceInfo { id: id.into(), kind, name: name.into(), state: state.into(), detail: vec![] }
    }

    #[test]
    fn parses_specs_and_selects() {
        assert_eq!(DeviceSpec::parse("ios-device:0000-1").expect("spec").kind, Some(DeviceKind::IosDevice));
        assert_eq!(DeviceSpec::parse("emulator-5554").expect("spec"), DeviceSpec { kind: None, id: "emulator-5554".into() });
        assert!(DeviceSpec::parse("ios-device:").is_err());
        assert!(DeviceSpec::parse("ios-device:x").expect("spec").check_platform(Platform::Android).is_err());

        let all = vec![
            d("4EC1-AAAA", DeviceKind::Simulator, "iPhone 16", "Booted"),
            d("9E60-BBBB", DeviceKind::Simulator, "iPhone 16 Plus", "Shutdown"),
            d("00008110-001C", DeviceKind::IosDevice, "Yogi's iPhone", "paired"),
            d("emulator-5554", DeviceKind::Emulator, "sdk_gphone64", "device"),
            d("R5CT1", DeviceKind::AndroidDevice, "SM-A536E", "unauthorized"),
        ];
        // No spec: the booted simulator, never the physical iPhone; the only usable Android.
        assert_eq!(select(&all, None, Platform::Ios).expect("ios").id, "4EC1-AAAA");
        assert_eq!(select(&all, None, Platform::Android).expect("android").id, "emulator-5554");
        let spec = |s: &str| DeviceSpec::parse(s).expect("spec");
        assert_eq!(select(&all, Some(&spec("ios-device:00008110-001C")), Platform::Ios).expect("dev").kind, DeviceKind::IosDevice);
        assert_eq!(select(&all, Some(&spec("iPhone 16")), Platform::Ios).expect("name").id, "4EC1-AAAA");
        assert_eq!(select(&all, Some(&spec("yogi")), Platform::Ios).expect("prefix").id, "00008110-001C");
        assert!(matches!(select(&all, Some(&spec("iphone 16")), Platform::Ios), Err(SelectError::Ambiguous { .. })));
        assert!(matches!(select(&all, Some(&spec("nope")), Platform::Ios), Err(SelectError::NotFound { .. })));
        assert!(matches!(select(&all, Some(&spec("simulator:00008110-001C")), Platform::Ios), Err(SelectError::NotFound { .. })));
        let two = vec![d("a", DeviceKind::Emulator, "x", "device"), d("b", DeviceKind::AndroidDevice, "y", "device")];
        assert!(matches!(select(&two, None, Platform::Android), Err(SelectError::Several { .. })));
    }
}
