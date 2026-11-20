//! Pengaturan › Model AI (Design must-fix): changing the provider puts that provider's server in
//! the Base URL field (hosted ones read-only), so the previous server is not left behind. Driven by
//! fastpilot in Chrome; skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::Path;

type Client = RunningService<RoleClient, ()>;
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| Path::new(p).exists())
}

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

async fn eval(client: &Client, js: &str) -> String {
    let flow = format!("- evalScript: {}\n", serde_json::to_string(js).expect("js"));
    let (ok, out) = call(client, "run_flow", json!({ "flow": flow, "session": "viewer" })).await;
    assert!(ok, "{out}");
    out.lines().find_map(|l| l.rsplit_once(" = ").map(|(_, v)| v.trim().to_owned())).unwrap_or_default()
}

struct Ui(std::process::Child);
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_change_puts_its_server_in_the_field() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-prov-page-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    std::fs::write(home.join(".fastpilot/config.toml"), "provider = \"custom\"\nprovider_base_url = \"http://127.0.0.1:18767/v1\"\nprovider_model = \"m\"\n").expect("config");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .env("HOME", &home)
        .env("FASTPILOT_KEYSTORE", "file")
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_PROVIDER")
        .env_remove("FASTPILOT_PROVIDER_BASE_URL")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let _ui = Ui(child);
    let viewer_home = home.join("viewer");
    std::fs::create_dir_all(&viewer_home).expect("viewer");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").current_dir(ROOT).env("HOME", &viewer_home).env("FASTPILOT_LIVE_DIR", viewer_home.join("live")).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let flow = format!("- session: {{ start: viewer, platform: web, target: \"{}\" }}\n", link.trim());
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    eval(&client, "location.hash = '#/settings/model', 'ok'").await;
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: \"Base URL\", timeout: 5000 }\n", "session": "viewer" })).await;
    assert!(ok, "{out}");
    let pick = |p: &str| format!("(() => {{ const s = document.querySelector('select[aria-label^=\"Model provider\"]'); s.value = '{p}'; s.dispatchEvent(new Event('change')); const u = document.querySelector('input[aria-label=\"Base URL\"]'); return u.value + '|' + u.readOnly; }})()");
    assert_eq!(eval(&client, "document.querySelector('input[aria-label=\"Base URL\"]').value").await, "\"http://127.0.0.1:18767/v1\"");
    assert_eq!(eval(&client, &pick("openrouter")).await, "\"https://openrouter.ai/api/v1|true\"", "the old server is replaced and locked");
    assert_eq!(eval(&client, &pick("groq")).await, "\"https://api.groq.com/openai/v1|true\"");
    assert_eq!(eval(&client, &pick("ollama")).await, "\"http://127.0.0.1:11434/v1|false\"");
    assert_eq!(eval(&client, &pick("custom")).await, "\"|false\"", "custom starts empty: no server from another provider");
    // An unsaved choice survives the page being redrawn (the device check finishing, a refused save).
    eval(&client, &pick("openrouter")).await;
    eval(&client, "(document.querySelector('input[aria-label=\"Model id\"]').value = 'meta/llama', document.querySelector('input[aria-label=\"Model id\"]').dispatchEvent(new Event('input')), render(), 'ok')").await;
    let kept = eval(&client, "document.querySelector('select[aria-label^=\"Model provider\"]').value + '|' + document.querySelector('input[aria-label=\"Base URL\"]').value + '|' + document.querySelector('input[aria-label=\"Model id\"]').value").await;
    assert_eq!(kept, "\"openrouter|https://openrouter.ai/api/v1|meta/llama\"");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(home);
}
