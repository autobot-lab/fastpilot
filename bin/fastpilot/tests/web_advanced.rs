//! Web edge cases over MCP: cross-site iframe (OOPIF), popup, download, invisible nodes,
//! console diagnostics, Set-of-Mark screenshot, snapshot find, and tools on other sessions not
//! waiting for a running flow. Serves pages from two local origins (localhost / 127.0.0.1 are
//! different sites, so Chrome isolates the iframe). Skips without Chrome.

use std::time::{Duration, Instant};

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Client = RunningService<RoleClient, ()>;

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String, CallToolResult) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text, res)
}

fn page(path: &str, port: u16) -> String {
    match path {
        "/" => format!(
            r#"<!doctype html><title>Utama</title>
<h1>Utama</h1>
<iframe src="http://127.0.0.1:{port}/frame" style="height:80px"></iframe>
<button onclick="window.open('/popup', 'p', 'width=400,height=300')">Buka popup</button>
<p id="status" role="status"></p>
<a href="data:text/plain,halo" download="laporan.txt">Unduh laporan</a>
<p style="opacity:0">Teks tersembunyi</p>
<button onclick="console.error('stok habis'); fetch('/missing')">Pesan</button>
<script>window.addEventListener('message', (e) => document.getElementById('status').textContent = e.data);</script>"#
        ),
        "/frame" => r#"<!doctype html><button onclick="this.textContent='Frame diklik'">Tombol lintas situs</button>"#.to_owned(),
        "/popup" => r#"<!doctype html><title>Popup</title><p>Isi popup</p>
<button onclick="window.opener.postMessage('Dikonfirmasi', '*'); window.close()">Konfirmasi</button>"#.to_owned(),
        _ => String::new(),
    }
}

/// Minimal HTTP server: GET path -> HTML (404 when empty).
async fn serve() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body = page(&path, port);
                let status = if body.is_empty() { "404 Not Found" } else { "200 OK" };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    port
}

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

#[tokio::test]
async fn web_edge_cases_over_mcp() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let port = serve().await;
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let dir = std::env::temp_dir().join(format!("fastpilot-e2e-adv-{}", std::process::id()));
    let d = dir.clone();
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(root).env("FASTPILOT_DIR", &d);
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");

    let flow = format!(
        r#"
- session: {{ start: w, target: "http://localhost:{port}/" }}
- tapOn: "Tombol lintas situs"
- assertVisible: "Frame diklik"
- assertNotVisible: {{ text: "Teks tersembunyi", timeout: 500 }}
- tapOn: "Buka popup"
- assertVisible: "Isi popup"
- tapOn: "Konfirmasi"
- assertVisible: "Dikonfirmasi"
- tapOn: "Unduh laporan"
- assertVisible: "download completed: laporan.txt"
"#
    );
    let (ok, out, _) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    assert!(ok, "{out}");
    // Failure carries the page's recent errors.
    let (ok, out, _) = call(&client, "run_flow", json!({ "flow": "- tapOn: Pesan\n- assertVisible: { text: Terkirim, timeout: 800 }\n" })).await;
    println!("{out}");
    assert!(!ok && out.contains("recent errors") && out.contains("console.error: stok habis") && out.contains("HTTP 404"), "{out}");

    // Find with context, coords, file.
    let (ok, out, _) = call(&client, "snapshot", json!({ "filter": "diklik", "coords": true })).await;
    assert!(ok && out.contains("button \"Frame diklik\"") && out.contains(" @") && !out.contains("Unduh"), "{out}");
    let (ok, out, _) = call(&client, "snapshot", json!({ "file": true })).await;
    assert!(ok && out.contains("lines written to"), "{out}");

    // Set-of-Mark: image + marks list.
    let (ok, out, res) = call(&client, "screenshot", json!({ "annotate": "som" })).await;
    assert!(ok && out.contains("marks") && out.contains(" @"), "{out}");
    assert!(res.content.first().and_then(|c| c.as_image()).is_some());

    // A long flow on `w` must not block tools on another session.
    let (ok, _, _) = call(&client, "session", json!({ "action": "start", "id": "other", "target": "examples/demo/login.html" })).await;
    assert!(ok);
    let slow = call(&client, "run_flow", json!({ "flow": "- assertVisible: { text: Tidak ada, timeout: 3000 }\n", "session": "w" }));
    let quick = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let t = Instant::now();
        let r = call(&client, "snapshot", json!({ "session": "other" })).await;
        (r, t.elapsed())
    };
    let ((slow_ok, _, _), ((snap_ok, _, _), took)) = tokio::join!(slow, quick);
    assert!(!slow_ok && snap_ok);
    assert!(took < Duration::from_millis(1500), "snapshot waited {took:?} for another session's flow");

    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(dir);
}
