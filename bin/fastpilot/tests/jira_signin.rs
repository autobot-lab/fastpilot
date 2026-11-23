//! E5 (v0.7): Jira through "Sign in with Atlassian", against a fake Atlassian (OAuth 2.1 + MCP) on
//! 127.0.0.1. Connect → Atlassian's sign-in → back with `code` and `state` → connected, with no
//! token typed. The fake checks what Atlassian checks: PKCE S256 against the challenge, the exact
//! redirect address, a code used once. Then Jira is read through the MCP tools (the access token
//! expires after one second, so it is refreshed first), and Disconnect revokes at Atlassian.
//! Refused: a wrong `state`, and the same return twice. No token is in a page, a tool result or a
//! file as text.

use rmcp::RoleClient;
use rmcp::ServiceExt;
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type Client = RunningService<RoleClient, ()>;

/// What the fake Atlassian saw.
#[derive(Default, Debug)]
struct Seen {
    redirect: String,
    challenge: String,
    code_used: bool,
    exchanges: usize,
    refreshes: usize,
    revoked: Vec<String>,
    tools: Vec<String>,
    /// The access token that works now.
    access: String,
}

type Shared = Arc<Mutex<Seen>>;

fn form(body: &str) -> HashMap<String, String> {
    reqwest::Url::parse(&format!("http://x/?{body}")).map(|u| u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()).unwrap_or_default()
}

fn s256(v: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ring::digest::digest(&ring::digest::SHA256, v.as_bytes()).as_ref())
}

fn reply(s: &mut TcpStream, status: &str, headers: &[(&str, String)], body: &str) {
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(body);
    let _ = s.write_all(out.as_bytes());
}

fn tool_result(v: Value) -> String {
    v.to_string()
}

