//! F2 `fastpilot demo`: the demo folder is written from the binary alone (pages, test cases,
//! .mcp.json), a first web test runs from it as Claude Code would start fastpilot there, an existing
//! folder is never overwritten, and Android is skipped without a device. `--android` installs the
//! demo app and the Android login runs on it (needs FASTPILOT_ANDROID_SERIAL or one device).

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

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

/// `fastpilot demo <args>` in `cwd`, with no Android device reachable unless `adb` is true.
fn demo(cwd: &Path, args: &[&str], adb: bool) -> (Option<i32>, String) {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    c.arg("demo").args(args).current_dir(cwd).env("HOME", cwd).env_remove("FASTPILOT_CONFIG");
    if !adb {
        // An adb server nobody listens on: "no device".
        c.env("FASTPILOT_ADB_SERVER", "127.0.0.1:1").env_remove("FASTPILOT_ANDROID_SERIAL").env_remove("ANDROID_SERIAL");
    }
    let out = c.output().expect("demo");
    (out.status.code(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// `fastpilot demo` in a pseudo-terminal (`script`, macOS), answering the dashboard question with
/// `answer`; stops once `until` shows up (or after 60 s) and returns what was printed.
fn demo_in_terminal(cwd: &Path, answer: &str, until: &str) -> String {
    use std::io::{Read, Write};
    let mut c = std::process::Command::new("script");
    c.args(["-q", "/dev/null", env!("CARGO_BIN_EXE_fastpilot"), "demo"])
        .current_dir(cwd)
        .env("HOME", cwd)
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_ADB_SERVER", "127.0.0.1:1")
        .env("FASTPILOT_NO_BROWSER", "1")
        .env("FASTPILOT_LIVE_DIR", cwd.join("live"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut child = c.spawn().expect("script");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(String::from_utf8_lossy(&buf[..n]).into_owned()).is_err() {
                break;
            }
        }
    });
    let (mut out, mut answered) = (String::new(), false);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < end && !out.contains(until) {
        if let Ok(chunk) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
            out.push_str(&chunk.replace('\r', ""));
        }
        if !answered && out.contains("[Y/n]") {
            let _ = stdin.write_all(answer.as_bytes());
            answered = true;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    // The dashboard started under `script` may outlive it.
    let _ = std::process::Command::new("pkill").args(["-f", &format!("{} ui --no-open", env!("CARGO_BIN_EXE_fastpilot"))]).status();
    out
}

/// The dashboard (`fastpilot ui --no-open`) in `folder` with HOME `home`: its port and session
/// cookie, after trading the link's code, and where that first open lands.
struct Dash {
    child: std::process::Child,
    port: u16,
    cookie: String,
    link: String,
    landing: String,
}
impl Drop for Dash {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn http(port: u16, path: &str, cookie: &str) -> String {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\n\r\n").expect("write");
    let mut out = String::new();
    s.read_to_string(&mut out).expect("read");
    out
}
/// `redeem`: trade the link's code here (for the API); else the code is left for a browser.
fn dashboard(folder: &Path, home: &Path, redeem: bool) -> Dash {
    dashboard_with(folder, home, redeem, &[])
}
fn dashboard_with(folder: &Path, home: &Path, redeem: bool, env: &[(&str, &str)]) -> Dash {
    use std::io::BufRead;
    // Its own port, before `--no-open`: `demo_offers_the_dashboard` stops every "ui --no-open" of
    // this binary when it ends, and may run at the same time.
    let port = std::net::TcpListener::bind(("127.0.0.1", 0)).and_then(|l| l.local_addr()).map(|a| a.port()).expect("free port").to_string();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--port", &port, "--no-open"])
        .current_dir(folder)
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .envs(env.iter().copied())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut link = String::new();
    std::io::BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut link).expect("link");
    let link = link.trim().to_owned();
    let rest = link.strip_prefix("http://127.0.0.1:").expect("link");
    let port: u16 = rest.split('/').next().and_then(|p| p.parse().ok()).expect("port");
    if !redeem {
        return Dash { child, port, cookie: String::new(), link, landing: String::new() };
    }
    let head = http(port, &format!("/{}", rest.split_once('/').map(|(_, r)| r).unwrap_or("")), "");
    let header = |name: &str| head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_owned())).unwrap_or_default();
    let cookie = header("set-cookie").split(';').next().unwrap_or("").to_owned();
    Dash { child, port, cookie, link, landing: header("location") }
}
fn api(d: &Dash, path: &str) -> Value {
    let out = http(d.port, path, &d.cookie);
    serde_json::from_str(out.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")).unwrap_or_else(|e| panic!("{path}: {e}\n{out}"))
}

/// E2 (v0.7.2, found by FastGrowth on 0.7.1): `fastpilot demo` opened a dashboard that said "No app
/// yet", with the demo's tests nowhere in sight. Now the demo folder is the app "fastpilot demo"
/// (a website) with its agent "Demo shop"; the dashboard opens on that agent's Chat, which lists the
/// three website tests of the folder's `tests/`. No form, no question; `--force` again adds nothing.
#[tokio::test(flavor = "multi_thread")]
async fn demo_opens_the_dashboard_on_the_demo_agent_with_its_tests() {
    let tmp = std::env::temp_dir().join(format!("fp-demo-e2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("tmp");
    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .arg("demo")
            .args(args)
            .current_dir(&tmp)
            .env("HOME", &tmp)
            .env_remove("FASTPILOT_CONFIG")
            .env_remove("FASTPILOT_DIR")
            .env("FASTPILOT_KEYSTORE", "file")
            .env("FASTPILOT_ADB_SERVER", "127.0.0.1:1")
            .env_remove("FASTPILOT_ANDROID_SERIAL")
            .output()
            .expect("demo");
        (out.status.code(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let (code, out) = run(&[]);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("app “fastpilot demo”") && out.contains("agent “Demo shop” in the dashboard"), "{out}");
    let folder = tmp.join("fastpilot-demo");
    let title = |rel: &str| std::fs::read_to_string(folder.join(rel)).unwrap_or_default().lines().next().unwrap_or("").to_owned();
    assert_eq!((title("tests/login.md"), title("tests/cart.md"), title("tests/card-form.md")), ("# Check the login".into(), "# Add a product to the cart".into(), "# Fill in the card form (stops before Pay)".into()));
    assert!(folder.join("tests/android/login-android.md").is_file() && !folder.join("tests/login-android.md").exists(), "the Android tests are kept apart");

    let check = |d: &Dash| {
        let agents = api(d, "/api/agents");
        let list = agents["agents"].as_array().cloned().unwrap_or_default();
        let demo: Vec<&Value> = list.iter().filter(|x| x["agent"]["name"] == "Demo shop").collect();
        assert_eq!(demo.len(), 1, "one demo agent: {agents}");
        let x = demo[0];
        assert_eq!(x["app_line"], "fastpilot demo · Website", "{x}");
        assert_eq!(x["demo"]["tests"], json!(["Add a product to the cart", "Check the login", "Fill in the card form (stops before Pay)"]), "{x}");
        assert!(!list.iter().any(|y| y["app_line"] == "No app yet"), "nothing says No app yet: {agents}");
        assert_eq!((agents["saved_tests"].as_u64(), agents["current_app"].clone()), (Some(3), x["app_id"].clone()), "{agents}");
        let apps = api(d, "/api/apps");
        assert_eq!(apps["apps"].as_array().map(Vec::len), Some(1), "one app: {apps}");
        assert_eq!(apps["apps"][0]["name"], "fastpilot demo", "{apps}");
        assert_eq!(d.landing, format!("/#/agents/{}/chat", x["agent"]["id"].as_str().unwrap_or("")), "the first open lands in its Chat");
    };
    let d = dashboard(&folder, &tmp, true);
    check(&d);
    drop(d);

    // The page itself, opened from a new link: the welcome of e2a, with the three tests by name, in
    // the demo agent's Chat, and no "No app yet".
    if chrome_available() {
        let d = dashboard(&folder, &tmp, false);
        let viewer = tmp.join("viewer");
        std::fs::create_dir_all(&viewer).expect("viewer");
        let v2 = viewer.clone();
        let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
            c.arg("mcp").current_dir(ROOT).env("HOME", &v2).env("FASTPILOT_LIVE_DIR", v2.join("live")).env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
        });
        let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
        let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- session: {{ start: viewer, platform: web, target: \"{}\" }}\n- assertVisible: {{ text: \"This is the demo.\", timeout: 15000 }}\n- assertVisible: \"Fill in the card form (stops before Pay)\"\n- assertVisible: \"Add a product to the cart\"\n- assertVisible: \"Check the login\"\n- assertVisible: \"run the demo tests\"\n- assertNotVisible: \"No app yet\"\n- assertNotVisible: \"What should I test?\"\n", d.link) })).await;
        assert!(ok, "{out}");
        let (ok, out) = call(&client, "run_flow", json!({ "flow": "- evalScript: \"location.hash\"\n", "session": "viewer" })).await;
        assert!(ok && out.contains("#/agents/default/chat"), "the demo agent's Chat: {out}");
        // The practice shop works for the tests: add to the cart; the card form, with Pay never pressed.
        let products = url::Url::from_file_path(folder.join("demo/products.html")).expect("url");
        let checkout = url::Url::from_file_path(folder.join("demo/checkout.html")).expect("url");
        let (ok, out) = call(&client, "run_flow", json!({ "flow": format!("- session: {{ start: shop, platform: web, target: \"{products}\" }}\n- tapOn: \"Add Blue mug to cart\"\n- assertVisible: \"Cart: 1 item (Blue mug)\"\n- openUrl: \"{checkout}\"\n- inputText: {{ into: \"Card number\", text: \"4242 4242 4242 4242\" }}\n- assertVisible: \"Pay\"\n") })).await;
        assert!(ok, "{out}");
        let (ok, out) = call(&client, "run_flow", json!({ "flow": "- tapOn: \"Pay\"\n", "session": "shop" })).await;
        assert!(!ok && out.contains("blocked"), "the built-in limit never presses Pay: {out}");
        let _ = call(&client, "session", json!({ "action": "stop", "id": "shop" })).await;
        let _ = call(&client, "session", json!({ "action": "stop", "id": "viewer" })).await;
        drop(d);
    }

    // `fastpilot demo` again (`--force`, same folder): still one app and one demo agent.
    let (code, out) = run(&["--force"]);
    assert_eq!(code, Some(0), "{out}");
    let d = dashboard(&folder, &tmp, true);
    check(&d);
    drop(d);
    let _ = std::fs::remove_dir_all(&tmp);
}

fn post(d: &Dash, path: &str, body: &Value) -> Value {
    use std::io::{Read, Write};
    let body = body.to_string();
    let mut s = std::net::TcpStream::connect(("127.0.0.1", d.port)).expect("connect");
    write!(s, "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: http://127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCookie: {}\r\n\r\n{body}", d.port, d.port, body.len(), d.cookie).expect("write");
    let mut out = String::new();
    s.read_to_string(&mut out).expect("read");
    serde_json::from_str(out.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")).unwrap_or_else(|e| panic!("{path}: {e}\n{out}"))
}

/// F2 (FastTest v0.7.2): `fastpilot demo`, the dashboard in the demo folder, Chat "run the demo
/// tests": "Check the login" could not run (DEMO_PASS was only in `.mcp.json`, which a run started
/// from Chat does not read) and Chat said only "Running…". Now the demo password is the demo app's
/// own (used on the demo pages only) and the chat gets the result when the run ends.
#[test]
fn the_demo_tests_run_from_chat_all_pass_and_chat_says_so() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let tmp = std::env::temp_dir().join(format!("fp-demo-f2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("tmp");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .arg("demo")
        .current_dir(&tmp)
        .env("HOME", &tmp)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env_remove("FASTPILOT_VARS")
        .env_remove("DEMO_PASS")
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_ADB_SERVER", "127.0.0.1:1")
        .output()
        .expect("demo");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let fake = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude-chat/claude");
    let d = dashboard_with(&tmp.join("fastpilot-demo"), &tmp, true, &[("FASTPILOT_CLAUDE", fake), ("FAKE_CHAT_DELAY_MS", "0"), ("FAKE_CHAT_NO_RUN", "1"), ("FASTPILOT_ADB_SERVER", "127.0.0.1:1")]);
    let sent = post(&d, "/api/chat", &json!({ "message": "run the demo tests" }));
    let chat = sent["chat"].as_str().unwrap_or_default().to_owned();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(240);
    let answer = loop {
        let v = api(&d, &format!("/api/chat?chat={chat}"));
        let msgs = v["messages"].as_array().cloned().unwrap_or_default();
        if let Some(m) = msgs.iter().skip(2).find(|m| m["role"] == "bot") {
            assert_eq!(v["running"], false, "{v}");
            break m["text"].as_str().unwrap_or("").to_owned();
        }
        assert!(std::time::Instant::now() < end, "no result in Chat: {v}");
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    assert!(answer.starts_with("**All 3 demo tests passed.**"), "{answer}");
    for row in ["| Check the login | Passed |", "| Add a product to the cart | Passed |", "| Fill in the card form \\(stops before Pay\\) | Passed |"] {
        assert!(answer.contains(row), "{row}: {answer}");
    }
    assert!(answer.trim_end().ends_with("To try your own website, paste its address."), "{answer}");
    drop(d);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// D3 (v9 G7): the demo ends by offering the dashboard for the demo folder. Outside a terminal, a
/// line says how; in a terminal, "n" leaves it for later and Enter starts it there.
#[test]
fn demo_offers_the_dashboard() {
    let tmp = std::env::temp_dir().join(format!("fp-demo-d3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("tmp");
    let (code, out) = demo(&tmp, &["first"], false);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("Dashboard: run `cd first && fastpilot ui` and keep this window open.") && !out.contains("[Y/n]"), "{out}");
    if cfg!(target_os = "macos") {
        let out = demo_in_terminal(&tmp, "n\n", "Later:");
        assert!(out.contains("Open the dashboard now to chat, schedule and see results?") && out.contains("Later: run `cd fastpilot-demo && fastpilot ui`"), "{out}");
        let _ = std::fs::remove_dir_all(tmp.join("fastpilot-demo"));
        let out = demo_in_terminal(&tmp, "\n", "?code=");
        assert!(out.contains("Starting the dashboard") && out.contains("http://127.0.0.1:") && out.contains("?code="), "{out}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).expect("file")));
            }
        }
    }
    out.sort();
    out
}

/// The MCP server as Claude Code starts it from the demo folder's .mcp.json.
async fn mcp_from(folder: &Path, extra: &[(&str, &str)]) -> Client {
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(folder.join(".mcp.json")).expect(".mcp.json")).expect("json");
    let server = cfg["mcpServers"]["fastpilot"].clone();
    assert_eq!(server["args"].as_array().and_then(|a| a.last()).and_then(Value::as_str), Some("mcp"));
    let env: Vec<(String, String)> = server["env"].as_object().expect("env").iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned())).collect();
    let extra: Vec<(String, String)> = extra.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    let (dir, home) = (folder.to_owned(), folder.join(".home"));
    std::fs::create_dir_all(&home).expect("home");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(&dir).env("HOME", &home).env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_LIVE_DIR").env_remove("FASTPILOT_BLOCK_ACTIONS");
        for (k, v) in env.iter().chain(extra.iter()) {
            c.env(k, v);
        }
    });
    ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize")
}

