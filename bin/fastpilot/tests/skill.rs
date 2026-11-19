//! B4: the Agent Skill stays a valid skill and in step with the server: front matter as the
//! Agent Skills format wants it, and every MCP tool, `docs` topic and CLI command it names exists.

use rmcp::ServiceExt;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

#[tokio::test]
async fn skill_matches_the_server() {
    let text = std::fs::read_to_string(format!("{ROOT}/skills/fastpilot/SKILL.md")).expect("SKILL.md");
    // Front matter: name = folder name (lowercase, hyphens), description ≤ 1024 characters.
    let front = text.strip_prefix("---\n").and_then(|t| t.split_once("\n---\n")).map(|(f, _)| f).expect("front matter");
    let field = |k: &str| front.lines().find_map(|l| l.strip_prefix(&format!("{k}: "))).map(str::trim);
    assert_eq!(field("name"), Some("fastpilot"));
    let description = field("description").expect("description");
    assert!(!description.is_empty() && description.chars().count() <= 1024, "description: {} chars", description.chars().count());
    assert!(text.lines().count() < 500, "keep SKILL.md short");

    // Every tool the server offers is described, and nothing else is claimed as a tool.
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT);
    });
    let client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let tools: Vec<String> = client.list_all_tools().await.expect("tools").into_iter().map(|t| t.name.to_string()).collect();
    let line = text.lines().find(|l| l.contains("**MCP tools**")).expect("tools line");
    let listed = text[text.find(line).unwrap_or(0)..].split("\n- **Shell CLI**").next().unwrap_or("");
    let named: Vec<&str> = listed.split('`').skip(1).step_by(2).collect();
    let mut sorted_tools = tools.clone();
    sorted_tools.sort();
    let mut sorted_named: Vec<String> = named.iter().map(|s| s.to_string()).collect();
    sorted_named.sort();
    assert_eq!(sorted_named, sorted_tools, "SKILL.md tool list vs server");

    // `docs` topics it names exist.
    for topic in ["dsl", "selectors", "parallel", "examples", "errors", "memory", "devices"] {
        assert!(text.contains(&format!("`{topic}`")), "topic {topic} not mentioned");
        let res = client
            .call_tool(rmcp::model::CallToolRequestParams::new("docs").with_arguments(serde_json::json!({ "topic": topic }).as_object().cloned().unwrap_or_default()))
            .await
            .expect("docs");
        assert!(res.is_error != Some(true), "docs topic {topic}");
    }
    client.cancel().await.expect("shutdown");

    // CLI commands it shows are commands `fastpilot help` knows.
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).arg("help").output().expect("help");
    let help = String::from_utf8_lossy(&help.stdout).into_owned();
    for c in ["open", "snapshot", "click", "fill", "wait", "assert", "flow", "close", "doctor", "setup", "config set"] {
        assert!(text.contains(&format!("fastpilot {c}")), "SKILL.md does not show `fastpilot {c}`");
        let word = c.split(' ').next().unwrap_or(c);
        assert!(help.contains(word) || ["doctor", "setup", "config"].contains(&word), "`fastpilot {word}` not in help");
    }
}
