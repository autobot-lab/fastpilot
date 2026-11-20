//! Who may take a session over (security review Medium / Low): the control socket serves only a
//! `fastpilot ui` process, a takeover belongs to the page that started it (its token never leaves
//! that process), and a takeover whose page went away ends by itself so the AI's step goes on.
//! Skips without Chrome or off Unix.

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

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
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

fn request(ui: &Ui, method: &str, path: &str, body: &str) -> (u16, String) {
    let port = ui.port;
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        ui.cookie,
        body.len()
    );
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, raw.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default())
}

fn start_ui(live: &Path) -> Ui {
    start_ui_from(Path::new(env!("CARGO_BIN_EXE_fastpilot")), live)
}

fn start_ui_from(program: &Path, live: &Path) -> Ui {
    let mut child = std::process::Command::new(program)
        .args(["ui", "--no-open"])
        .env("FASTPILOT_LIVE_DIR", live)
        .env_remove("FASTPILOT_LIVE_ANY_PEER")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
    let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("link").to_owned();
    let (port, code) = rest.split_once("/?code=").expect("code");
    let port: u16 = port.parse().expect("port");
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.write_all(format!("GET /?code={code} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let cookie = raw.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().to_owned())).and_then(|c| c.split(';').next().map(str::to_owned)).expect("cookie");
    Ui { child, port, cookie }
}

fn session(ui: &Ui, id: &str) -> Value {
    let (_, body) = request(ui, "GET", "/api/live", "");
    let v: Value = serde_json::from_str(&body).expect("json");
    v["sessions"].as_array().expect("sessions").iter().find(|s| s["id"] == id).cloned().unwrap_or_else(|| panic!("{v}"))
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn takeovers_belong_to_the_page_and_end_when_it_goes() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let live = std::env::temp_dir().join(format!("fp-take-guard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&live);
    let (l2, home) = (live.clone(), live.join("home"));
    std::fs::create_dir_all(&home).expect("home");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(ROOT).env("HOME", &home).env("FASTPILOT_LIVE_DIR", &l2).env("FASTPILOT_TAKEOVER_IDLE_SECS", "4").env_remove("FASTPILOT_LIVE_ANY_PEER").env_remove("FASTPILOT_CONFIG");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let page = format!("file://{ROOT}/examples/demo/login.html");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- session: {{ start: shop, platform: web, target: \"{page}\" }}\n") })).await;
    assert!(ok, "{out}");

    // A script (here: this test) on the socket itself is turned away, for every request.
    let sock = std::fs::read_dir(&live).expect("live").flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "sock")).expect("socket");
    for req in [r#"{"op":"takeover","session":"shop"}"#, r#"{"op":"frame","session":"shop"}"#] {
        let mut s = std::os::unix::net::UnixStream::connect(&sock).expect("connect");
        s.write_all(format!("{req}\n").as_bytes()).expect("write");
        let mut answer = String::new();
        BufReader::new(s).read_line(&mut answer).expect("read");
        assert!(answer.contains("only fastpilot ui may use this socket"), "{answer}");
    }

    // Two pages: the takeover belongs to the one that started it.
    let a = start_ui(&live);
    let b = start_ui(&live);
    let pid = session(&a, "shop")["pid"].as_u64().expect("pid");
    let body = |op: &str| json!({ "pid": pid, "session": "shop", "op": op, "x": 0.5, "y": 0.5 }).to_string();
    let (st, out) = request(&a, "POST", "/api/live/control", &body("takeover"));
    assert_eq!(st, 200, "{out}");
    let (st, out) = request(&b, "POST", "/api/live/control", &body("takeover"));
    assert!(st == 409 && out.contains("halaman lain"), "{st} {out}");
    for op in ["tap", "release"] {
        let (st, out) = request(&b, "POST", "/api/live/control", &body(op));
        assert!(st == 409 && out.contains("dari halaman ini"), "{op}: {st} {out}");
    }
    assert_eq!((session(&a, "shop")["taken_over_here"].as_bool(), session(&b, "shop")["taken_over_here"].as_bool()), (Some(true), Some(false)));
    assert!(session(&b, "shop")["taken_over_ms"].is_u64());

    {
        // The AI's step waits; the page that took over goes away; the takeover ends by itself.
        let waiting = call(&client, "run_flow", json!({ "flow": "- assertVisible: Masuk\n", "session": "shop" }));
        tokio::pin!(waiting);
        assert!(tokio::time::timeout(Duration::from_millis(1500), &mut waiting).await.is_err(), "waits during the takeover");
        drop(a);
        let started = Instant::now();
        let (ok, out) = tokio::time::timeout(Duration::from_secs(20), waiting).await.expect("the step goes on after the page went away");
        assert!(ok && out.contains("ended by itself because the fastpilot ui page stopped answering"), "{out}");
        assert!(started.elapsed() < Duration::from_secs(12), "{:?}", started.elapsed());
        assert!(session(&b, "shop")["taken_over_ms"].is_null());
    }
    drop(b);
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(live);
}

