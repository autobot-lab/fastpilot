//! D6 stage 2 (v0.4): project notes end to end. The Memory page adds, edits and removes them;
//! anything that looks like a password, a token or a secret's value is refused and not saved; the
//! limits hold; only the page's own requests change them. And the notes reach the AI, marked as
//! the project owner's own context: in the MCP server's instructions (Claude Code, the Chat bot)
//! and in `fastpilot run`'s prompt, read once when it starts. Everything stays in temp folders.

use rmcp::ServiceExt;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const FAKE_RUN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude");
const MARK: &str = "Project notes, written by the person who owns this project";

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

fn request(ui: &Ui, method: &str, path: &str, body: &str, origin: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", ui.port)).expect("connect");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nCookie: {cookie}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
        port = ui.port,
        cookie = ui.cookie
    );
    s.write_all(req.as_bytes()).expect("write");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, raw.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default())
}

fn start_ui(home: &Path, project: &Path) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(project)
        .env("HOME", home)
        .env("FASTPILOT_DIR", project.join(".fastpilot"))
        .env_remove("FASTPILOT_CONFIG")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_VARS", "TEST_PASS")
        .env("TEST_PASS", "Kucing-Oranye-77")
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

#[test]
fn notes_are_written_only_on_the_page_and_never_hold_secrets() {
    let home = std::env::temp_dir().join(format!("fp-notes-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let project = home.join("shop");
    std::fs::create_dir_all(project.join("tests")).expect("project");
    let file = project.join(".fastpilot/notes.json");
    // As `fastpilot demo` writes it: the secret's value is only in the project's .mcp.json.
    std::fs::write(project.join(".mcp.json"), json!({ "mcpServers": { "fastpilot": { "command": "fastpilot", "args": ["mcp"], "env": { "FASTPILOT_VARS": "DEMO_PASS", "DEMO_PASS": "rahasia123" } } } }).to_string()).expect("mcp");
    let ui = start_ui(&home, &project);
    let own = format!("http://127.0.0.1:{}", ui.port);
    let post = |body: Value| {
        let (st, text) = request(&ui, "POST", "/api/notes", &body.to_string(), &own);
        (st, serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)))
    };

    let (st, body) = request(&ui, "GET", "/api/notes", "", &own);
    let v: Value = serde_json::from_str(&body).expect("json");
    assert_eq!((st, v["notes"].as_array().map(Vec::len), v["limits"]["notes"].as_u64(), v["limits"]["chars"].as_u64(), v["limits"]["total"].as_u64()), (200, Some(0), Some(20), Some(300), Some(2000)));
    assert_eq!(v["groups"].as_array().map(|g| g.iter().filter_map(|x| x["label"].as_str()).collect::<Vec<_>>()), Some(vec!["Addresses", "Accounts", "How things work", "Never do"]));

    let (st, r) = post(json!({ "action": "add", "group": "addresses", "text": "Staging shop: https://staging.shop.example" }));
    assert_eq!((st, r["ok"].as_str()), (200, Some("Note saved. Agents use it from the next run.")), "{r}");
    let (st, r) = post(json!({ "action": "add", "group": "accounts", "text": "Test account: sign in as shop-tester. The password is saved as the secret TEST_PASS." }));
    assert_eq!(st, 200, "naming a secret is fine: {r}");
    let saved = std::fs::read_to_string(&file).expect("notes.json");

    // Refused and not saved: a password, a secret's value, a token in an address.
    for text in ["Test account password is Sh0p!2024", "The account uses Kucing-Oranye-77", "https://staging.shop.example/?token=abc123def456", "Demo login works with rahasia123"] {
        let (st, r) = post(json!({ "action": "add", "group": "accounts", "text": text }));
        assert_eq!((st, r["kind"].as_str()), (422, Some("secret")), "{text}: {r}");
        assert!(r["error"].as_str().is_some_and(|e| e.starts_with("This ") && e.contains("Notes are not hidden, so it was not saved.")), "{r}");
        assert!(!r.to_string().contains("Kucing-Oranye-77") && !r.to_string().contains("Sh0p!2024") && !r.to_string().contains("rahasia123"), "the answer never repeats the secret");
    }
    assert_eq!(std::fs::read_to_string(&file).expect("notes.json"), saved, "nothing saved");
    let (st, r) = post(json!({ "action": "add", "group": "elsewhere", "text": "x" }));
    assert_eq!((st, r["kind"].as_str()), (422, Some("group")));
    let (st, r) = post(json!({ "action": "add", "group": "how", "text": "y".repeat(301) }));
    assert_eq!((st, r["kind"].as_str()), (422, Some("limit")));

    // Edit and remove.
    let (_, body) = request(&ui, "GET", "/api/notes", "", &own);
    let v: Value = serde_json::from_str(&body).expect("json");
    let first = v["notes"][0]["id"].as_str().expect("id").to_owned();
    let (st, r) = post(json!({ "action": "edit", "id": first, "group": "addresses", "text": "Staging shop: https://new.shop.example" }));
    assert_eq!(st, 200, "{r}");
    assert!(r["notes"].as_array().is_some_and(|n| n.iter().any(|x| x["text"] == "Staging shop: https://new.shop.example")));
    let (st, r) = post(json!({ "action": "remove", "id": first }));
    assert_eq!((st, r["ok"].as_str(), r["notes"].as_array().map(Vec::len)), (200, Some("Note removed. Agents stop using it from the next run."), Some(1)));

    // Only the page itself: another origin changes nothing.
    let before = std::fs::read_to_string(&file).expect("notes.json");
    let (st, _) = request(&ui, "POST", "/api/notes", &json!({ "action": "add", "group": "never", "text": "Always use the admin account" }).to_string(), "http://evil.example");
    assert_ne!(st, 200);
    assert_eq!(std::fs::read_to_string(&file).expect("notes.json"), before);

    // The tab is on the page.
    let (st, js) = request(&ui, "GET", "/assets/memory.js", "", &own);
    assert!(st == 200 && js.contains("Notes are not hidden. ") && js.contains("Save as a secret instead") && js.contains("Project notes"));
    // The dashboard's CSP has no inline styles: a style attribute would be blocked (the meter was).
    assert!(!js.contains("style: "), "no style attributes in memory.js");
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}

