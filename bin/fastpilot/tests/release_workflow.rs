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
    assert_eq!(job["if"].as_str(), Some("github.ref_type == 'tag'"), "only for a tag: a dry run publishes nothing");
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
    for s in steps {
        if let Some(uses) = s["uses"].as_str() {
            let pin = uses.split('@').nth(1).unwrap_or("");
            assert!(pin.len() == 40 && pin.chars().all(|c| c.is_ascii_hexdigit()), "pinned to a commit: {uses}");
        }
    }
    assert!(steps.iter().any(|s| s["uses"].as_str().is_some_and(|u| u.starts_with("actions/checkout@")) && s["with"]["persist-credentials"].as_bool() == Some(false)), "no git credentials left behind");
    let run = |s: &Value| s["run"].as_str().unwrap_or("").to_owned();
    let guard = steps.iter().position(|s| run(s).contains("GITHUB_REF_NAME") && run(s).contains("packaging/npm/package.json")).expect("a tag = version guard");
    let publish = steps.iter().position(|s| run(s).contains("npm publish")).expect("a publish step");
    assert!(guard < publish, "the guard runs before the publish");
    assert!(run(&steps[guard]).contains("exit 1"), "a wrong tag stops the job");
    let p = run(&steps[publish]);
    assert!(p.contains("--provenance") && p.contains("Trusted Publisher") && p.contains("autobot-lab") && p.contains("release.yml"), "says what to configure when npm refuses: {p}");
    assert!(p.contains("npm view") && p.contains("already on npm"), "a version published by hand is not an error");
    assert_eq!(steps[publish]["working-directory"].as_str(), Some("packaging/npm"));
    assert!(steps.iter().any(|s| run(s).contains("npm install -g npm@11.")), "npm 11.5.1+ for Trusted Publishing");
}
