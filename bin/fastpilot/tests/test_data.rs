//! Test-data setup over MCP: `httpRequest` against a local API (seed, capture, teardown) and
//! `readOtp` from Mailpit- and MailHog-shaped inboxes and a plain HTTP endpoint. The mail
//! servers are local fakes returning the response shapes documented by Mailpit (API v1) and
//! MailHog (API v2). Skips without Chrome.

use std::sync::{Arc, Mutex};

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Client = RunningService<RoleClient, ()>;
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

/// Fake API: POST /users (needs `Authorization: Bearer admin`), DELETE /users/7, a Mailpit and
/// a MailHog inbox, /sms (plain text). Records the requests it saw.
async fn serve(seen: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16384];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let line = req.lines().next().unwrap_or("").to_owned();
                let body_in = req.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
                seen.lock().unwrap().push(format!("{line} {body_in}"));
                let auth = req.to_ascii_lowercase().contains("authorization: bearer admin");
                let (status, body) = match line.split_whitespace().take(2).collect::<Vec<_>>()[..] {
                    ["POST", "/users"] if auth => ("201 Created", json!({ "id": 7, "session": { "token": "tok-SECRET-123" } }).to_string()),
                    ["POST", "/users"] => ("401 Unauthorized", "{}".to_owned()),
                    ["DELETE", "/users/7"] => ("204 No Content", String::new()),
                    ["GET", "/api/v1/messages?limit=20"] => ("200 OK", json!({ "messages": [
                        { "ID": "m2", "To": [{ "Address": "budi@x.id" }], "Subject": "Kode masuk" },
                        { "ID": "m1", "To": [{ "Address": "other@x.id" }], "Subject": "Kode masuk" }
                    ]}).to_string()),
                    ["GET", "/api/v1/message/m2"] => ("200 OK", json!({ "Text": "Halo Budi,\r\nkode Anda 482913, berlaku 5 menit." }).to_string()),
                    ["GET", "/api/v2/messages?limit=20"] => ("200 OK", json!({ "items": [
                        { "Content": { "Headers": { "To": ["budi@x.id"], "Subject": ["OTP"] }, "Body": "Kode: 55=\r\n1177" } }
                    ]}).to_string()),
                    ["GET", "/sms"] => ("200 OK", "Your code is 9081".to_owned()),
                    ["GET", "/device"] => ("200 OK", "<!doctype html><title>Perangkat</title><h1>Kontrol perangkat</h1>".to_owned()),
                    ["GET", p] if p.starts_with("/ping") => ("200 OK", "pong".to_owned()),
                    _ => ("404 Not Found", String::new()),
                };
                let resp = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    port
}

#[tokio::test]
async fn http_requests_and_one_time_codes_over_mcp() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let port = serve(seen.clone()).await;
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT).env("ADMIN_TOKEN", "admin").env("FASTPILOT_VARS", "ADMIN_TOKEN");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let api = format!("http://127.0.0.1:{port}");
    // Seeding needs no session; captured values feed later steps; tokens never echo back.
    let flow = format!(
        r#"
- httpRequest:
    method: POST
    url: "{api}/users"
    headers: {{ Authorization: "Bearer ${{ADMIN_TOKEN}}" }}
    json: {{ email: "budi@x.id", roles: ["buyer"] }}
    expect: 201
    capture: {{ USER_ID: id, USER_TOKEN: session.token }}
- readOtp: {{ from: mailpit, url: "{api}", to: "budi@x.id", after: "kode" }}
- readOtp: {{ from: mailhog, url: "{api}", to: "budi@x.id", into: MH_OTP }}
- readOtp: {{ from: http, url: "{api}/sms", length: 4, into: SMS_CODE, timeout: 2000 }}
- session: {{ start: w, target: examples/demo/maestro.html }}
- inputText: {{ into: "Kode", text: "${{OTP}}-${{MH_OTP}}-${{SMS_CODE}}" }}
- inputText: {{ into: "Salinan", text: "${{USER_TOKEN}}" }}
- evalScript: "document.getElementById('kode').value + ' ' + document.getElementById('salin').value"
- httpRequest: {{ method: DELETE, url: "{api}/users/${{USER_ID}}", expect: 204 }}
"#
    );
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    if out.contains("Chrome not found") {
        return eprintln!("skipping: no Chrome");
    }
    assert!(ok, "{out}");
    assert!(out.contains("= 201; captured USER_ID, USER_TOKEN"), "{out}");
    assert!(out.contains("= code from mailpit in ${OTP}"), "{out}");
    // The codes reach the page but every one-time code and token is masked in what comes back.
    assert!(out.contains("= \"***-***-*** ***\""), "{out}");
    assert!(!out.contains("482913") && !out.contains("551177") && !out.contains("tok-SECRET-123"), "{out}");
    assert!(out.contains("= 204"), "{out}");
    let log = seen.lock().unwrap().join("\n");
    assert!(log.contains("\"email\":\"budi@x.id\"") && log.contains("DELETE /users/7"), "{log}");

    // A wrong status fails as an assertion with the response; unknown capture paths too.
    let flow = format!("- httpRequest: {{ method: POST, url: \"{api}/users\", json: {{ a: 1 }} }}\n");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(!ok && out.contains("assertion_failed") && out.contains("returned 401, expected 2xx"), "{out}");
    let (ok, out) = call(&client, "check_flow", json!({ "flow": "- readOtp: { from: pigeon }\n" })).await;
    assert!(!ok && out.contains("unknown source"), "{out}");
    client.cancel().await.expect("shutdown");
}

/// Device control on Chrome: geolocation, permissions, locale/timezone, offline and latency.
#[tokio::test]
async fn device_control_on_web() {
    let port = serve(Arc::new(Mutex::new(Vec::new()))).await;
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(|c| {
        c.arg("mcp").current_dir(ROOT);
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let flow = format!(
        r#"
- session: {{ start: d, target: "http://localhost:{port}/device" }}
- setPermissions: {{ permissions: {{ location: allow, camera: deny }} }}
- setLocation: {{ latitude: -6.2088, longitude: 106.8456 }}
- evalScript: "new Promise(r => navigator.geolocation.getCurrentPosition(p => r(p.coords.latitude.toFixed(4) + ',' + p.coords.longitude.toFixed(4)), e => r('error ' + e.code)))"
- evalScript: "navigator.permissions.query({{ name: 'camera' }}).then(s => s.state)"
- setLocale: {{ locale: id-ID, timezone: Asia/Jakarta }}
- evalScript: "Intl.DateTimeFormat().resolvedOptions().timeZone + ' ' + navigator.language + ' ' + (1234.5).toLocaleString()"
- setNetwork: offline
- evalScript: "fetch('/ping').then(() => 'up', () => 'down')"
- setNetwork: {{ latency: 600 }}
- evalScript: "(async () => {{ const t = performance.now(); await fetch('/ping?' + Math.random()); return Math.round(performance.now() - t) >= 500; }})()"
- setNetwork: online
"#
    );
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    println!("{out}");
    if out.contains("Chrome not found") {
        return eprintln!("skipping: no Chrome");
    }
    assert!(ok, "{out}");
    for want in ["= \"-6.2088,106.8456\"", "= \"denied\"", "= \"Asia/Jakarta id-ID 1.234,5\"", "= \"down\"", "= true"] {
        assert!(out.contains(want), "missing {want} in\n{out}");
    }
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- pushNotification: { title: Halo }\n" })).await;
    assert!(!ok && out.contains("unsupported"), "{out}");
    client.cancel().await.expect("shutdown");
}