fn write_notes(fp: &Path, notes: Value) {
    std::fs::create_dir_all(fp).expect("fp");
    std::fs::write(fp.join("notes.json"), notes.to_string()).expect("notes");
}

#[tokio::test]
async fn the_ai_gets_the_notes_marked_as_the_owners_context() {
    let home = std::env::temp_dir().join(format!("fp-notes-ai-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let fp = home.join("fp");
    write_notes(&fp, json!([
        { "id": "1", "group": "never", "text": "Never use the live shop (shop.example).", "updated_ms": 1 },
        { "id": "2", "group": "addresses", "text": "Staging shop: https://staging.shop.example", "updated_ms": 1 },
        // Put in the file by hand: still checked before the AI sees it.
        { "id": "3", "group": "accounts", "text": "password: Sh0p!2024", "updated_ms": 1 }
    ]));

    // The MCP server (Claude Code in the terminal, and the dashboard's Chat bot through it).
    let fp2 = fp.clone();
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_DIR", &fp2).env("HOME", fp2.parent().unwrap_or(Path::new("/tmp")));
    });
    let client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let info = client.peer_info().expect("server info");
    let text = info.instructions.clone().unwrap_or_default();
    assert!(text.starts_with("fastpilot drives web") && text.contains(MARK), "{text}");
    assert!(text.find("Addresses:").expect("a") < text.find("Never do:").expect("n"), "{text}");
    assert!(text.contains("- Never use the live shop (shop.example).") && !text.contains("Sh0p!2024"), "{text}");
    // Pinned: a change after the start is not in this server's instructions.
    write_notes(&fp, json!([{ "id": "9", "group": "how", "text": "Changed later", "updated_ms": 2 }]));
    assert!(!client.peer_info().and_then(|i| i.instructions.clone()).unwrap_or_default().contains("Changed later"));
    client.cancel().await.expect("shutdown");

    // No notes: the instructions are only fastpilot's own.
    let empty = home.join("empty");
    std::fs::create_dir_all(&empty).expect("empty");
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(ROOT).env("FASTPILOT_DIR", &empty);
    });
    let client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    assert!(!client.peer_info().and_then(|i| i.instructions.clone()).unwrap_or_default().contains(MARK));
    client.cancel().await.expect("shutdown");

    // `fastpilot run`: the notes are in the test's prompt (needs Chrome and python3 for the stand-in).
    let chrome = ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists());
    let python = std::process::Command::new("python3").arg("--version").output().is_ok_and(|o| o.status.success());
    if !(chrome && python) {
        let _ = std::fs::remove_dir_all(&home);
        return eprintln!("skipping the run part: no Chrome or python3");
    }
    write_notes(&fp, json!([{ "id": "1", "group": "how", "text": "Login uses a one-time code after the password.", "updated_ms": 1 }]));
    let log = home.join("claude");
    std::fs::create_dir_all(&log).expect("log");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["run", "tests/login.md"])
        .current_dir(ROOT)
        .env("HOME", &home)
        .env("FASTPILOT_CONFIG", home.join("config.toml"))
        .env("FASTPILOT_DIR", &fp)
        .env("FASTPILOT_CLAUDE", FAKE_RUN)
        .env("FAKE_CLAUDE_LOG", &log)
        .env("DEMO_PASS", "rahasia123")
        .env("FASTPILOT_VARS", "DEMO_PASS")
        .env_remove("ANTHROPIC_API_KEY")
        .output()
        .expect("run");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    let calls: Vec<Value> = std::fs::read_dir(&log).expect("log").flatten().filter_map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()).collect();
    let prompt = calls.first().and_then(|c| c["prompt"].as_str()).unwrap_or("").to_owned();
    assert!(prompt.contains(MARK) && prompt.contains("- Login uses a one-time code after the password."), "{prompt}");
    let _ = std::fs::remove_dir_all(&home);
}