/// The fake Atlassian: metadata, registration, authorize (redirects straight back, as after the
/// person's Accept), token (code, refresh, revoke) and the MCP endpoint.
fn atlassian() -> (String, Shared) {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://127.0.0.1:{}", l.local_addr().expect("addr").port());
    let seen: Shared = Arc::default();
    let (b, st) = (base.clone(), seen.clone());
    std::thread::spawn(move || {
        for stream in l.incoming().flatten() {
            let mut r = BufReader::new(stream);
            let mut line = String::new();
            if r.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (method, target) = (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("/").to_owned());
            let (mut len, mut auth) = (0usize, String::new());
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).map(|n| n == 0).unwrap_or(true) || h == "\r\n" {
                    break;
                }
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if lower.starts_with("authorization:") {
                    auth = h.split_once(':').map(|x| x.1.trim().to_owned()).unwrap_or_default();
                }
            }
            let mut body = vec![0u8; len];
            let _ = r.read_exact(&mut body);
            let body = String::from_utf8_lossy(&body).into_owned();
            let mut s = r.into_inner();
            let (path, query) = target.split_once('?').unwrap_or((&target, ""));
            let mut g = st.lock().expect("seen");
            match (method.as_str(), path) {
                ("GET", "/.well-known/oauth-authorization-server") => reply(&mut s, "200 OK", &[("Content-Type", "application/json".into())], &json!({
                    "issuer": b, "authorization_endpoint": format!("{b}/v1/authorize"), "token_endpoint": format!("{b}/v1/token"),
                    "registration_endpoint": format!("{b}/v1/register"), "revocation_endpoint": format!("{b}/v1/token"),
                    "code_challenge_methods_supported": ["plain", "S256"], "token_endpoint_auth_methods_supported": ["none"],
                }).to_string()),
                ("POST", "/v1/register") => {
                    let v: Value = serde_json::from_str(&body).unwrap_or_default();
                    g.redirect = v["redirect_uris"][0].as_str().unwrap_or("").to_owned();
                    reply(&mut s, "201 Created", &[("Content-Type", "application/json".into())], &json!({ "client_id": "fake-client", "redirect_uris": [g.redirect] }).to_string());
                }
                ("GET", "/v1/authorize") => {
                    let q = form(query);
                    let ok = q.get("client_id").map(String::as_str) == Some("fake-client") && q.get("redirect_uri") == Some(&g.redirect) && q.get("code_challenge_method").map(String::as_str) == Some("S256");
                    if !ok {
                        reply(&mut s, "400 Bad Request", &[], "bad authorize");
                        continue;
                    }
                    g.challenge = q.get("code_challenge").cloned().unwrap_or_default();
                    let to = format!("{}?code=CODE-1&state={}", g.redirect, q.get("state").cloned().unwrap_or_default());
                    reply(&mut s, "302 Found", &[("Location", to)], "");
                }
                ("POST", "/v1/token") => {
                    let f = form(&body);
                    match f.get("grant_type").map(String::as_str) {
                        Some("authorization_code") => {
                            g.exchanges += 1;
                            let pkce = f.get("code_verifier").is_some_and(|v| s256(v) == g.challenge);
                            let ok = !g.code_used && f.get("code").map(String::as_str) == Some("CODE-1") && f.get("redirect_uri") == Some(&g.redirect) && f.get("client_id").map(String::as_str) == Some("fake-client") && pkce;
                            if !ok {
                                reply(&mut s, "400 Bad Request", &[("Content-Type", "application/json".into())], r#"{"error":"invalid_grant"}"#);
                                continue;
                            }
                            g.code_used = true;
                            g.access = "AT-ONE-x1".into();
                            // Expires at once: the first use refreshes it.
                            reply(&mut s, "200 OK", &[("Content-Type", "application/json".into())], &json!({ "access_token": "AT-ONE-x1", "refresh_token": "RT-ONE-x1", "expires_in": 1, "token_type": "Bearer" }).to_string());
                        }
                        Some("refresh_token") => {
                            g.refreshes += 1;
                            let n = g.refreshes + 1;
                            g.access = format!("AT-NEW-x{n}");
                            reply(&mut s, "200 OK", &[("Content-Type", "application/json".into())], &json!({ "access_token": g.access, "refresh_token": format!("RT-NEW-x{n}"), "expires_in": 3600 }).to_string());
                        }
                        _ => {
                            // Revocation (RFC 7009).
                            g.revoked.push(f.get("token").cloned().unwrap_or_default());
                            reply(&mut s, "200 OK", &[], "");
                        }
                    }
                }
                ("POST", "/v1/mcp") => {
                    if auth != format!("Bearer {}", g.access) {
                        reply(&mut s, "401 Unauthorized", &[("WWW-Authenticate", "Bearer".into())], "{}");
                        continue;
                    }
                    let v: Value = serde_json::from_str(&body).unwrap_or_default();
                    let id = v["id"].clone();
                    let result = match v["method"].as_str() {
                        Some("initialize") => json!({ "protocolVersion": "2025-06-18", "capabilities": { "tools": {} }, "serverInfo": { "name": "fake-atlassian", "version": "1" } }),
                        Some("tools/call") => {
                            let name = v["params"]["name"].as_str().unwrap_or("").to_owned();
                            g.tools.push(name.clone());
                            let text = match name.as_str() {
                                "getAccessibleAtlassianResources" => tool_result(json!([{ "id": "cloud-1", "url": "https://acme.atlassian.net", "name": "Acme", "scopes": ["read:jira-work"] }])),
                                "atlassianUserInfo" => tool_result(json!({ "accountId": "acc-1", "name": "Budi QA", "email": "budi@acme.test" })),
                                // A duplicate search (summary ~ …) finds nothing.
                                "searchJiraIssuesUsingJql" if body.contains("summary ~") => tool_result(json!({ "issues": [] })),
                                "searchJiraIssuesUsingJql" => tool_result(json!({ "issues": [{ "key": "KAN-1", "fields": { "summary": "Login fails", "project": { "key": "KAN", "name": "Kanban" } } }] })),
                                "createJiraIssue" => tool_result(json!({ "key": "KAN-7" })),
                                "getJiraIssue" => tool_result(json!({ "key": "KAN-1", "fields": { "summary": "Login fails", "status": { "name": "To Do" }, "issuetype": { "name": "Bug" }, "description": "Open the login page and sign in." } })),
                                _ => tool_result(json!({ "error": "unexpected tool" })),
                            };
                            json!({ "content": [{ "type": "text", "text": text }] })
                        }
                        _ => {
                            reply(&mut s, "202 Accepted", &[], "");
                            continue;
                        }
                    };
                    reply(&mut s, "200 OK", &[("Content-Type", "application/json".into()), ("Mcp-Session-Id", "sess-1".into())], &json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string());
                }
                _ => reply(&mut s, "404 Not Found", &[], ""),
            }
        }
    });
    (base, seen)
}

