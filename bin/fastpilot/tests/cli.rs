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
    // ... and fails while a ✗ is left (here web, which no setup run can fix without a terminal).
    let (ok, out) = run(&["setup"], Some("/nonexistent/chrome"));
    assert!(!ok && !out.contains("  done") && !out.contains("  started"), "{out}");
    if out.contains("can be applied") {
        assert!(out.contains("plan (run in a terminal"), "{out}");
    }
    if std::path::Path::new("/Applications/Google Chrome.app").exists() || std::path::Path::new("/usr/bin/chromium").exists() {
        let (ok, out) = run(&["doctor", "web"], None);
        assert!(ok && out.contains("✓ Chrome") && out.contains("starts headless"), "{out}");
        let (ok, out) = run(&["setup", "web"], None);
        assert!(ok && out.contains("nothing to set up"), "web ready, nothing left: {out}");
    }
}
