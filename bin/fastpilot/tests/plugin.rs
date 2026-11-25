//! B1 I2 (v0.5): the repo is its own Claude Code marketplace with one plugin, `fastpilot`: the MCP
//! server through `npx -y fastpilot mcp` and the 24 skills of `skills/` (namespaced
//! `fastpilot:<skill>`). Checks the manifests' shape, that every skill folder has a SKILL.md with
//! a name and a description, and, when an installed `claude` knows `plugin validate --strict`,
//! runs it (else the shape checks stand in for it, e.g. in CI).

use serde_json::Value;
use std::path::Path;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn json(rel: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(Path::new(ROOT).join(rel)).expect(rel)).expect(rel)
}

#[test]
fn the_repo_is_a_claude_code_marketplace_with_the_fastpilot_plugin() {
    let plugin = json(".claude-plugin/plugin.json");
    assert_eq!(plugin["name"], "fastpilot");
    assert_eq!(plugin["license"], "MIT");
    assert_eq!(plugin["repository"], "https://github.com/autobot-lab/fastpilot");
    assert_eq!(plugin["mcpServers"]["fastpilot"], serde_json::json!({ "command": "npx", "args": ["-y", "fastpilot", "mcp"] }), "the MCP server from npm");
    let market = json(".claude-plugin/marketplace.json");
    assert_eq!(market["name"], "fastpilot");
    assert_eq!(market["owner"]["name"], "autobot-lab");
    let plugins = market["plugins"].as_array().expect("plugins");
    assert_eq!(plugins.len(), 1);
    assert_eq!((plugins[0]["name"].as_str(), plugins[0]["source"].as_str()), (Some("fastpilot"), Some("./")));

    // Skills load from skills/<name>/SKILL.md; each has a name equal to its folder and a description.
    let mut names = Vec::new();
    for e in std::fs::read_dir(Path::new(ROOT).join("skills")).expect("skills").flatten().filter(|e| e.path().is_dir()) {
        let text = std::fs::read_to_string(e.path().join("SKILL.md")).unwrap_or_else(|_| panic!("{:?} has a SKILL.md", e.file_name()));
        let front = text.strip_prefix("---\n").and_then(|t| t.split_once("\n---")).map(|(f, _)| f).expect("frontmatter");
        let get = |k: &str| front.lines().find_map(|l| l.strip_prefix(&format!("{k}: "))).unwrap_or("").trim().to_owned();
        assert_eq!(get("name"), e.file_name().to_string_lossy(), "skill name = folder");
        assert!(get("description").len() > 20, "{:?} has a description", e.file_name());
        names.push(get("name"));
    }
    assert_eq!(names.len(), 24, "{names:?}");

    // The real validator, when this computer has a Claude Code that knows it.
    let claude = std::env::var("FASTPILOT_CLAUDE").unwrap_or_else(|_| "claude".into());
    let strict = std::process::Command::new(&claude).args(["plugin", "validate", "--help"]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("--strict"));
    if strict {
        let out = std::process::Command::new(&claude).args(["plugin", "validate", ".", "--strict"]).current_dir(ROOT).output().expect("claude");
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && text.contains("Validation passed"), "{text}");
    } else {
        eprintln!("claude plugin validate --strict is not available here: the shape checks above stand in for it");
    }
}
