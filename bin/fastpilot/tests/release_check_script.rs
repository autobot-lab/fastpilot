//! FastTest M3 (F7): the release check's "Since the last full run" compares with the newest earlier
//! run that ran in full, never with a `--skip` or interrupted run. `--previous-full DIR` prints the
//! run it would use.

use std::path::Path;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/release-check.sh");

fn run(dir: &Path, name: &str, rows: &[(&str, &str, &str)]) {
    let d = dir.join(name);
    std::fs::create_dir_all(&d).expect("dir");
    std::fs::write(d.join("meta.tsv"), "abc1234\tmain\t2026-11-20 10:00\n").expect("meta");
    let tsv: String = rows.iter().map(|(k, st, detail)| format!("{k}\t{st}\t{detail}\t10\n")).collect();
    std::fs::write(d.join("results.tsv"), tsv).expect("results");
}

#[test]
fn since_the_last_run_means_the_last_full_run() {
    let dir = std::env::temp_dir().join(format!("fp-release-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let full = [("code", "passed", "clean"), ("unit", "passed", "164 passed"), ("web", "passed", "85 passed"), ("android", "skipped", "skipped: no Android emulator (emulator-5556 is not running)"), ("ios", "passed", "3 passed")];
    run(&dir, "20261120-100000", &full);
    run(&dir, "20261120-110000", &[("code", "passed", "clean"), ("unit", "passed", "164 passed"), ("web", "passed", "85 passed"), ("android", "skipped", "skipped: --skip android"), ("ios", "skipped", "skipped: --skip ios")]);
    run(&dir, "20261120-120000", &[("code", "passed", "clean"), ("unit", "skipped", "skipped: interrupted"), ("web", "skipped", "skipped: interrupted"), ("android", "skipped", "skipped: interrupted"), ("ios", "skipped", "skipped: interrupted")]);
    let out = std::process::Command::new("bash").arg(SCRIPT).arg("--previous-full").arg(&dir).output().expect("bash");
    let chosen = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    assert!(chosen.ends_with("20261120-100000"), "a device that was missing is a full run; --skip and interrupted runs are not: {chosen:?}");
    // Nothing full yet: no comparison.
    let only_partial = dir.join("only-partial");
    run(&only_partial, "20261120-110000", &[("code", "passed", "clean"), ("ios", "skipped", "skipped: --skip ios")]);
    let out = std::process::Command::new("bash").arg(SCRIPT).arg("--previous-full").arg(&only_partial).output().expect("bash");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "");
    let _ = std::fs::remove_dir_all(&dir);
}

/// v0.4.0 gate: `fastpilot run --workers` in the web tests listed every attached device and sent a
/// read-only getprop to emulator-5554. Only the Android check may reach an Android device: every
/// other check runs its cargo commands through `noandroid` (no adb, no SDK, an adb server address
/// where nothing listens), and the Android check pins its serial.
#[test]
fn only_the_android_check_reaches_an_android_device() {
    let script = std::fs::read_to_string(SCRIPT).expect("script");
    let noandroid = script.lines().skip_while(|l| !l.starts_with("noandroid() {")).take_while(|l| *l != "}").collect::<Vec<_>>().join("\n");
    for part in ["ANDROID_HOME=\"$NO_ANDROID_SDK\"", "ANDROID_SDK_ROOT=\"$NO_ANDROID_SDK\"", "FASTPILOT_ADB_SERVER=127.0.0.1:1", "PATH=\"$NO_ADB_PATH\""] {
        assert!(noandroid.contains(part), "noandroid sets {part}: {noandroid}");
    }
    let mut check = "";
    let mut cargo_lines = 0;
    let lines: Vec<&str> = script.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if let Some(name) = line.strip_prefix("check_").and_then(|r| r.strip_suffix("() {")) {
            check = name;
        }
        if line.trim_start().starts_with('#') || check.is_empty() || (!line.contains("cargo clippy") && !line.contains("cargo test")) {
            continue;
        }
        cargo_lines += 1;
        // A command may continue the line before it (`env X=… \` then `cargo test …`).
        let start = if i > 0 && lines[i - 1].trim_end().ends_with('\\') { lines[i - 1] } else { line };
        if check == "android" {
            assert!(start.contains("tenv env FASTPILOT_ANDROID_SERIAL=\"$SERIAL\" ANDROID_SERIAL=\"$SERIAL\""), "the Android check pins its serial: {start}");
        } else {
            assert!(start.trim_start().starts_with("noandroid "), "check_{check} keeps Android out of reach: {start}");
        }
    }
    assert!(cargo_lines >= 7, "every check's cargo command was looked at ({cargo_lines})");
}
