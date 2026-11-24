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
    // E1 (v0.4): --help shows the help, checks and changes nothing (it ran the checks before).
    for cmd in ["doctor", "setup"] {
        let (ok, out) = run(&[cmd, "--help"], Some("/nonexistent/chrome"));
        assert!(ok && out.starts_with(&format!("usage: fastpilot {cmd}")) && !out.contains("✓") && !out.contains("✗"), "{cmd} --help: {out}");
    }
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

/// QA (Low): unknown or `=value` options of `run` were ignored silently, and `open --help` opened
/// https://--help. Now: a clear message with the usage (exit 2), and the help.
#[test]
fn run_options_are_checked_and_help_is_help() {
    let dir = std::env::temp_dir().join(format!("fastpilot-cli-help-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).args(args).env("FASTPILOT_SOCKET", dir.join("d.sock")).output().expect("run");
        (out.status.code(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let (code, out) = run(&["run", "tests/login.md", "--reset=1"]);
    assert!(code == Some(2) && out.contains("--reset is a switch and takes no value") && out.contains("[--report-github[=dry]]"), "{code:?} {out}");
    let (code, out) = run(&["run", "tests/login.md", "--bogus"]);
    assert!(code == Some(2) && out.contains("unknown argument `--bogus`") && out.contains("[--record] [--reset]"), "{code:?} {out}");
    let (code, out) = run(&["run", "tests/login.md", "--report-github=bogus"]);
    assert!(code == Some(1) && out.contains("--report-github takes `dry` or nothing, not `bogus`"), "{code:?} {out}");
    let (code, out) = run(&["open", "--help"]);
    assert!(code == Some(0) && out.contains("fastpilot shell commands"), "{code:?} {out}");
    assert!(!dir.join("d.sock").exists(), "help must not start the daemon");
    let (_, out) = run(&["--help"]);
    assert!(out.contains("one-time link") && !out.contains("127.0.0.1 + token"), "{out}");
    // v0.4 (FastTest): the release commands are listed, and their help is help, not an error.
    assert!(out.contains("schedule watch TESTS --every MIN --for HOURS") && out.contains("ship-check [url]..."), "{out}");
    for args in [&["schedule", "watch", "--help"][..], &["schedule", "--help"], &["schedule", "help"]] {
        let (code, out) = run(args);
        assert!(code == Some(0) && out.starts_with("fastpilot schedule — run tests on a timer\n\nUsage: fastpilot schedule") && out.contains("watch TESTS --every MIN") && !out.contains("not found"), "{args:?}: {code:?} {out}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// QA B1 (real Claude, v0.4): Claude Code 2.1.144 started, so `init`/`doctor` said ✓, and then every
/// Chat message failed with "version 2.1.280 or newer is required". Now it is a ✗ with the fix.
#[test]
fn an_old_claude_code_is_a_failure_with_the_fix() {
    let fake = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude");
    let doctor = |version: &str| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .args(["doctor", "run"])
            .current_dir(ROOT)
            .env("FASTPILOT_CLAUDE", fake)
            .env("FAKE_CLAUDE_VERSION", version)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("doctor");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let (ok, out) = doctor("2.1.144");
    assert!(!ok && out.contains("✗") && out.contains("2.1.144 (Claude Code, fake): too old") && out.contains("run `claude update` in Terminal"), "{out}");
    let (ok, out) = doctor("2.1.280");
    assert!(ok && out.contains("✓") && !out.contains("too old"), "{out}");
}

/// QA v0.6 Low 8: `fastpilot … | head -1` must not panic with "Broken pipe" when the reader stops
/// early. The read end is closed before fastpilot starts, so its first print meets a closed pipe.
#[cfg(unix)]
#[test]
fn a_closed_pipe_stops_quietly_without_a_panic() {
    let home = std::env::temp_dir().join(format!("fp-cli-pipe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    for args in [&["--version"][..], &["config", "path"][..], &["doctor", "web"][..]] {
        let (reader, writer) = std::io::pipe().expect("pipe");
        drop(reader);
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .args(args)
            .env("HOME", &home)
            .env("FASTPILOT_KEYSTORE", "file")
            .env_remove("FASTPILOT_CONFIG")
            .stdout(writer)
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("run");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains("panicked") && !err.contains("Broken pipe"), "{args:?}: {err}");
        assert_eq!(out.status.code(), Some(141), "{args:?}: stops like a tool stopped by SIGPIPE; stderr: {err}");
    }
    let _ = std::fs::remove_dir_all(&home);
}