/// A raw HTTP request; `(status, headers, body)`.
fn raw(port: u16, method: &str, path: &str, headers: &[(&str, String)], body: &str) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).expect("write");
    let mut out = String::new();
    s.read_to_string(&mut out).expect("read");
    let status = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let (head, body) = out.split_once("\r\n\r\n").map(|(h, b)| (h.to_owned(), b.to_owned())).unwrap_or_default();
    (status, head, body)
}

struct Ui {
    child: std::process::Child,
    port: u16,
    cookie: String,
}
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Ui {
    fn start(home: &Path, base: &str) -> Ui {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .args(["ui", "--no-open"])
            .current_dir(home)
            .env("HOME", home)
            .env("FASTPILOT_KEYSTORE", "file")
            .env("FASTPILOT_ATLASSIAN_TEST_BASE", base)
            .env_remove("FASTPILOT_CONFIG")
            .env_remove("FASTPILOT_JIRA_TEST_BASE")
            .env("FASTPILOT_LIVE_DIR", home.join("live"))
            .env("FASTPILOT_CLAUDE", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude-chat/claude"))
            .env("FAKE_CHAT_DELAY_MS", "0")
            .env("FAKE_CHAT_NO_RUN", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("ui");
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
        let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("link").to_owned();
        let (port, code) = rest.split_once("/?code=").expect("code");
        let port: u16 = port.parse().expect("port");
        let (_, head, _) = raw(port, "GET", &format!("/?code={code}"), &[], "");
        let cookie = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().split(';').next().unwrap_or("").to_owned())).expect("cookie");
        Ui { child, port, cookie }
    }
    fn post(&self, body: Value) -> (u16, Value) {
        self.post_to("/api/jira", body)
    }
    fn post_to(&self, path: &str, body: Value) -> (u16, Value) {
        let h = [("Cookie", self.cookie.clone()), ("Origin", format!("http://127.0.0.1:{}", self.port)), ("Content-Type", "application/json".to_owned())];
        let (st, _, text) = raw(self.port, "POST", path, &h, &body.to_string());
        (st, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
    fn get(&self, path: &str) -> Value {
        let (_, _, text) = raw(self.port, "GET", path, &[("Cookie", self.cookie.clone())], "");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }
    fn state(&self) -> Value {
        let (_, _, text) = raw(self.port, "GET", "/api/jira", &[("Cookie", self.cookie.clone())], "");
        serde_json::from_str(&text).expect("json")
    }
}

fn holding(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(p);
            } else if std::fs::read(&p).is_ok_and(|b| String::from_utf8_lossy(&b).contains(needle)) {
                found.push(p);
            }
        }
    }
    found
}

