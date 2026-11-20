//! D10d: agents run side by side; at most `max_parallel` tests at once and one test per device,
//! the rest wait in a queue (visible in Live with the reason) and start by themselves. Two real
//! `fastpilot mcp` processes play two agents. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

type Client = RunningService<RoleClient, ()>;
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists())
}

fn env(home: &Path) -> Vec<(String, String)> {
    vec![
        ("HOME".into(), home.display().to_string()),
        ("FASTPILOT_LIVE_DIR".into(), home.join("live").display().to_string()),
        ("FASTPILOT_MAX_PARALLEL".into(), "1".into()),
    ]
}

async fn mcp(home: &Path, agent: &str, name: &str, queue_timeout: u64) -> Client {
    let mut vars = env(home);
    vars.push(("FASTPILOT_QUEUE_TIMEOUT".into(), queue_timeout.to_string()));
    if queue_timeout < 10 {
        // The device case: a limit that is not reached, so the device is what it waits for.
        vars.retain(|(k, _)| k != "FASTPILOT_MAX_PARALLEL");
        vars.push(("FASTPILOT_MAX_PARALLEL".into(), "3".into()));
    }
    vars.push(("FASTPILOT_AGENT".into(), agent.into()));
    vars.push(("FASTPILOT_AGENT_NAME".into(), name.into()));
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(ROOT).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_ANDROID_SERIAL").envs(vars.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    });
    ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize")
}

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args.as_object().cloned().unwrap_or_default())).await.expect("call");
    (res.is_error != Some(true), res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n"))
}

struct Ui(std::process::Child, u16, String);
impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_ui(home: &Path) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .env_remove("FASTPILOT_CONFIG")
        .envs(env(home))
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
    Ui(child, port, cookie)
}

fn get(ui: &Ui, path: &str) -> Value {
    let mut s = TcpStream::connect(("127.0.0.1", ui.1)).expect("connect");
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nCookie: {}\r\n\r\n", ui.1, ui.2).as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    serde_json::from_str(raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("{}")).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_wait_for_the_parallel_limit_and_for_a_device() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home: PathBuf = std::env::temp_dir().join(format!("fp-parallel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".fastpilot")).expect("home");
    let ui = start_ui(&home);
    let a = mcp(&home, "qa-checkout", "QA Checkout", 60).await;
    let b = mcp(&home, "admin-panel", "Admin Panel", 60).await;
    let (ok, out) = call(&a, "session", json!({ "action": "start", "id": "a1", "platform": "web", "target": "examples/demo/login.html" })).await;
    assert!(ok, "{out}");

    // Over the limit of 1: B waits in the queue, which Live shows with the reason.
    let b_start = tokio::spawn(async move {
        let r = call(&b, "session", json!({ "action": "start", "id": "b1", "platform": "web", "target": "examples/demo/login.html" })).await;
        (b, r)
    });
    let until = Instant::now() + Duration::from_secs(20);
    let live = loop {
        let v = get(&ui, "/api/live");
        if v["queue"].as_array().is_some_and(|q| !q.is_empty()) {
            break v;
        }
        assert!(Instant::now() < until, "no queue: {v}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(live["queue"][0]["reason"], "parallel limit of 1 reached", "{live}");
    assert_eq!((live["running"].as_u64(), live["limit"].as_u64()), (Some(1), Some(1)), "{live}");
    let running = live["sessions"].as_array().and_then(|s| s.iter().find(|x| x["id"] == "a1")).cloned().expect("a1");
    assert_eq!(running["agent"], "qa-checkout", "{running}");
    assert!(!b_start.is_finished(), "B waits");

    // A stops: B starts by itself, and the queue is empty.
    let (ok, out) = call(&a, "session", json!({ "action": "stop", "id": "a1" })).await;
    assert!(ok, "{out}");
    let (b, (ok, out)) = tokio::time::timeout(Duration::from_secs(30), b_start).await.expect("B started").expect("task");
    assert!(ok && out.contains("session b1 started"), "{out}");
    assert!(get(&ui, "/api/live")["queue"].as_array().is_some_and(Vec::is_empty));
    let _ = call(&b, "session", json!({ "action": "stop", "id": "b1" })).await;

    // One test per device: an Android emulator another (living) process holds makes a session wait,
    // with who holds it, then give up after the queue timeout (3 s here). Nothing touches a device.
    let mut holder = std::process::Command::new("sleep").arg("60").spawn().expect("sleep");
    let claims = home.join("claims");
    std::fs::create_dir_all(&claims).expect("claims");
    std::fs::write(claims.join("device-android_emulator-5556.lock"), format!("{} hold", holder.id())).expect("lock");
    std::fs::write(
        claims.join(format!("{}-hold.json", holder.id())),
        json!({ "pid": holder.id(), "session": "hold", "agent": "regression-android", "agent_name": "Regression Android", "platform": "android", "device": "android:emulator-5556", "since_ms": 1, "waiting": false, "reason": "" }).to_string(),
    )
    .expect("entry");
    let c = mcp(&home, "release-smoke", "Release Smoke", 3).await;
    let (ok, out) = call(&c, "session", json!({ "action": "start", "id": "and1", "platform": "android", "device": "emulator-5556" })).await;
    assert!(!ok && out.contains("waited 3 s in the queue and gave up: the Android emulator is in use by Regression Android"), "{out}");
    c.cancel().await.expect("c");
    let _ = holder.kill();
    let _ = holder.wait();
    // The holder is gone: its claim is dropped the next time anyone looks.
    let v = get(&ui, "/api/agents");
    assert!(v["agents"].is_array());
    assert!(!claims.join(format!("{}-hold.json", holder.id())).exists());
    a.cancel().await.expect("a");
    b.cancel().await.expect("b");
    drop(ui);
    let _ = std::fs::remove_dir_all(home);
}