/// v0.3.1 (FastTest T1): a program merely named `fastpilot`, run with `ui`, is not this fastpilot:
/// a copy of the binary elsewhere is turned away; and `allow_takeover = false` switches takeover off.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn takeover_needs_this_installation_and_can_be_switched_off() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let live = std::env::temp_dir().join(format!("fp-take-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&live);
    let home = live.join("home");
    std::fs::create_dir_all(&home).expect("home");
    let page = format!("file://{ROOT}/examples/demo/login.html");
    let mcp = |off: bool| {
        let (l2, h2) = (live.clone(), home.clone());
        let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
            c.arg("mcp").current_dir(ROOT).env("HOME", &h2).env("FASTPILOT_LIVE_DIR", &l2).env_remove("FASTPILOT_LIVE_ANY_PEER").env_remove("FASTPILOT_CONFIG");
            if off {
                c.env("FASTPILOT_ALLOW_TAKEOVER", "false");
            } else {
                c.env_remove("FASTPILOT_ALLOW_TAKEOVER");
            }
        });
        TokioChildProcess::new(cmd).expect("spawn")
    };
    let on: Client = ().serve(mcp(false)).await.expect("initialize");
    let off: Client = ().serve(mcp(true)).await.expect("initialize");
    for (c, id) in [(&on, "shop"), (&off, "locked")] {
        let (ok, out) = call(c, "run_flow", json!({ "flow": format!("- session: {{ start: {id}, platform: web, target: \"{page}\" }}\n") })).await;
        assert!(ok, "{out}");
    }

    // A copy of the real binary in another folder, still named `fastpilot`, run as `fastpilot ui`.
    let other = live.join("elsewhere");
    std::fs::create_dir_all(&other).expect("dir");
    let copy = other.join("fastpilot");
    std::fs::copy(env!("CARGO_BIN_EXE_fastpilot"), &copy).expect("copy");
    let impostor = start_ui_from(&copy, &live);
    let pid = session(&impostor, "shop")["pid"].as_u64().expect("pid");
    let (st, out) = request(&impostor, "POST", "/api/live/control", &json!({ "pid": pid, "session": "shop", "op": "takeover" }).to_string());
    assert!(st == 409 && out.contains("same installation"), "{st} {out}");
    drop(impostor);

    // The real one may; the process started with allow_takeover = false says no, and says so.
    let ui = start_ui(&live);
    let (st, out) = request(&ui, "POST", "/api/live/control", &json!({ "pid": pid, "session": "shop", "op": "takeover" }).to_string());
    assert_eq!(st, 200, "{out}");
    let (st, _) = request(&ui, "POST", "/api/live/control", &json!({ "pid": pid, "session": "shop", "op": "release" }).to_string());
    assert_eq!(st, 200);
    let locked = session(&ui, "locked");
    assert_eq!((locked["takeover_allowed"].as_bool(), session(&ui, "shop")["takeover_allowed"].as_bool()), (Some(false), Some(true)), "{locked}");
    let lpid = locked["pid"].as_u64().expect("pid");
    let (st, out) = request(&ui, "POST", "/api/live/control", &json!({ "pid": lpid, "session": "locked", "op": "takeover" }).to_string());
    assert!(st == 409 && out.contains("allow_takeover = false"), "{st} {out}");
    drop(ui);
    on.cancel().await.expect("shutdown");
    off.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(live);
}
