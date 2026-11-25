//! E9 (v0.7.2): `fastpilot report` makes a shareable report of a run (a web page that works
//! offline and a Markdown file), saved with the run, in FastUI/UX's words (TEXT-terminal-mcp.md
//! 8.9). Every screenshot of a failed test and the last one of a passing test go in; passwords and
//! card numbers in the text show as ***; nothing is uploaded. A temp HOME and a History made on
//! disk; no model, no Chrome, no device.

use std::path::{Path, PathBuf};
use std::process::Command;

const JPEG_A: &[u8] = b"\xff\xd8\xff\xe0fake-fail-shot\xff\xd9";
const JPEG_B: &[u8] = b"\xff\xd8\xff\xe0fake-flow-shot\xff\xd9";
const JPEG_C: &[u8] = b"\xff\xd8\xff\xe0fake-last-pass-shot\xff\xd9";

fn home() -> PathBuf {
    let h = std::env::temp_dir().join(format!("fp-reports-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(&h).expect("home");
    std::fs::canonicalize(&h).expect("home")
}

fn fp(home: &Path, args: &[&str]) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("FASTPILOT_DIR", home.join(".fastpilot"))
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_VARS", "DEMO_PASS")
        .env("DEMO_PASS", "rahasia123")
        .env_remove("FASTPILOT_CONFIG")
        .output()
        .expect("fastpilot");
    (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

/// A suite of two tests as `fastpilot run` leaves it: a failed login (two screenshots, steps that
/// spell out a password and a card number) and a passing checkout (two screens, only the last
/// goes in the report).
fn seed(home: &Path) {
    let root = home.join(".fastpilot");
    let fail = root.join("runs/20261125-100000-login");
    let pass = root.join("runs/20261125-100100-checkout");
    std::fs::create_dir_all(&fail).expect("fail");
    std::fs::create_dir_all(&pass).expect("pass");
    std::fs::write(fail.join("report.md"), "# login — fail\n\n## Steps\n\n- typed rahasia123 into Password\n- paid with 4111 1111 1111 1111\n- expected the Dashboard, saw <b>Error</b>\n").expect("md");
    std::fs::write(fail.join("fail-1.jpg"), JPEG_A).expect("shot");
    std::fs::write(fail.join("flow-1.jpg"), JPEG_B).expect("shot");
    std::fs::write(pass.join("report.md"), "# checkout — pass\n").expect("md");
    std::fs::write(pass.join("flow-1.jpg"), JPEG_B).expect("shot");
    std::fs::write(pass.join("flow-2.jpg"), JPEG_C).expect("shot");
    std::fs::create_dir_all(root.join("history/suites")).expect("suites");
    let suite = serde_json::json!({ "id": "1795600000000", "title": "Run all tests", "platforms": ["web"], "started_ms": 1795599990000u64, "finished_ms": 1795600000000u64,
        "passed": 1, "failed": 1, "not_run": 0, "runs": ["20261125-100000-login", "20261125-100100-checkout"], "report": null, "report_owned": false });
    std::fs::write(root.join("history/suites/1795600000000.json"), suite.to_string()).expect("suite");
}

#[test]
fn a_report_of_the_newest_run_is_made_with_its_screenshots_and_no_secrets() {
    let home = home();
    // Nothing ran yet: the plain sentence, exit 1.
    let (code, out) = fp(&home, &["report"]);
    assert_eq!((code, out.trim()), (1, "✗ There is no run to report yet. Run a test first."));
    let (_, out) = fp(&home, &["report", "--list"]);
    assert_eq!(out.trim(), "No reports yet. Make one: fastpilot report");

    seed(&home);
    let (code, out) = fp(&home, &["report"]);
    assert_eq!(code, 0, "{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].starts_with("✓ Made report for “Run all tests” · ") && lines[0].ends_with(" · 1 of 2 failed · 3 screenshots"), "{out}");
    let dir = home.join(".fastpilot/history/reports/suite-1795600000000");
    assert_eq!(lines[1], format!("  Web page   {}", dir.join("report.html").display()));
    assert_eq!(lines[2], format!("  Markdown   {}", dir.join("report.md").display()));
    assert_eq!(&lines[3..], ["  Made on this Mac. Nothing was uploaded.", "  Screenshots show the page as it was. Look at them before you share."]);

    let html = std::fs::read_to_string(dir.join("report.html")).expect("html");
    let md = std::fs::read_to_string(dir.join("report.md")).expect("md");
    for text in [&html, &md] {
        assert!(!text.contains("rahasia123") && !text.contains("4111 1111 1111 1111"), "no password, no card number");
        assert!(text.contains("typed *** into Password") && text.contains("paid with ***"), "they show as ***");
    }
    // Every screenshot of the failed test, the last one of the passing test, inlined in the page.
    use base64::Engine;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    assert!(html.contains(&b64(JPEG_A)) && html.contains(&b64(JPEG_B)) && html.contains(&b64(JPEG_C)), "three screenshots inlined");
    assert_eq!(html.matches("<img ").count(), 3);
    assert!(html.contains("Content-Security-Policy") && !html.contains("<script") && html.contains("saw &lt;b&gt;Error&lt;/b&gt;"), "page text is text, nothing runs");
    assert!(html.contains("Screenshots show the page as it was. Look at them before you share.") && html.contains("Nothing was uploaded."));
    assert!(md.trim_end().ends_with("Screenshots show the page as it was. Look at them before you share."), "the last line of the Markdown");
    assert_eq!(std::fs::read(dir.join("shot-1.jpg")).expect("shot"), JPEG_A, "the Markdown's screenshots sit next to it");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join("report.html")).expect("html").permissions().mode() & 0o777, 0o600);
    }

    // --markdown prints only the Markdown; --list shows the run with its result.
    let (_, out) = fp(&home, &["report", "--markdown"]);
    assert_eq!(out, md);
    let (_, out) = fp(&home, &["report", "--list"]);
    assert!(out.starts_with("Reports\n") && out.contains("suite:1795600000000") && out.contains("1 of 2 failed"), "{out}");
    let (code, out) = fp(&home, &["report", "suite:999"]);
    assert_eq!((code, out.trim()), (1, "✗ No run with id “suite:999”. Run fastpilot report --list to see recent runs."));
    // A run whose screenshot cannot be read: the plain sentence, the run is kept.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bad = home.join(".fastpilot/runs/20261125-100000-login/fail-2.jpg");
        std::fs::write(&bad, JPEG_A).expect("shot");
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000)).expect("unreadable");
        let (code, out) = fp(&home, &["report"]);
        assert_eq!((code, out.trim()), (1, "✗ Couldn’t make the report: the screenshots of this run could not be read. The run itself is kept. Run the command again."));
        assert!(home.join(".fastpilot/runs/20261125-100000-login/report.md").is_file(), "the run is kept");
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o600)).expect("readable again");
    }
    let _ = std::fs::remove_dir_all(home);
}
