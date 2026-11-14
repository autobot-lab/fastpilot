//! Secret masking on real Chrome over MCP (issues #2 and #3): a `FASTPILOT_VARS` value typed
//! into a plain text field must show as `***` in tool output and in every evidence file,
//! even when it is longer than the snapshot's value clip or contains `"` / `\`.
//! Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
// Fake values.
const LONG: &str = "sk-test-FAKE0123456789abcdefghijklmnopqrstuvwxyzABCDEFGH";
const QUOTED: &str = r#"pa"ss\word99"#;

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

/// No piece of either secret, raw or escaped, long enough to identify it.
fn leaks(text: &str) -> Vec<&'static str> {
    ["FAKE0123", "sk-test-F", "word99", r#"pa\"ss"#, r#"ss\\word"#, "pa&quot;ss"].into_iter().filter(|p| text.contains(p)).collect()
}

fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() { files(&p, out) } else { out.push(p) }
    }
}

#[tokio::test]
async fn long_and_escaped_secrets_never_leak() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-secrets-web-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp")
            .current_dir(ROOT)
            .env("FASTPILOT_DIR", &dir)
            .env("FASTPILOT_VARS", "LONGTOK,QTOK")
            .env("LONGTOK", LONG)
            .env("QTOK", QUOTED);
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    let (ok, out) = call(&client, "session", json!({ "action": "start", "id": "w", "platform": "web", "target": "examples/demo/secret.html", "test_id": "secret" })).await;
    assert!(ok, "{out}");
    let flow = r#"
- inputText: { into: "API Token", text: "${LONGTOK}" }
- inputText: { into: "Kata sandi lama", text: "${QTOK}" }
- assertVisible: { text: "Tidak ada", timeout: 500 }
"#;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow, "test_id": "secret" })).await;
    println!("{out}");
    assert!(!ok, "the flow fails on purpose: {out}");
    assert!(out.contains("textfield \"API Token\" value=\"***\""), "{out}");
    assert!(out.contains("textfield \"Kata sandi lama\" value=\"***\""), "{out}");
    assert!(leaks(&out).is_empty(), "leaked {:?} in\n{out}", leaks(&out));
    for args in [json!({ "session": "w" }), json!({ "session": "w", "full": true, "coords": true }), json!({ "session": "w", "filter": "token" })] {
        let (ok, snap) = call(&client, "snapshot", args).await;
        assert!(ok && leaks(&snap).is_empty(), "leaked {:?} in\n{snap}", leaks(&snap));
    }
    let (ok, out) = call(&client, "report", json!({ "test_id": "secret", "status": "fail", "notes": "secret check" })).await;
    assert!(ok, "{out}");
    client.cancel().await.expect("shutdown");
    let mut all = Vec::new();
    files(&dir, &mut all);
    assert!(all.iter().any(|p| p.to_string_lossy().contains("snapshot")), "no failure snapshot written: {all:?}");
    for p in all {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        assert!(leaks(&text).is_empty(), "leaked {:?} in {}", leaks(&text), p.display());
    }
    let _ = std::fs::remove_dir_all(dir);
}
