//! A2 TP1 (v0.4.2): npm Trusted Publishing from `release.yml`. The `publish-npm` job runs only
//! for a tag and after `release` (the package downloads its binary from the GitHub Release), gets
//! an OIDC identity and nothing more (`id-token: write`, `contents: read`), uses no token or
//! secret, pins its actions to commits, checks that the tag is the package version, and says what
//! to configure on npmjs.com when npm refuses. A dry run (workflow_dispatch, no tag) publishes
//! nothing.

use serde_yaml::Value;

const WORKFLOW: &str = include_str!("../../../.github/workflows/release.yml");

#[test]
fn npm_is_published_by_the_release_workflow_without_a_token() {
    let wf: Value = serde_yaml::from_str(WORKFLOW).expect("release.yml is YAML");
    let job = &wf["jobs"]["publish-npm"];
    assert!(job.is_mapping(), "a publish-npm job");
    assert_eq!(job["if"].as_str(), Some("github.event_name == 'push' && github.ref_type == 'tag'"), "only a pushed tag: a manual run publishes nothing");
    assert_eq!(wf["jobs"]["release"]["if"].as_str(), Some("github.event_name == 'push' && github.ref_type == 'tag'"), "a manual run releases nothing either");
    // Security, Medium 1: a person approves in GitHub (environment `npm`) before the job starts.
    assert_eq!(job["environment"].as_str(), Some("npm"));
    assert!(job["timeout-minutes"].as_u64().is_some_and(|m| m <= 30));
    let needs: Vec<&str> = match &job["needs"] {
        Value::String(s) => vec![s.as_str()],
        Value::Sequence(v) => v.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    assert_eq!(needs, ["release"], "after the GitHub Release exists");
    let perms = job["permissions"].as_mapping().expect("its own permissions");
    let perms: Vec<(String, String)> = perms.iter().map(|(k, v)| (k.as_str().unwrap_or("").to_owned(), v.as_str().unwrap_or("").to_owned())).collect();
    assert_eq!(perms, [("contents".to_owned(), "read".to_owned()), ("id-token".to_owned(), "write".to_owned())], "OIDC and read only");
    // The workflow's default stays read-only; only `release` may write (the GitHub Release).
    assert_eq!(wf["permissions"]["contents"].as_str(), Some("read"));

    let text = serde_yaml::to_string(job).expect("yaml");
    for forbidden in ["secrets.", "NPM_TOKEN", "NODE_AUTH_TOKEN", "_authToken", "registry-url"] {
        assert!(!text.contains(forbidden), "no token in publish-npm ({forbidden})");
    }
    let steps = job["steps"].as_sequence().expect("steps");
    // Security, Low 2: every action of the whole workflow (the binary npm users run is built
    // here) is pinned to a commit, and no checkout leaves git credentials behind.
    for (name, j) in wf["jobs"].as_mapping().expect("jobs") {
        for s in j["steps"].as_sequence().expect("steps") {
            if let Some(uses) = s["uses"].as_str() {
                let pin = uses.split('@').nth(1).unwrap_or("");
                assert!(pin.len() == 40 && pin.chars().all(|c| c.is_ascii_hexdigit()), "{name:?}: pinned to a commit: {uses}");
                if uses.starts_with("actions/checkout@") {
                    assert_eq!(s["with"]["persist-credentials"].as_bool(), Some(false), "{name:?}: no git credentials left behind");
                }
            }
        }
    }
    assert!(steps.iter().any(|s| s["uses"].as_str().is_some_and(|u| u.starts_with("actions/checkout@")) && s["with"]["persist-credentials"].as_bool() == Some(false)), "no git credentials left behind");
    let run = |s: &Value| s["run"].as_str().unwrap_or("").to_owned();
    let guard = steps.iter().position(|s| run(s).contains("GITHUB_REF_NAME") && run(s).contains("packaging/npm/package.json")).expect("a tag = version guard");
    let publish = steps.iter().position(|s| run(s).contains("npm publish")).expect("a publish step");
    assert!(guard < publish, "the guard runs before the publish");
    assert!(run(&steps[guard]).contains("exit 1"), "a wrong tag stops the job");
    let p = run(&steps[publish]);
    assert!(p.contains("--provenance") && p.contains("Trusted Publisher") && p.contains("autobot-lab") && p.contains("release.yml") && p.contains("environment npm"), "says what to configure when npm refuses: {p}");
    assert!(p.contains("npm view") && p.contains("already on npm"), "a version published by hand is not an error");
    assert_eq!(steps[publish]["working-directory"].as_str(), Some("packaging/npm"));
    assert!(steps.iter().any(|s| run(s).contains("npm install -g npm@11.")), "npm 11.5.1+ for Trusted Publishing");
}

/// B2 G1: the MCP Registry listing comes after npm, with GitHub OIDC and a mcp-publisher pinned by
/// its SHA-256; tag, server.json and npm versions must agree.
#[test]
fn the_mcp_registry_is_published_after_npm_with_a_pinned_publisher() {
    let wf: Value = serde_yaml::from_str(WORKFLOW).expect("release.yml is YAML");
    let job = &wf["jobs"]["publish-mcp"];
    assert!(job.is_mapping(), "a publish-mcp job");
    assert_eq!(job["if"].as_str(), Some("github.event_name == 'push' && github.ref_type == 'tag'"));
    assert_eq!(job["needs"].as_str(), Some("publish-npm"), "after npm has the version");
    let perms: Vec<(String, String)> = job["permissions"].as_mapping().expect("permissions").iter().map(|(k, v)| (k.as_str().unwrap_or("").to_owned(), v.as_str().unwrap_or("").to_owned())).collect();
    assert_eq!(perms, [("contents".to_owned(), "read".to_owned()), ("id-token".to_owned(), "write".to_owned())]);
    let text = serde_yaml::to_string(job).expect("yaml");
    assert!(!text.contains("secrets.") && !text.contains("TOKEN"), "no token");
    let run = |s: &Value| s["run"].as_str().unwrap_or("").to_owned();
    let steps = job["steps"].as_sequence().expect("steps");
    let guard = steps.iter().position(|s| run(s).contains("GITHUB_REF_NAME") && run(s).contains("server.json") && run(s).contains("packaging/npm/package.json")).expect("version guard");
    let fetch = steps.iter().position(|s| run(s).contains("mcp-publisher") && run(s).contains("sha256sum -c")).expect("pinned publisher");
    // FastRelease (v0.6): every release had to re-run this job by hand while npm caught up. It now
    // waits for npm to show the version with its mcpName (about 10 minutes), then says what to do.
    let wait = steps.iter().position(|s| run(s).contains("https://registry.npmjs.org/fastpilot/${VERSION}")).expect("waits for npm");
    let w = run(&steps[wait]);
    assert!(w.contains("seq 1 30") && w.contains("sleep 20") && w.contains(".mcpName") && w.contains("jq -r .name server.json"), "30 checks, 20 s apart, of npm's mcpName: {w}");
    assert!(w.contains("::error::npm still does not show fastpilot") && w.contains("re-run this job"), "a clear message when npm never shows it: {w}");
    assert!(job["timeout-minutes"].as_u64().is_some_and(|m| m >= 15), "room for the wait");
    let publish = steps.iter().position(|s| run(s).contains("login github-oidc") && run(s).contains("publish server.json")).expect("publish");
    assert!(run(&steps[publish]).contains("for try in 1 2 3"), "retries for npm's delay");
    assert!(guard < wait && wait < publish && guard < fetch && fetch < publish);
    assert!(run(&steps[fetch]).contains("/download/v1.") && run(&steps[fetch]).contains("  mcp-publisher.tar.gz"), "a fixed version and its hash");
    assert!(run(&steps[publish]).contains("::error::"), "a refusal is reported");
}

/// QA v0.6 re-test Low 4: a published binary always carries the Android helper. Packaging for a
/// release stops when the APK is missing (it only warned before), after `agent-android/build.sh`
/// built it, and a step checks that the packaged binary holds it.
#[test]
fn published_binaries_carry_the_android_helper() {
    let wf: Value = serde_yaml::from_str(WORKFLOW).expect("release.yml is YAML");
    let steps = wf["jobs"]["package"]["steps"].as_sequence().expect("package steps");
    let run = |s: &Value| s["run"].as_str().unwrap_or("").to_owned();
    let pos = |needle: &str| steps.iter().position(|s| run(s).contains(needle)).unwrap_or_else(|| panic!("a step runs {needle}"));
    let (build, package) = (pos("agent-android/build.sh"), pos("scripts/package.sh"));
    assert!(build < package, "the helper is built before packaging");
    assert!(run(&steps[package]).contains("--require-android"), "packaging for a release requires the helper");
    let check = pos("classes.dex");
    assert!(check > package && steps[check]["if"].is_null(), "every target's binary is checked for the helper");
    assert!(run(&steps[check]).contains("grep -aq \"classes.dex\"") && run(&steps[check]).contains("::error::the release binary has no Android helper"));
    // Without the APK (as in a plain checkout and CI), `--require-android` stops before building.
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    if std::path::Path::new(root).join("agent-android/out/fastpilot-agent.apk").exists() {
        return eprintln!("skipping the script part: the helper APK is built here");
    }
    let out = std::process::Command::new("sh").arg("scripts/package.sh").args(["--target", "x86_64-unknown-linux-gnu", "--require-android"]).current_dir(root).output().expect("package.sh");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("this release would have no Android helper. Run agent-android/build.sh first."), "{err}");
}