#[tokio::test(flavor = "multi_thread")]
async fn sign_in_with_atlassian_connects_reads_refreshes_and_disconnect_revokes() {
    let home = std::env::temp_dir().join(format!("fp-jira-signin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    let (base, seen) = atlassian();
    let ui = Ui::start(&home, &base);

    // Connect: Atlassian's sign-in page, for this dashboard's exact return address, with PKCE.
    let (st, r) = ui.post(json!({ "action": "connect" }));
    assert_eq!(st, 200, "{r}");
    let url = reqwest::Url::parse(r["url"].as_str().expect("url")).expect("parse");
    assert!(url.as_str().starts_with(&format!("{base}/v1/authorize?")), "{url}");
    let q: HashMap<String, String> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let back = format!("http://127.0.0.1:{}/oauth/atlassian/callback", ui.port);
    assert_eq!(q.get("redirect_uri"), Some(&back));
    assert_eq!(q.get("code_challenge_method").map(String::as_str), Some("S256"));
    assert_eq!(q.get("state").map(String::len), Some(43), "32 random bytes, base64url");

    // A return with another state is refused, before any token is asked for.
    let (st, _, page) = raw(ui.port, "GET", "/oauth/atlassian/callback?code=CODE-1&state=not-ours", &[], "");
    assert_eq!(st, 400, "{page}");
    assert!(page.contains("Jira is not connected"));
    assert_eq!(seen.lock().expect("seen").exchanges, 0, "no code was traded");

    // The person signs in and accepts; Atlassian sends the browser back (no dashboard cookie).
    let at = url.as_str().strip_prefix(&base).expect("on the fake");
    let fake_port: u16 = base.rsplit(':').next().and_then(|p| p.parse().ok()).expect("port");
    let mut s = TcpStream::connect(("127.0.0.1", fake_port)).expect("fake");
    s.write_all(format!("GET {at} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes()).expect("write");
    let mut out = String::new();
    s.read_to_string(&mut out).expect("read");
    let location = out.lines().find_map(|l| l.strip_prefix("Location: ")).expect("redirect back").trim().to_owned();
    let path = location.strip_prefix(&format!("http://127.0.0.1:{}", ui.port)).expect("to this dashboard").to_owned();
    let (st, _, page) = raw(ui.port, "GET", &path, &[], "");
    assert_eq!(st, 200, "{page}");
    assert!(page.contains("Jira is connected") && page.contains("url=/#/integrations/jira"), "{page}");
    // The same return again: the state worked once.
    let (st, _, _) = raw(ui.port, "GET", &path, &[], "");
    assert_eq!(st, 400, "a state is used once");
    assert_eq!(seen.lock().expect("seen").exchanges, 1);

    // Connected: the site, the person and the most recent project, with no token anywhere.
    let v = ui.state();
    assert_eq!(v["connected"], true, "{v}");
    let s = &v["settings"];
    assert_eq!((s["site"].as_str(), s["display_name"].as_str(), s["project_key"].as_str(), s["auth"].as_str()), (Some("acme.atlassian.net"), Some("Budi QA"), Some("KAN"), Some("atlassian")), "{v}");
    assert!(!v.to_string().contains("AT-") && !v.to_string().contains("RT-"), "{v}");
    assert!(seen.lock().expect("seen").refreshes >= 1, "the expired access token was refreshed");

    // Claude Code's fastpilot reads a ticket through Atlassian's MCP, as data.
    let h = home.clone();
    let b = base.clone();
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(&h).env("HOME", &h).env("FASTPILOT_KEYSTORE", "file").env("FASTPILOT_ATLASSIAN_TEST_BASE", &b).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_JIRA_TEST_BASE").env("FASTPILOT_LIVE_DIR", h.join("live"));
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let res = client.call_tool(rmcp::model::CallToolRequestParams::new("jira_read").with_arguments(json!({ "key": "KAN-1" }).as_object().cloned().unwrap_or_default())).await.expect("call");
    let text = res.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect::<Vec<_>>().join("\n");
    assert!(res.is_error != Some(true) && text.contains("Login fails") && text.contains("Open the login page"), "{text}");
    assert!(!text.contains("AT-") && !text.contains("RT-"), "{text}");
    client.cancel().await.expect("shutdown");
    {
        let g = seen.lock().expect("seen");
        let allowed = ["atlassianUserInfo", "getAccessibleAtlassianResources", "getJiraIssue", "searchJiraIssuesUsingJql", "createJiraIssue", "addOrEditJiraIssueComment"];
        assert!(g.tools.iter().all(|t| allowed.contains(&t.as_str())), "only Jira tools: {:?}", g.tools);
    }
    for t in ["AT-ONE-x1", "RT-ONE-x1", "AT-NEW", "RT-NEW"] {
        assert!(holding(&home, t).is_empty(), "{t} is written out as text");
    }

    // QA #5980 bug 2: everything that asks "is Jira connected?" knows the sign-in. The terminal…
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["jira", "status"])
        .current_dir(&home)
        .env("HOME", &home)
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_ATLASSIAN_TEST_BASE", &base)
        .env_remove("FASTPILOT_CONFIG")
        .output()
        .expect("status");
    let said = String::from_utf8_lossy(&status.stdout);
    assert!(said.contains("connected to acme.atlassian.net as Budi QA (Sign in with Atlassian)"), "{said}");
    // …and Chat: a "Create Jira issue" card, and the issue on the click.
    let (st, r) = ui.post_to("/api/chat", json!({ "message": "make a jira ticket for the voucher bug" }));
    assert_eq!(st, 200, "{r}");
    let mut chat = Value::Null;
    for _ in 0..200 {
        chat = ui.get("/api/chat");
        if chat["busy"] == false && chat["messages"].as_array().is_some_and(|m| m.len() >= 2) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    let card = chat["messages"][1]["proposals"][0].clone();
    assert_eq!((card["kind"].as_str(), card["status"].as_str()), (Some("jira_issue"), Some("pending")), "{chat}");
    let (st, r) = ui.post_to("/api/chat/decide", json!({ "id": card["id"], "apply": true }));
    assert!(st == 200 && r.to_string().contains("KAN-7"), "{st} {r}");
    assert!(seen.lock().expect("seen").tools.iter().any(|t| t == "createJiraIssue"));

    // Disconnect: revoked at Atlassian, then gone here.
    let (st, r) = ui.post(json!({ "action": "disconnect" }));
    assert_eq!(st, 200, "{r}");
    assert!(r["ok"].as_str().unwrap_or("").contains("revoked"), "{r}");
    let revoked = seen.lock().expect("seen").revoked.clone();
    assert!(revoked.iter().any(|t| t.starts_with("RT-")), "the refresh token was revoked: {revoked:?}");
    assert_eq!(ui.state()["connected"], false);
    assert!(!home.join(".fastpilot/keys/jira.oauth").exists(), "removed from the key store");
    assert!(!home.join(".fastpilot/jira-oauth.json").exists(), "the registration is forgotten too");

    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}

/// The same, the way a person does it in Chrome: one click on "Sign in with Atlassian", the sign-in
/// page (the fake accepts at once), and back on the dashboard, connected. Skips without Chrome.
#[tokio::test(flavor = "multi_thread")]
async fn one_click_in_the_dashboard_signs_in_and_comes_back_connected() {
    if !["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists()) {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-jira-signin-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("person")).expect("home");
    let (base, seen) = atlassian();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(&home)
        .env("HOME", &home)
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_ATLASSIAN_TEST_BASE", &base)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_JIRA_TEST_BASE")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let person = home.join("person");
    let p = person.clone();
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").env("HOME", &p).env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").env("FASTPILOT_LIVE_DIR", p.join("live")).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let run = |steps: String| {
        let client = &client;
        async move {
            let mut args = json!({ "flow": steps }).as_object().cloned().unwrap_or_default();
            if !steps.contains("start: person") {
                args.insert("session".into(), json!("person"));
            }
            let res = client.call_tool(rmcp::model::CallToolRequestParams::new("run_flow").with_arguments(args)).await.expect("call");
            (res.is_error != Some(true), res.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect::<Vec<_>>().join("\n"))
        }
    };
    let (ok, out) = run(format!("- session: {{ start: person, platform: web, target: \"{}#/integrations/jira\" }}\n- assertVisible: {{ text: \"Sign in with Atlassian\", timeout: 10000 }}\n", link.trim())).await;
    assert!(ok, "{out}");
    let (ok, out) = run("- tapOn: Sign in with Atlassian\n- assertVisible: { text: \"Connected as\", timeout: 15000 }\n- assertVisible: Budi QA\n- assertVisible: acme.atlassian.net\n- assertVisible: Disconnect Jira…\n".into()).await;
    assert!(ok, "back on the dashboard, connected: {out}");
    assert_eq!(seen.lock().expect("seen").exchanges, 1);
    let _ = client.cancel().await;
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&home);
}
