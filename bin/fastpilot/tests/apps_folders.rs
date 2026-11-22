//! E6c (v0.7): no project folder. The dashboard and Claude Code's fastpilot work in the current
//! app's folder (`~/fastpilot/apps/<id>/`), so tests, runs, memory and History belong to the app;
//! switching the app in the dashboard switches them. A project folder (`fastpilot init` ran there)
//! is one more app, "Repo", where its tests stay. Older project folders are searched for only on
//! the person's click, and only what the search found can be added. Temp HOME throughout.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

type Client = RunningService<RoleClient, ()>;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

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

fn raw(port: u16, method: &str, path: &str, headers: &[(&str, String)], body: &str) -> (u16, String) {
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
    (status, out)
}

fn start(home: &Path, cwd: &Path) -> Ui {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .args(["ui", "--no-open"])
        .current_dir(cwd)
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("ui");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout")).read_line(&mut line).expect("link");
    let rest = line.trim().strip_prefix("http://127.0.0.1:").expect("link").to_owned();
    let (port, code) = rest.split_once("/?code=").expect("code");
    let port: u16 = port.parse().expect("port");
    let (_, head) = raw(port, "GET", &format!("/?code={code}"), &[], "");
    let cookie = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().split(';').next().unwrap_or("").to_owned())).expect("cookie");
    Ui { child, port, cookie }
}

