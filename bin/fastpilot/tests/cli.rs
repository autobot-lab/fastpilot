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
