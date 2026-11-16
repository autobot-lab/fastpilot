//! Saved login state on real Chrome over MCP: log in once in one session, save the state
//! (encrypted), and a fresh isolated session that loads it is logged in without the login
//! page. A local HTTP server sets an HttpOnly session cookie and the page keeps a preference
//! in localStorage. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const TOKEN: &str = "tok-SESSION-8f2c1a";
const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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

/// `/app` greets a logged-in user (cookie) and shows the theme kept in localStorage;
/// `/login` sets the HttpOnly cookie and the theme, then goes to `/app`.
async fn serve() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let logged_in = req.lines().any(|l| l.to_ascii_lowercase().starts_with("cookie:") && l.contains(&format!("sid={TOKEN}")));
                let (extra, body) = match path.as_str() {
                    "/login" => (
                        format!("Set-Cookie: sid={TOKEN}; Path=/; HttpOnly; Max-Age=3600\r\n"),
                        "<!doctype html><title>Login</title><script>localStorage.setItem('theme', 'dark'); location.href = '/app';</script>".to_owned(),
                    ),
                    _ => (
                        String::new(),
                        format!(
                            "<!doctype html><title>App</title><h1>{}</h1><p id=t></p>\
                             <script>document.getElementById('t').textContent = 'Tema: ' + (localStorage.getItem('theme') || 'terang');</script>",
                            if logged_in { "Halo Budi" } else { "Silakan login" }
                        ),
                    ),
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    port
}

#[tokio::test]
async fn saved_login_state_skips_the_login() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let port = serve().await;
    let dir = std::env::temp_dir().join(format!("fastpilot-state-web-{}", std::process::id()));
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_DIR", &dir).env("FASTPILOT_STATE_KEY", KEY);
    });
    let Ok(client) = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await else {
        return eprintln!("skipping: server did not start");
    };
    let client: Client = client;
    let app = format!("http://localhost:{port}/app");

    // Session A logs in and saves.
    let flow = format!(
        "- session: {{ start: a, platform: web, target: \"{app}\" }}\n- assertVisible: \"Silakan login\"\n\
         - openUrl: \"http://localhost:{port}/login\"\n- assertVisible: \"Halo Budi\"\n- assertVisible: \"Tema: dark\"\n- saveState: admin\n"
    );
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok && out.contains("admin.json (encrypted)"), "{out}");
    let raw = std::fs::read(dir.join("states/admin.json")).expect("state file");
    assert!(raw.starts_with(b"FPSTATE1") && !String::from_utf8_lossy(&raw).contains(TOKEN), "token in plain text");

    // Session B: its own BrowserContext, so not logged in, until the state is loaded.
    let flow = format!(
        "- session: {{ start: b, platform: web, target: \"{app}\" }}\n- assertVisible: \"Silakan login\"\n- loadState: admin\n\
         - assertVisible: \"Halo Budi\"\n- assertVisible: \"Tema: dark\"\n"
    );
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok && out.contains("loadState admin") && !out.contains(TOKEN), "{out}");

    // The tool: list, then load into a third session before it navigates.
    let (ok, out) = call(&client, "state", json!({ "action": "list" })).await;
    assert!(ok && out.contains("admin") && out.contains("encrypted"), "{out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- session: { start: c, platform: web }\n" })).await;
    assert!(ok, "{out}");
    let (ok, out) = call(&client, "state", json!({ "action": "load", "name": "admin", "session": "c" })).await;
    assert!(ok, "{out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- openUrl: \"{app}\"\n- assertVisible: \"Halo Budi\"\n- assertVisible: \"Tema: dark\"\n"), "session": "c" })).await;
    assert!(ok, "{out}");

    // Missing state, and delete.
    let (ok, out) = call(&client, "state", json!({ "action": "load", "name": "nope", "session": "c" })).await;
    assert!(!ok && out.contains("no saved state `nope`"), "{out}");
    let (ok, out) = call(&client, "state", json!({ "action": "delete", "name": "admin" })).await;
    assert!(ok && !dir.join("states/admin.json").exists(), "{out}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(dir);
}