#[tokio::test(flavor = "multi_thread")]
async fn demo_folder_runs_a_first_web_test_and_is_never_overwritten() {
    let tmp = std::env::temp_dir().join(format!("fp-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("tmp");
    let (code, out) = demo(&tmp, &[], false);
    assert_eq!(code, Some(0), "{out}");
    for line in [
        "Creating the demo folder: ./fastpilot-demo",
        "tests/login.md",
        "tests/cart.md",
        "tests/card-form.md",
        "tests/android/login-android.md",
        "tests/android/login-cross-platform.md",
        "fastpilot + DEMO_PASS",
        "Android (skipped: no device)",
        "fastpilot demo --android",
        "cd fastpilot-demo && claude",
        "\"Use fastpilot to run tests/login.md, then report\"",
        // QA (v0.4): the demo says which account to use, so nobody guesses an email.
        "Demo account: a@b.com, password rahasia123",
        "tests write ${DEMO_PASS}",
    ] {
        assert!(out.contains(line), "missing {line:?}:\n{out}");
    }
    let folder = tmp.join("fastpilot-demo");
    // V042 (QA Low): every demo page scales to a phone (Ship check marked each one without it).
    for e in std::fs::read_dir(format!("{ROOT}/examples/demo")).expect("demo").flatten().filter(|e| e.path().extension().is_some_and(|x| x == "html")) {
        let text = std::fs::read_to_string(e.path()).expect("page");
        assert!(text.contains(r#"<meta name="viewport" content="width=device-width, initial-scale=1">"#), "{} has no viewport meta tag", e.path().display());
    }
    for page in ["login", "widgets", "pin", "secret", "maestro"] {
        let repo = std::fs::read(format!("{ROOT}/examples/demo/{page}.html")).expect("repo page");
        assert_eq!(std::fs::read(folder.join(format!("demo/{page}.html"))).expect("page"), repo, "{page}");
    }
    let login = std::fs::read_to_string(folder.join("tests/login.md")).expect("test");
    assert!(login.contains("target: demo/login.html") && login.contains("a@b.com / ${DEMO_PASS}"), "{login}");
    let page = std::fs::read_to_string(folder.join("demo/login.html")).expect("page");
    assert!(page.contains("Demo account: a@b.com, password ${DEMO_PASS}") && !page.contains("Demo account: a@b.com, password rahasia123"), "the page names the account, the password as its variable");
    // QA Low 4: the Skill is in the demo folder, so Claude knows the steps.
    let skill = std::fs::read(format!("{ROOT}/skills/fastpilot/SKILL.md")).expect("repo skill");
    assert_eq!(std::fs::read(folder.join(".claude/skills/fastpilot/SKILL.md")).expect("skill"), skill);
    assert!(out.contains("24 skills"), "{out}");
    // S5 (v0.4): the whole skill library is in the demo folder too.
    let regression = std::fs::read(format!("{ROOT}/skills/regression/SKILL.md")).expect("repo regression");
    assert_eq!(std::fs::read(folder.join(".claude/skills/regression/SKILL.md")).expect("regression"), regression);

    // Already there: nothing changes, exit ≠ 0; --force writes it again.
    let before = snapshot(&folder);
    std::fs::write(folder.join("tests/login.md"), "mine").expect("edit");
    let edited = snapshot(&folder);
    let (code, out) = demo(&tmp, &[], false);
    assert!(code != Some(0) && out.contains("already exists"), "{code:?} {out}");
    assert_eq!(snapshot(&folder), edited, "nothing overwritten");
    let (code, _) = demo(&tmp, &["--force"], false);
    assert_eq!(code, Some(0));
    assert_eq!(snapshot(&folder), before, "--force writes the demo again");
    // --android without a device: skipped, exit 0.
    let (code, out) = demo(&tmp, &["--android"], false);
    assert!(code == Some(0) && out.contains("skipped: no device"), "{code:?} {out}");
    // QA Low 5: run from inside the demo folder: warned that the new one is nested.
    let (code, out) = demo(&folder, &[], false);
    assert!(code == Some(0) && out.contains("inside a demo folder") && out.contains("cd .."), "{code:?} {out}");
    let _ = std::fs::remove_dir_all(folder.join("fastpilot-demo"));
    let (_, out) = demo(&tmp, &["elsewhere"], false);
    assert!(!out.contains("di dalam folder demo"), "no warning outside a demo folder: {out}");

    // Security Low 2: --force keeps the user's own .mcp.json (other servers stay, a backup is made).
    let mine = "{\n  \"mcpServers\": {\n    \"other\": { \"command\": \"other-mcp\" }\n  }\n}\n";
    std::fs::write(folder.join(".mcp.json"), mine).expect("mine");
    let (code, out) = demo(&tmp, &["--force"], false);
    assert!(code == Some(0) && out.contains("was saved as .mcp.json.bak"), "{code:?} {out}");
    assert_eq!(std::fs::read_to_string(folder.join(".mcp.json.bak")).expect("backup"), mine);
    let merged: Value = serde_json::from_str(&std::fs::read_to_string(folder.join(".mcp.json")).expect("merged")).expect("json");
    assert_eq!(merged["mcpServers"]["other"]["command"], "other-mcp", "the other server stays");
    assert_eq!(merged["mcpServers"]["fastpilot"]["env"]["FASTPILOT_VARS"], "DEMO_PASS");
    let (_, out) = demo(&tmp, &["--force"], false);
    assert!(!out.contains(".mcp.json.bak") && !folder.join(".mcp.json.bak-2").exists(), "nothing to save the second time: {out}");
    std::fs::remove_file(folder.join(".mcp.json.bak")).expect("rm");

    // Security Low 1: never written through a symlink (a folder inside, or the demo folder itself).
    #[cfg(unix)]
    {
        let outside = tmp.join("outside");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::rename(folder.join("tests"), tmp.join("tests-real")).expect("move");
        std::os::unix::fs::symlink(&outside, folder.join("tests")).expect("link");
        let (code, out) = demo(&tmp, &["--force"], false);
        assert!(code != Some(0) && out.contains("symlink"), "{code:?} {out}");
        assert_eq!(std::fs::read_dir(&outside).expect("outside").count(), 0, "nothing written outside");
        std::fs::remove_file(folder.join("tests")).expect("unlink");
        std::fs::rename(tmp.join("tests-real"), folder.join("tests")).expect("back");
        std::os::unix::fs::symlink(&outside, tmp.join("linked-demo")).expect("link");
        let (code, out) = demo(&tmp, &["linked-demo", "--force"], false);
        assert!(code != Some(0) && out.contains("symlink"), "{code:?} {out}");
        assert_eq!(std::fs::read_dir(&outside).expect("outside").count(), 0);
        // T1-T3 (Security Low 3): a symlinked .mcp.json is refused before it is read, so its
        // target (here a "private" file) is never copied into .mcp.json.bak.
        let private = tmp.join("private.txt");
        std::fs::write(&private, "PRIVATE-CONTENT").expect("private");
        std::fs::remove_file(folder.join(".mcp.json")).expect("rm mcp");
        std::os::unix::fs::symlink(&private, folder.join(".mcp.json")).expect("link mcp");
        let (code, out) = demo(&tmp, &["--force"], false);
        assert!(code != Some(0) && out.contains(".mcp.json is a symlink"), "{code:?} {out}");
        let copied = std::fs::read_dir(&folder).expect("folder").flatten().any(|e| std::fs::read_to_string(e.path()).is_ok_and(|t| t.contains("PRIVATE-CONTENT")) && e.file_name() != ".mcp.json");
        assert!(!copied, "the link's target was copied into the demo folder");
        assert_eq!(std::fs::read_to_string(&private).expect("private"), "PRIVATE-CONTENT", "the target is untouched");
        std::fs::remove_file(folder.join(".mcp.json")).expect("unlink mcp");
        let (code, out) = demo(&tmp, &["--force"], false);
        assert_eq!(code, Some(0), "the demo folder is whole again: {out}");
    }

    // Another folder name.
    let (code, out) = demo(&tmp, &["coba lagi"], false);
    assert!(code == Some(0) && out.contains("cd \"coba lagi\" && claude") && tmp.join("coba lagi/.mcp.json").exists(), "{out}");

    // The first web test, from the demo folder as Claude Code starts fastpilot there.
    if !chrome_available() {
        return eprintln!("skipping the web run: no Chrome");
    }
    let client = mcp_from(&folder, &[]).await;
    // The demo folder's fastpilot tells Claude Code the demo account (MCP instructions).
    let told = client.peer_info().and_then(|i| i.instructions.clone()).unwrap_or_default();
    assert!(told.contains("email a@b.com, password ${DEMO_PASS}") && told.contains("do not ask the user") && !told.contains("rahasia123"), "{told}");
    let flow = "- session: { start: web, platform: web, target: demo/login.html }\n- inputText: { into: Email, text: a@b.com }\n- inputText: { into: Password, text: \"${DEMO_PASS}\" }\n- tapOn: Masuk\n- assertVisible: Budi\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok && out.contains("ok 5/5"), "{out}");
    assert!(!out.contains("rahasia123"), "the demo password stays masked: {out}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[path = "support/android_device.rs"]
mod android_device;

/// `--android` on the emulator named in FASTPILOT_ANDROID_SERIAL. No device fails, saying why
/// (FASTPILOT_SKIP_ANDROID=1 skips on purpose).
#[tokio::test(flavor = "multi_thread")]
async fn demo_android_installs_the_app_and_its_login_runs() {
    let Some(serial) = android_device::device() else { return };
    android_device::need_file(&format!("{ROOT}/examples/demo-android/out/fastpilot-demo.apk"), "the demo app");
    let adb = android_device::adb();
    // Start without the app, so the install is the demo's.
    let _ = std::process::Command::new(&adb).args(["-s", &serial, "uninstall", "com.fastpilot.demo"]).output();
    let tmp = std::env::temp_dir().join(format!("fp-demo-and-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("tmp");

    // QA Low 3: a demo app signed with another key is installed: the message says how to fix it.
    let sdk = std::env::var("ANDROID_HOME").unwrap_or_else(|_| format!("{}/Library/Android/sdk", std::env::var("HOME").unwrap_or_default()));
    let signer = std::fs::read_dir(format!("{sdk}/build-tools")).ok().and_then(|d| d.flatten().map(|e| e.path().join("apksigner")).filter(|p| p.exists()).max());
    if let Some(signer) = signer {
        let key = tmp.join("other.jks");
        let apk = tmp.join("other.apk");
        std::fs::copy(format!("{ROOT}/examples/demo-android/out/fastpilot-demo.apk"), &apk).expect("copy");
        let made = std::process::Command::new("keytool")
            .args(["-genkeypair", "-keystore"]).arg(&key)
            .args(["-storepass", "android", "-keypass", "android", "-alias", "other", "-keyalg", "RSA", "-keysize", "2048", "-validity", "1", "-dname", "CN=other"])
            .output()
            .is_ok_and(|o| o.status.success());
        let signed = made
            && std::process::Command::new(&signer).args(["sign", "--ks"]).arg(&key).args(["--ks-pass", "pass:android"]).arg(&apk).output().is_ok_and(|o| o.status.success());
        if signed && std::process::Command::new(&adb).args(["-s", &serial, "install", "-r"]).arg(&apk).output().is_ok_and(|o| o.status.success()) {
            let (code, out) = demo(&tmp, &["other-demo", "--android", "--device", &serial], true);
            assert_eq!(code, Some(1), "{out}");
            assert!(out.contains(&format!("adb -s {serial} uninstall com.fastpilot.demo")) && !out.contains("INSTALL_FAILED"), "{out}");
        } else {
            eprintln!("note: could not install a differently signed demo app; the hint is covered by the unit test");
        }
        let _ = std::process::Command::new(&adb).args(["-s", &serial, "uninstall", "com.fastpilot.demo"]).output();
    }
    let (code, out) = demo(&tmp, &["--android", "--device", &serial], true);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains(&format!("✓ {serial} · com.fastpilot.demo installed")) && out.contains("tests/android/login-android.md"), "{out}");
    let listed = std::process::Command::new(&adb).args(["-s", &serial, "shell", "pm", "list", "packages", "com.fastpilot.demo"]).output().expect("adb");
    assert!(String::from_utf8_lossy(&listed.stdout).contains("com.fastpilot.demo"), "installed");

    let folder = tmp.join("fastpilot-demo");
    let client = mcp_from(&folder, &[("FASTPILOT_ANDROID_SERIAL", serial.as_str())]).await;
    // tests/login-android.md, its login part.
    let flow = "- session: { start: and1, platform: android }\n- launchApp: com.fastpilot.demo/.LoginActivity\n- inputText: { into: Email, text: a@b.com }\n- inputText: { into: Password, text: \"${DEMO_PASS}\" }\n- tapOn: Masuk\n- assertVisible: \"Selamat datang, Budi\"\n";
    let (ok, out) = call(&client, "run_flow", json!({ "flow": flow })).await;
    assert!(ok, "{out}");
    // Its last step (Design M2, F5): "Bayar" is refused by the built-in limits; the app stays put.
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- tapOn: Bayar\n", "session": "and1" })).await;
    assert!(!ok && out.contains("contains \"bayar\" (the built-in blocked list)") && !out.contains("block_actions"), "{out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: \"Selamat datang, Budi\"\n- assertNotVisible: \"Total Rp 150.000\"\n", "session": "and1" })).await;
    assert!(ok, "still on the dashboard: {out}");
    let md = std::fs::read_to_string(folder.join("tests/android/login-android.md")).expect("login-android.md");
    assert!(md.contains("must block that step") && !md.contains("Bayar sekarang"), "{md}");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(&tmp);
}
