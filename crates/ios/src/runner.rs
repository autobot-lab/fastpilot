//! The XCUITest runner build: which `.xctestrun` to use (simulator or device build, newest
//! first), starting it with `xcodebuild test-without-building` while keeping the output's
//! tail, and turning known xcodebuild / devicectl failures into the fix the user needs.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Simulator,
    Device,
}

/// The simulator runner `fastpilot setup ios` builds (F3): `~/.fastpilot/ios-runner/build`, next to
/// the settings file. Installs from npm or a release have no checkout to build it in.
pub fn setup_dir() -> PathBuf {
    fastpilot_core::config::path().parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join("ios-runner")
}

/// The simulator runner build: `FASTPILOT_IOS_RUNNER` (`ios_runner`), else a checkout's
/// `ios-runner/build` when this binary was built from one and it is there, else the one
/// `fastpilot setup ios` builds. Device builds: `ios-runner/build-device` /
/// `FASTPILOT_IOS_RUNNER_DEVICE`.
pub fn build_dir(target: Target) -> PathBuf {
    match target {
        Target::Simulator => fastpilot_core::config::var_os("FASTPILOT_IOS_RUNNER").map(PathBuf::from).unwrap_or_else(|| {
            let checkout = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ios-runner/build"));
            if pick_xctestrun(&checkout.join("Build/Products"), Target::Simulator).is_some() { checkout } else { setup_dir().join("build") }
        }),
        Target::Device => crate::device::device_build_dir(),
    }
}

/// Newest `.xctestrun` for the target's SDK (the products folder can hold both kinds).
pub fn xctestrun(target: Target) -> Option<PathBuf> {
    pick_xctestrun(&build_dir(target).join("Build/Products"), target)
}

pub fn pick_xctestrun(products: &Path, target: Target) -> Option<PathBuf> {
    let sdk = match target {
        Target::Simulator => "iphonesimulator",
        Target::Device => "iphoneos",
    };
    std::fs::read_dir(products)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xctestrun") && p.file_name().is_some_and(|n| n.to_string_lossy().contains(sdk)))
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// The fix for a known failure in xcodebuild / devicectl output.
pub fn classify(output: &str) -> Option<&'static str> {
    let o = output.to_ascii_lowercase();
    Some(if o.contains("not been explicitly trusted") || o.contains("untrusted developer") {
        "on the phone: Settings → General → VPN & Device Management → trust your developer certificate"
    } else if o.contains("maximum number of installed apps") || o.contains("maximum number of apps") {
        "a free Apple ID allows 3 apps signed by you per device: delete one of them from the phone"
    } else if (o.contains("profile") && o.contains("expired")) || o.contains("no profile") || o.contains("no provisioning profile") {
        "the signing profile expired (free profiles last 7 days): run ios-runner/build.sh --device <udid> again"
    } else if o.contains("developer mode") {
        "Settings → Privacy & Security → Developer Mode → on, then restart the phone"
    } else if o.match_indices("locked").any(|(i, _)| !o[..i].ends_with("un")) {
        "unlock the phone (Auto-Lock → Never while testing)"
    } else if ["newer version of xcode", "not supported by this version of xcode", "doesn't support ios", "does not support ios", "unsupported os version"]
        .iter()
        .any(|p| o.contains(p))
    {
        "this Xcode is too old for the device's iOS: install the matching Xcode and xcode-select it"
    } else {
        return None;
    })
}

/// A started runner and the last lines it printed (stdout and stderr).
pub struct Spawned {
    pub child: Child,
    pub tail: Arc<Mutex<VecDeque<String>>>,
}

impl Spawned {
    pub fn tail_text(&self) -> String {
        self.tail.lock().unwrap_or_else(|p| p.into_inner()).iter().cloned().collect::<Vec<_>>().join(" | ")
    }
}

/// `xcodebuild test-without-building -xctestrun <x> -destination id=<udid>` with
/// `TEST_RUNNER_*` environment for the runner; output tails are kept for error messages.
/// Device runners only: the child dies with its handle (a failed start or the session's end).
pub fn spawn(xctestrun: &Path, udid: &str, env: &[(String, String)]) -> std::io::Result<Spawned> {
    let mut cmd = Command::new("xcodebuild");
    cmd.arg("test-without-building").arg("-xctestrun").arg(xctestrun).args(["-destination", &format!("id={udid}")]);
    for (k, v) in env {
        cmd.env(format!("TEST_RUNNER_{k}"), v);
    }
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn()?;
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let keep = |stream: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>, tail: Arc<Mutex<VecDeque<String>>>| {
        if let Some(s) = stream {
            tokio::spawn(async move {
                let mut lines = BufReader::new(s).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    let mut t = tail.lock().unwrap_or_else(|p| p.into_inner());
                    t.push_back(l);
                    if t.len() > 40 {
                        t.pop_front();
                    }
                }
            });
        }
    };
    keep(child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), tail.clone());
    keep(child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), tail.clone());
    Ok(Spawned { child, tail })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_per_platform_and_classifies() {
        let dir = std::env::temp_dir().join(format!("fastpilot-xctestrun-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("FastpilotRunner_iphonesimulator18.1-arm64.xctestrun"), "").expect("w");
        std::fs::write(dir.join("FastpilotRunnerDevice_iphoneos26.0-arm64.xctestrun"), "").expect("w");
        let sim = pick_xctestrun(&dir, Target::Simulator).expect("sim");
        assert!(sim.to_string_lossy().contains("iphonesimulator"));
        assert!(pick_xctestrun(&dir, Target::Device).expect("dev").to_string_lossy().contains("iphoneos26"));
        let _ = std::fs::remove_dir_all(dir);
        assert!(classify("The application could not be launched because the Developer App Certificate is not trusted. ... has not been explicitly trusted").is_some_and(|h| h.contains("VPN & Device Management")));
        assert!(classify("This device has reached the maximum number of installed apps using a free developer profile").is_some_and(|h| h.contains("3 apps")));
        assert!(classify("Provisioning profile \"iOS Team Provisioning Profile\" expired").is_some_and(|h| h.contains("7 days")));
        assert!(classify("Developer Mode disabled").is_some());
        assert!(classify("Device is locked").is_some_and(|h| h.contains("unlock")));
        assert_eq!(classify("device unlocked; Xcode 26.0 build version 17A324"), None, "neither locked nor an Xcode mismatch");
        assert!(classify("iPhone is running iOS 26.6.1, which is not supported by this version of Xcode").is_some_and(|h| h.contains("matching Xcode")));
        assert_eq!(classify("** TEST EXECUTE SUCCEEDED **"), None);
    }
}
