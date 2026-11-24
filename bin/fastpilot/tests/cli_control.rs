//! v0.7.1 FC2: what the dashboard's pages do, from the terminal: apps (`fastpilot app`), one-click
//! schedules with pause and resume (`fastpilot schedule add|pause|resume`), and the current app's
//! passwords (`fastpilot secret`), typed hidden or piped in, never an argument, kept in the same
//! store and under the same names as the dashboard's masked field.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn home() -> PathBuf {
    let h = std::env::temp_dir().join(format!("fp-cli-control-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(&h).expect("home");
    std::fs::canonicalize(&h).expect("home")
}

/// `fastpilot ARGS` in `home` (stdin piped from `input`): (exit code, stdout + stderr).
fn fp(home: &Path, args: &[&str], input: &str) -> (i32, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_SCHEDULER", "launchd")
        .env("FASTPILOT_SCHEDULER_DIR", home.join("agents"))
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env_remove("FASTPILOT_VARS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("fastpilot");
    c.stdin.take().expect("stdin").write_all(input.as_bytes()).expect("input");
    let o = c.wait_with_output().expect("wait");
    (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

fn apps_json(home: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(home.join("fastpilot/apps.json")).expect("apps.json")).expect("json")
}

#[test]
fn apps_passwords_and_schedules_from_the_terminal() {
    let home = home();

    // `--help` lists the new commands.
    let (_, help) = fp(&home, &["--help"], "");
    assert!(help.contains("app [list|add|switch|remove]") && help.contains("secret [list|set NAME|remove NAME]") && help.contains("schedule [list|add|pause|resume|run|remove]"), "{help}");

    // Apps: a website, an app id needs its kind, then an Android app with a name.
    let (code, out) = fp(&home, &["app", "add", "https://shop.example"], "");
    assert!(code == 0 && out.contains("It is the current app."), "{out}");
    let (code, out) = fp(&home, &["app", "add", "com.shop.app"], "");
    assert!(code != 0 && out.contains("looks like an app id") && out.contains("--android or --ios"), "{out}");
    let (code, out) = fp(&home, &["app", "add", "com.shop.app", "--android", "--name", "Shop Android"], "");
    assert!(code == 0 && out.contains("Added \"Shop Android\""), "{out}");
    let shop = apps_json(&home)["apps"][0]["id"].as_str().expect("id").to_owned();
    let (_, list) = fp(&home, &["app", "list"], "");
    assert!(list.lines().any(|l| l.starts_with("* ") && l.contains("https://shop.example")) && list.contains("Android com.shop.app"), "the first app is the current one: {list}");
    let (code, out) = fp(&home, &["app", "switch", "shop android"], "");
    assert!(code == 0 && out.contains("Now working on \"Shop Android\""), "{out}");
    assert_eq!(apps_json(&home)["current"], "shop-android");
    assert_eq!(fp(&home, &["app", "switch", &shop], "").0, 0);

    // Passwords: piped in (a person types it hidden), never an argument; the dashboard's store.
    let (code, out) = fp(&home, &["secret", "set", "TEST_PASSWORD", "hunter2-in-history"], "");
    assert!(code != 0 && out.contains("never given on the command line"), "{out}");
    let (code, out) = fp(&home, &["secret", "set", "TEST_PASSWORD"], "fp-cli-pass-77\n");
    assert!(code == 0 && out.contains("TEST_PASSWORD is saved for") && !out.contains("fp-cli-pass-77"), "{out}");
    let (code, out) = fp(&home, &["secret", "set", "SHORT"], "abc\n");
    assert!(code != 0, "too short: {out}");
    let (_, list) = fp(&home, &["secret", "list"], "");
    assert!(list.contains("TEST_PASSWORD") && list.contains("saved") && !list.contains("fp-cli-pass-77"), "{list}");
    let kept = home.join(format!(".fastpilot/keys/secrets/{shop}/TEST_PASSWORD"));
    assert!(kept.is_file(), "the dashboard's store: {}", kept.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&kept).expect("kept").permissions().mode() & 0o777, 0o600);
    }
    assert_eq!(apps_json(&home)["apps"][0]["secrets"], serde_json::json!(["TEST_PASSWORD"]));
    let config = std::fs::read_to_string(home.join(".fastpilot/config.toml")).unwrap_or_default();
    assert!(!config.contains("TEST_PASSWORD"), "not on the environment list: {config}");
    let log = std::fs::read_to_string(home.join(".fastpilot/audit.jsonl")).unwrap_or_default();
    assert!(log.contains("secret TEST_PASSWORD of") && log.contains("terminal") && !log.contains("fp-cli-pass-77"), "{log}");

    // Schedules: no saved tests yet, then the nightly preset, pause, resume, remove.
    let (code, out) = fp(&home, &["schedule", "add", "--nightly"], "");
    assert!(code != 0 && out.contains("no saved tests yet"), "{out}");
    let tests = home.join(format!("fastpilot/apps/{shop}/tests"));
    std::fs::create_dir_all(&tests).expect("tests");
    std::fs::write(tests.join("login.md"), "# Login works\nid: login\nplatform: web\ntarget: https://shop.example\n- Open the page\n").expect("test");
    let (code, out) = fp(&home, &["schedule", "add", "--nightly"], "");
    assert!(code == 0 && out.contains("Created \"All tests · every night\""), "{out}");
    let id = out.split('(').nth(1).and_then(|r| r.split(')').next()).expect("id").to_owned();
    let entries = || std::fs::read_dir(home.join("agents")).map(|d| d.count()).unwrap_or(0);
    assert_eq!(entries(), 1, "installed with the system scheduler");
    let (code, out) = fp(&home, &["schedule", "add", "--nightly"], "");
    assert!(code == 0 && out.contains("Turned on"), "not made twice: {out}");
    let (code, out) = fp(&home, &["schedule", "pause", &id], "");
    assert!(code == 0 && out.contains("is off") && entries() == 0, "{out}");
    let (_, list) = fp(&home, &["schedule", "list"], "");
    assert!(list.contains(&format!("○ {id}")), "{list}");
    let (code, out) = fp(&home, &["schedule", "resume", &id], "");
    assert!(code == 0 && out.contains("is on again") && entries() == 1, "{out}");
    let (code, out) = fp(&home, &["schedule", "remove", "All tests · every night"], "");
    assert!(code == 0 && out.contains("Removed") && entries() == 0, "by its name: {out}");

    // Removing an app asks first; without a terminal and without --yes nothing changes.
    let (code, out) = fp(&home, &["app", "remove", "Shop Android"], "");
    assert!(code != 0 && out.contains("Nothing removed"), "{out}");
    let (code, out) = fp(&home, &["app", "remove", "Shop Android", "--yes"], "");
    assert!(code == 0 && out.contains("Its folder stays"), "{out}");
    assert_eq!(apps_json(&home)["apps"].as_array().map(Vec::len), Some(1));
    let _ = std::fs::remove_dir_all(&home);
}
