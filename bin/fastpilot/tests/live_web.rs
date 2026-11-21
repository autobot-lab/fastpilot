//! C7a: the Live panel of `fastpilot ui` shows what a running `fastpilot mcp` is doing (real
//! Chrome): its web session, the steps of the flow with the running one and the one a guard rail
//! will block, and the screen as JPEG frames, read-only and only with the page's cookie. The
//! DevTools address of fastpilot's Chrome never reaches the browser. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| Path::new(p).exists())
}

async fn call(client: &rmcp::service::Peer<RoleClient>, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

/// GET with the page cookie (or none); `(status, body)`; reads at most `limit` body bytes.
fn get(port: u16, path: &str, cookie: Option<&str>, limit: usize) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(15))).ok();
    let c = cookie.map(|c| format!("Cookie: {c}\r\n")).unwrap_or_default();
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{c}Connection: close\r\n\r\n").expect("write");
    let mut raw = Vec::new();
    let mut buf = [0u8; 16384];
    while raw.len() < limit {
        match s.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
        }
    }
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("head");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).expect("status");
    (status, raw[split + 4..].to_vec())
}

/// Trade the one-time code for the session cookie (`name=value`).
fn login(port: u16, code: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(s, "GET /?code={code} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").expect("write");
    let mut head = String::new();
    s.read_to_string(&mut head).expect("read");
    head.lines()
        .find_map(|l| l.to_ascii_lowercase().starts_with("set-cookie:").then(|| l["set-cookie:".len()..].trim().split(';').next().unwrap_or("").to_owned()))
        .expect("session cookie")
}

struct Ui(std::process::Child);
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn live_panel_shows_a_running_web_session() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-live-web-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    std::fs::write(home.join(".fastpilot/config.toml"), "block_actions = [\"Keluar\"]\n").expect("config");

    // The page.
    let mut ui = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    ui.args(["ui", "--no-open"]).env("HOME", &home).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_LIVE_DIR").stdout(std::process::Stdio::piped());
    let mut child = ui.spawn().expect("ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("url");
    let _ui = Ui(child);
    let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("url").to_owned();
    let (port, code) = rest.split_once("/?code=").expect("code");
    let port: u16 = port.parse().expect("port");
    let cookie = login(port, code);

    let (status, body) = get(port, "/api/live", Some(&cookie), 1 << 20);
    assert_eq!(status, 200);
    assert_eq!(serde_json::from_slice::<Value>(&body).expect("json")["sessions"], json!([]), "nothing runs yet");

    // A test run by an agent, through the MCP server of the same user.
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("HOME", &home).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_LIVE_DIR").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let page = format!("file://{ROOT}/examples/demo/login.html");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- session: {{ start: shop, platform: web, target: \"{page}\" }}\n") })).await;
    assert!(ok, "{out}");
    let flow = "- inputText: { into: Email, text: a@b.com }\n- assertVisible: { text: \"Pesanan siap\", timeout: 20000 }\n- tapOn: Keluar\n";
    let runner = {
        let peer = client.peer().clone();
        tokio::spawn(async move { call(&peer, "run_flow", json!({ "flow": flow, "session": "shop" })).await })
    };

    // The panel sees the session, the running step and the one block_actions will refuse.
    let deadline = Instant::now() + Duration::from_secs(15);
    let session = loop {
        let (_, body) = get(port, "/api/live", Some(&cookie), 1 << 20);
        let v: Value = serde_json::from_slice(&body).expect("json");
        let text = String::from_utf8_lossy(&body).into_owned();
        assert!(!text.contains("ws://") && !text.contains("devtools"), "the DevTools address must stay on the server: {text}");
        let s = v["sessions"].as_array().and_then(|a| a.iter().find(|s| s["id"] == "shop")).cloned();
        if let Some(s) = s.filter(|s| s["flow"]["steps"][1]["state"] == "running") {
            break s;
        }
        assert!(Instant::now() < deadline, "session or running step never showed: {text}");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(session["platform"], "web");
    assert_eq!(session["watchable"], true);
    assert_eq!(session["kind"], "mcp");
    assert_eq!(session["flow"]["steps"][0]["state"], "done");
    assert!(session["flow"]["steps"][2]["will_block"].as_str().is_some_and(|w| w.contains("(your blocked words)") && !w.contains("block_actions")), "{session}");
    // V042: a terminal session is not "My agent".
    assert_eq!(session["agent_name"], "Started from the terminal", "{session}");
    let pid = session["pid"].as_u64().expect("pid");

    // The screen: JPEG frames, only with the cookie, only for a session that exists.
    let frames = format!("/api/live/frames?pid={pid}&session=shop&fps=4&w=640");
    assert_eq!(get(port, &frames, None, 1 << 16).0, 401);
    assert_eq!(get(port, &format!("/api/live/frames?pid={pid}&session=nope"), Some(&cookie), 1 << 16).0, 404);
    let (status, body) = get(port, &frames, Some(&cookie), 64 * 1024);
    assert_eq!(status, 200);
    // Chunked on the wire: look for the multipart parts and a JPEG start inside.
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("--fpframe\r\n") && text.contains("Content-Type: image/jpeg\r\nContent-Length: "), "{}", &text[..text.len().min(160)]);
    assert!(body.windows(3).any(|w| w == [0xFF, 0xD8, 0xFF]), "a JPEG frame");

    // The run ends (blocked), the session stops: it leaves the panel.
    let (ok, out) = runner.await.expect("runner");
    assert!(!ok, "{out}");
    let (_, body) = get(port, "/api/live", Some(&cookie), 1 << 20);
    let v: Value = serde_json::from_slice(&body).expect("json");
    let steps = &v["sessions"][0]["flow"]["steps"];
    assert_eq!((steps[1]["state"].as_str(), steps[2]["state"].as_str()), (Some("failed"), Some("skipped")), "{v}");
    let (ok, out) = call(&client, "session", json!({ "action": "stop", "id": "shop" })).await;
    assert!(ok, "{out}");
    let (_, body) = get(port, "/api/live", Some(&cookie), 1 << 20);
    assert_eq!(serde_json::from_slice::<Value>(&body).expect("json")["sessions"], json!([]));
    client.cancel().await.expect("shutdown");
    // The server's file goes with it.
    std::thread::sleep(Duration::from_millis(500));
    let left = std::fs::read_dir(home.join(".fastpilot/live")).map(|d| d.count()).unwrap_or(0);
    assert_eq!(left, 0, "live file left behind");
    let _ = std::fs::remove_dir_all(home);
}