impl Ui {
    fn get(&self, path: &str) -> Value {
        let (st, out) = raw(self.port, "GET", path, &[("Cookie", self.cookie.clone())], "");
        assert_eq!(st, 200, "{path}: {out}");
        serde_json::from_str(out.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")).expect("json")
    }
    fn apps(&self, body: Value) -> Value {
        let h = [("Cookie", self.cookie.clone()), ("Origin", format!("http://127.0.0.1:{}", self.port)), ("Content-Type", "application/json".to_owned())];
        let (st, out) = raw(self.port, "POST", "/api/apps", &h, &body.to_string());
        assert_eq!(st, 200, "{body}: {out}");
        serde_json::from_str(out.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")).expect("json")
    }
    fn runs(&self) -> Vec<String> {
        self.get("/api/history")["runs"].as_array().map(|r| r.iter().filter_map(|x| x["title"].as_str().map(str::to_owned)).collect()).unwrap_or_default()
    }
}

fn run(dir: &Path, name: &str) {
    let d = dir.join(format!(".fastpilot/runs/20261120-090000-{name}"));
    std::fs::create_dir_all(&d).expect("run");
    std::fs::write(d.join("report.md"), format!("# {name} — pass\n")).expect("report");
}

#[test]
fn the_dashboard_works_in_the_current_apps_folder() {
    let home = std::env::temp_dir().join(format!("fp-apps-folders-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("elsewhere")).expect("home");
    let ui = start(&home, &home.join("elsewhere"));
    ui.apps(json!({ "action": "add", "name": "Shop", "targets": { "web": "https://shop.example" } }));
    ui.apps(json!({ "action": "add", "name": "Demo", "targets": { "android": "com.fastpilot.demo" } }));
    let apps = home.join("fastpilot/apps");
    run(&apps.join("shop"), "shop-check");
    run(&apps.join("demo"), "demo-check");

    // History is the current app's; switching the app switches it.
    assert_eq!(ui.runs(), ["shop-check"], "Shop is current");
    ui.apps(json!({ "action": "use", "id": "demo" }));
    assert_eq!(ui.runs(), ["demo-check"], "switched to Demo");
    // A restart starts in the current app's folder, wherever it is started from.
    drop(ui);
    let ui = start(&home, &home.join("elsewhere"));
    assert_eq!(ui.runs(), ["demo-check"]);
    assert!(!home.join("elsewhere/.fastpilot").exists(), "nothing is written in the folder it was started from");

    // A project folder (fastpilot init ran there) is one more app, "Repo", and its data stays there.
    drop(ui);
    let repo = home.join("code/shop-repo");
    run(&repo, "repo-check");
    let ui = start(&home, &repo);
    let v = ui.get("/api/apps");
    let r = v["apps"].as_array().and_then(|a| a.iter().find(|a| a["repo"] == "shop-repo")).cloned().unwrap_or_default();
    assert_eq!((r["name"].as_str(), v["current"].as_str()), (Some("shop-repo"), r["id"].as_str()), "the folder it was started in is the current app: {v}");
    assert_eq!(ui.runs(), ["repo-check"]);
    assert!(!v.to_string().contains(&home.to_string_lossy().to_string()), "no full path is shown: {v}");
    ui.apps(json!({ "action": "use", "id": "shop" }));
    assert_eq!(ui.runs(), ["shop-check"], "switching away from the repo works too");

    // Older project folders: only on request, only what was found; the home folder's own
    // ~/.fastpilot (settings) is not a project.
    let old = home.join("projects/old-app");
    std::fs::create_dir_all(old.join(".fastpilot")).expect("old");
    std::fs::create_dir_all(home.join(".fastpilot")).expect("settings");
    let f = ui.apps(json!({ "action": "find_older" }));
    let found: Vec<String> = f["found"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str().map(str::to_owned)).collect()).unwrap_or_default();
    assert_eq!(found, ["old-app"], "the listed repo and the home folder are not offered: {f}");
    let a = ui.apps(json!({ "action": "add_found", "paths": ["/etc", home.join("elsewhere").to_string_lossy()] }));
    assert_eq!(a["added"], json!([]), "a page cannot name a folder itself");
    let path = f["found"][0]["path"].as_str().expect("path").to_owned();
    let a = ui.apps(json!({ "action": "add_found", "paths": [path] }));
    assert_eq!(a["added"][0]["repo"], "old-app");
    assert!(old.join(".fastpilot").is_dir() && std::fs::read_dir(old.join(".fastpilot")).map(|d| d.count()).unwrap_or(1) == 0, "nothing was moved or written there");
    drop(ui);
    let _ = std::fs::remove_dir_all(&home);
}

fn chrome() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists())
}

async fn call(client: &Client, tool: &'static str, args: Value) -> (bool, String) {
    let args = args.as_object().cloned().unwrap_or_default();
    let res: CallToolResult = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

/// Claude Code opened in any folder that is not a project: fastpilot's MCP server works in the
/// current app's folder, so a run lands with that app's runs.
#[tokio::test(flavor = "multi_thread")]
async fn claude_code_in_any_folder_works_on_the_current_app() {
    if !chrome() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-apps-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let anywhere = home.join("Downloads");
    std::fs::create_dir_all(home.join("fastpilot/apps/shop")).expect("app");
    std::fs::create_dir_all(&anywhere).expect("anywhere");
    std::fs::write(home.join("fastpilot/apps.json"), json!({ "current": "shop", "apps": [{ "id": "shop", "name": "Shop", "targets": { "web": "https://shop.example/" } }] }).to_string()).expect("apps");
    let page = format!("file://{}/examples/demo/login.html", std::fs::canonicalize(ROOT).expect("root").display());
    let (h, a) = (home.clone(), anywhere.clone());
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        c.arg("mcp").current_dir(&a).env("HOME", &h).env_remove("FASTPILOT_DIR").env_remove("FASTPILOT_CONFIG").env("FASTPILOT_LIVE_DIR", h.join("live"));
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let (ok, out) = call(&client, "session", json!({ "action": "start", "id": "w", "target": page, "test_id": "where-check" })).await;
    assert!(ok, "{out}");
    let (ok, out) = call(&client, "run_flow", json!({ "flow": "- assertVisible: Email\n", "test_id": "where-check" })).await;
    assert!(ok, "{out}");
    let (ok, out) = call(&client, "report", json!({ "test_id": "where-check", "status": "pass" })).await;
    assert!(ok, "{out}");
    let runs = |d: PathBuf| std::fs::read_dir(d.join(".fastpilot/runs")).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect::<Vec<_>>()).unwrap_or_default();
    assert!(runs(home.join("fastpilot/apps/shop")).iter().any(|r| r.ends_with("where-check")), "the run is the app's: {:?}", runs(home.join("fastpilot/apps/shop")));
    assert!(runs(anywhere.clone()).is_empty(), "nothing in the folder Claude Code was opened in");
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(&home);
}
