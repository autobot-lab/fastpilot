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

/// v0.6.0 gate (FastRelease, message 4455): the web check ran every suite with `--release`, where
/// the fake-Jira test address is off on purpose (crates/core/src/jira.rs, Security Low 8), so the
/// five fake-Jira tests could only fail; and the Android app map (map_android) ran in the web check,
/// where adb is out of reach. v0.7.0 gate (5003b32): `jira_signin` (the fake Atlassian sign-in) and
/// `one_command` (a pretend terminal) failed the same way, because their hooks are debug-only too.
/// The suites the script picks: every suite with a debug-only hook in its own debug run,
/// map_android only in the Android check.
#[test]
fn debug_only_suites_run_in_debug_and_the_android_map_runs_with_android() {
    let script = std::fs::read_to_string(SCRIPT).expect("script");
    let pick: String = script.lines().skip_while(|l| !l.starts_with("ANDROID_DEMO=")).take_while(|l| !l.starts_with("check_web() {")).collect::<Vec<_>>().join("\n");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(format!("{pick}\necho \"web:$WEB_SUITES\"\necho \"debug:${{DEBUG_SUITES:-}}\""))
        .current_dir(root)
        .output()
        .expect("bash");
    let text = String::from_utf8_lossy(&out.stdout);
    let suites = |k: &str| -> Vec<String> { text.lines().find_map(|l| l.strip_prefix(k)).unwrap_or("").split_whitespace().map(str::to_owned).collect() };
    let (web, debug) = (suites("web:"), suites("debug:"));
    assert!(web.len() > 20, "the web suites are picked: {text}");

    // The debug-only hooks, written in pieces so that this file does not name them itself (the script
    // looks for the names in the suites' text).
    let hooks = [
        concat!("FASTPILOT_JIRA", "_TEST_BASE"),
        concat!("FASTPILOT_ATLASSIAN", "_TEST_BASE"),
        concat!("FASTPILOT_ASSUME", "_TTY"),
    ];
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut with_hook = Vec::new();
    for e in std::fs::read_dir(&tests).expect("tests") {
        let p = e.expect("entry").path();
        if p.extension().is_some_and(|x| x == "rs") {
            let text = std::fs::read_to_string(&p).expect("suite");
            let fake_jira = text.lines().any(|l| l.starts_with("#[path = \"support/fake_jira.rs\"]"));
            if fake_jira || hooks.iter().any(|h| text.contains(h)) {
                with_hook.push(p.file_stem().expect("name").to_string_lossy().into_owned());
            }
        }
    }
    with_hook.sort();
    for s in ["jira_chat", "jira_connect", "jira_connect_live", "jira_tools", "jira_signin", "one_command"] {
        assert!(with_hook.iter().any(|f| f == s), "{s} needs a debug-only hook: {with_hook:?}");
    }
    for s in &with_hook {
        assert!(debug.contains(s), "{s} needs a debug-only hook, so it runs in the debug build: debug={debug:?}");
        assert!(!web.contains(s), "{s} is not in the release-build web run, where its hook is off: {web:?}");
    }
    assert_eq!(debug, with_hook, "only the suites with a debug-only hook leave the release-build run");
    assert!(!web.iter().any(|s| s == "map_android"), "the Android app map needs an emulator, not the web check: {web:?}");

    let body = |name: &str| -> String { script.lines().skip_while(|l| *l != format!("check_{name}() {{")).take_while(|l| *l != "}").collect::<Vec<_>>().join("\n") };
    let web_check = body("web");
    let debug_run = web_check.lines().find(|l| l.contains("cargo test") && l.contains("\"${jira[@]}\"")).expect("the web check runs the fake-Jira suites");
    assert!(!debug_run.contains("--release"), "the fake-Jira suites run in a debug build: {debug_run}");
    assert!(body("android").contains("--test map_android"), "the Android check runs the Android app map");
}
