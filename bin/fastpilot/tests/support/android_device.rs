//! The Android device the device suites run on (F6, v0.4): named on purpose, never guessed, and
//! never skipped in silence. Shared by tests/android.rs, tests/ui_takeover_device.rs and
//! tests/demo_cli.rs through `#[path]`.
//!
//! - `FASTPILOT_ANDROID_SERIAL` (or `ANDROID_SERIAL`) names the emulator or phone the tests may use.
//!   Without it the suite fails and lists what is attached: picking "the first device" could run
//!   the tests (which install apps and change settings) on someone's own emulator or phone.
//! - No device, a named device that is not ready, or no adb: the suite fails with what to do.
//! - `FASTPILOT_SKIP_ANDROID=1` skips the Android tests on purpose, and says so.

#![allow(dead_code)]

use std::sync::OnceLock;

/// adb from ANDROID_HOME / ANDROID_SDK_ROOT, else the default SDK place, else `adb` on PATH.
pub fn adb() -> String {
    let home = std::env::var("ANDROID_HOME")
        .or_else(|_| std::env::var("ANDROID_SDK_ROOT"))
        .unwrap_or_else(|_| format!("{}/Library/Android/sdk", std::env::var("HOME").unwrap_or_default()));
    let path = format!("{home}/platform-tools/adb");
    if std::path::Path::new(&path).exists() { path } else { "adb".to_owned() }
}

/// The serial to test on; `None` only when skipped on purpose (FASTPILOT_SKIP_ANDROID=1). Every other
/// case without a usable device panics with the reason and the fix.
pub fn device() -> Option<String> {
    static CHOSEN: OnceLock<Result<Option<String>, String>> = OnceLock::new();
    match CHOSEN.get_or_init(choose) {
        Ok(Some(s)) => Some(s.clone()),
        Ok(None) => {
            eprintln!("SKIPPED on purpose: FASTPILOT_SKIP_ANDROID=1 (the Android tests did not run)");
            None
        }
        Err(why) => panic!("{why}"),
    }
}

fn choose() -> Result<Option<String>, String> {
    if std::env::var("FASTPILOT_SKIP_ANDROID").is_ok_and(|v| v == "1") {
        return Ok(None);
    }
    let skip = "To skip the Android tests on purpose, set FASTPILOT_SKIP_ANDROID=1.";
    let out = std::process::Command::new(adb())
        .arg("devices")
        .output()
        .map_err(|e| format!("Android tests cannot run: adb not found ({e}). Install the Android platform-tools (`fastpilot setup android`). {skip}"))?;
    let attached: Vec<(String, String)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter_map(|l| l.split_once('\t').map(|(s, st)| (s.trim().to_owned(), st.trim().to_owned())))
        .collect();
    let named = ["FASTPILOT_ANDROID_SERIAL", "ANDROID_SERIAL"].iter().find_map(|k| std::env::var(k).ok().filter(|s| !s.is_empty()));
    let Some(serial) = named else {
        let list = if attached.is_empty() { "none".to_owned() } else { attached.iter().map(|(s, st)| format!("{s} ({st})")).collect::<Vec<_>>().join(", ") };
        return Err(if attached.is_empty() {
            format!("Android tests cannot run: no Android device or emulator is attached. Start an emulator (`fastpilot setup android`), then set FASTPILOT_ANDROID_SERIAL to it. {skip}")
        } else {
            format!(
                "Android tests cannot run: no device chosen. Set FASTPILOT_ANDROID_SERIAL to the emulator these tests may use (attached: {list}); they never pick one by themselves, because they install apps and change settings. {skip}"
            )
        });
    };
    match attached.iter().find(|(s, _)| *s == serial) {
        Some((_, st)) if st == "device" => Ok(Some(serial)),
        Some((_, st)) => Err(format!("Android tests cannot run: {serial} is attached but not ready ({st}). Unlock it or accept the USB debugging prompt, then run again. {skip}")),
        None => Err(format!("Android tests cannot run: FASTPILOT_ANDROID_SERIAL={serial} is not attached (`adb devices`). Start it, or name another one. {skip}")),
    }
}

/// A file the Android tests need (the demo app, the agent): missing means the suite fails, saying
/// how to build it.
pub fn need_file(path: &str, what: &str) {
    assert!(std::path::Path::new(path).exists(), "Android tests cannot run: {what} is not built ({path}). Run agent-android/build.sh, then run the tests again.");
}
