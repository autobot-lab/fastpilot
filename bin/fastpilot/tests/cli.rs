//! CLI subcommands that need no MCP client. Skips without Chrome.

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

#[test]
fn bench_tokens_reports_snapshot_diff_and_flow() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["bench", "tokens", "web"])
        .current_dir(ROOT)
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("Chrome") {
        return eprintln!("skipping: no Chrome");
    }
    assert!(out.status.success(), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("counter: approximation"), "{text}");
    let tokens = |row: &str| -> u64 {
        let line = text.lines().find(|l| l.starts_with(row)).unwrap_or_else(|| panic!("no row {row:?} in\n{text}"));
        line.split_whitespace().last().and_then(|t| t.parse().ok()).expect("token count")
    };
    let (snap, diff, flow) = (tokens("snapshot"), tokens("run_flow, 1 step"), tokens("run_flow, login"));
    assert!(snap > 20 && diff > 10 && flow > diff, "{text}");
    // The point of text snapshots: far cheaper than a screenshot.
    assert!(tokens("screenshot") > 5 * snap, "{text}");
}

#[test]
fn doctor_reports_each_platform_and_setup_only_plans_without_a_terminal() {
    let run = |args: &[&str], chrome: Option<&str>| {
        let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
        c.args(args).current_dir(ROOT).stdin(std::process::Stdio::null());
        if let Some(p) = chrome {
            c.env("FASTPILOT_CHROME", p);
        }
        let out = c.output().expect("run");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    // A Chrome that does not exist: web fails, with the fix.
    let (ok, out) = run(&["doctor", "web"], Some("/nonexistent/chrome"));
    assert!(!ok && out.contains("✗ Chrome") && out.contains("does not start") && out.contains("fastpilot config set chrome"), "{out}");
    // Unknown platform.
    let (ok, out) = run(&["doctor", "windows"], None);
    assert!(!ok && out.contains("unknown platform"), "{out}");
    // Every area is listed; `setup` without a terminal never runs anything.
    let (_, out) = run(&["doctor"], None);
    for area in ["web:", "android:", "ios:", "run:"] {
        assert!(out.contains(area), "{area} missing in\n{out}");
    }
    // No platform named: a ✗ in web fails `doctor` (as it fails `setup`), so CI can rely on it.
    let (ok, out) = run(&["doctor"], Some("/nonexistent/chrome"));
    assert!(!ok && out.contains("✗ Chrome"), "plain doctor with web ✗ must exit ≠ 0: {out}");
    // ... and fails while a ✗ is left (here web, which no setup run can fix without a terminal).
    let (ok, out) = run(&["setup"], Some("/nonexistent/chrome"));
    assert!(!ok && !out.contains("  done") && !out.contains("  started"), "{out}");
    if out.contains("can be applied") {
        assert!(out.contains("plan (run in a terminal"), "{out}");
    }
    if std::path::Path::new("/Applications/Google Chrome.app").exists() || std::path::Path::new("/usr/bin/chromium").exists() {
        let (ok, out) = run(&["doctor", "web"], None);
        assert!(ok && out.contains("✓ Chrome") && out.contains("starts headless"), "{out}");
        // Web ready: a ✗ in a platform nobody named (Android, iOS, run) does not fail a plain `doctor`.
        let (ok, out) = run(&["doctor"], None);
        assert!(ok, "plain doctor with web ✓ exits 0: {out}");
        let (ok, out) = run(&["setup", "web"], None);
        assert!(ok && out.contains("nothing to set up"), "web ready, nothing left: {out}");
    }
}

/// Chrome starts `fastpilot chrome-extension://<id>/` as the native messaging host: the relay token
/// goes to the fastpilot extension only (length-prefixed JSON on stdin/stdout).
#[test]
fn native_host_hands_the_relay_token_to_the_fastpilot_extension_only() {
    use std::io::{Read, Write};
    let home = std::env::temp_dir().join(format!("fastpilot-native-host-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".fastpilot/relay")).expect("dir");
    std::fs::write(home.join(".fastpilot/relay/9555.token"), "c0ffee\n").expect("token");
    let ask = |origin: &str, body: &str| -> String {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .arg(origin)
            .env("HOME", &home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn");
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(&(body.len() as u32).to_le_bytes()).expect("len");
        stdin.write_all(body.as_bytes()).expect("body");
        drop(stdin);
        let mut out = Vec::new();
        child.stdout.take().expect("stdout").read_to_end(&mut out).expect("read");
        assert!(child.wait().expect("wait").success());
        let len = u32::from_le_bytes(out[..4].try_into().expect("4 bytes")) as usize;
        assert_eq!(out.len(), 4 + len, "one framed reply");
        String::from_utf8(out[4..].to_vec()).expect("utf-8")
    };
    let ours = "chrome-extension://iehmgkdbiojkipljgbaljojdmdfpjeeg/";
    assert_eq!(ask(ours, r#"{"port":9555}"#), r#"{"token":"c0ffee"}"#);
    let other = ask("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/", r#"{"port":9555}"#);
    assert!(other.contains("not the fastpilot extension") && !other.contains("c0ffee"), "{other}");
    assert!(ask(ours, r#"{"port":9556}"#).contains("no fastpilot relay is waiting on port 9556"));
    assert!(ask(ours, "nope").contains("error"));
    let _ = std::fs::remove_dir_all(home);
}
