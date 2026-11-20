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
