//! E1 (v0.7): `fastpilot` alone, typed in a terminal, opens the dashboard. Typed again while it
//! runs, the same dashboard gets a fresh link (no second one on another port). After an update,
//! an older dashboard is restarted only after "yes", and only the process the owner-only pidfile
//! names, checked by user, start time and program name: a pidfile naming any other program is
//! never signalled. Without a terminal (scripts, MCP) `fastpilot` alone still prints the help.
//! A stand-in `open`/`xdg-open` records the link instead of opening a browser.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A stand-in Claude Code: `--version`, and `mcp add` writing `$HOME/.claude.json` (logged).
const FAKE_CLAUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude");

fn setup() -> (PathBuf, PathBuf) {
    setup_in("one")
}

fn setup_in(name: &str) -> (PathBuf, PathBuf) {
    let home = std::env::temp_dir().join(format!("fp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    // The browser: write down which page it was asked to open.
    for name in ["open", "xdg-open"] {
        let f = bin.join(name);
        std::fs::write(&f, format!("#!/bin/sh\necho \"$1\" >> '{}'\n", home.join("opened.log").display())).expect("opener");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    (home, bin)
}

fn fastpilot(home: &Path, bin: &Path, tty: bool) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    c.current_dir(home)
        .env("HOME", home)
        .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()))
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        .env("FASTPILOT_CLAUDE", FAKE_CLAUDE)
        .env("FAKE_CLAUDE_LOG", home.join("claude-log"))
        .env_remove("FASTPILOT_CONFIG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if tty {
        c.env("FASTPILOT_ASSUME_TTY", "1");
    }
    c
}

/// The link of the n-th page the stand-in browser was asked to open.
fn opened(home: &Path, n: usize) -> String {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let files: Vec<String> = std::fs::read_to_string(home.join("opened.log")).unwrap_or_default().lines().map(str::to_owned).collect();
        if let Some(f) = files.get(n) {
            let html = std::fs::read_to_string(f).expect("page");
            return html.split("url=").nth(1).and_then(|r| r.split('"').next()).expect("link").to_owned();
        }
        assert!(Instant::now() < deadline, "the browser was not asked to open a page #{n}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// GET the link: 303 with a session cookie means it let the browser in.
fn lets_in(url: &str) -> bool {
    let rest = url.strip_prefix("http://127.0.0.1:").expect("local");
    let (port, path) = rest.split_once('/').expect("path");
    let mut s = TcpStream::connect(("127.0.0.1", port.parse::<u16>().expect("port"))).expect("connect");
    s.write_all(format!("GET /{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes()).expect("write");
    let mut out = String::new();
    s.read_to_string(&mut out).expect("read");
    out.starts_with("HTTP/1.1 303") && out.to_ascii_lowercase().contains("set-cookie")
}

fn pidfile(home: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(home.join(".fastpilot/ui/pid")).expect("pidfile")).expect("json")
}

fn alive(pid: u32) -> bool {
    Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Start `fastpilot` alone and wait until it says it is up (its first output line).
fn start(home: &Path, bin: &Path, answer: &str) -> (Kill, String) {
    start_with(home, bin, answer, &[])
}

fn start_with(home: &Path, bin: &Path, answer: &str, env: &[(&str, &str)]) -> (Kill, String) {
    let mut c = fastpilot(home, bin, true).envs(env.iter().copied()).spawn().expect("fastpilot");
    let mut stdin = c.stdin.take().expect("stdin");
    stdin.write_all(answer.as_bytes()).expect("answer");
    drop(stdin);
    let mut out = BufReader::new(c.stdout.take().expect("stdout"));
    let mut said = String::new();
    // Questions are printed without a newline; read until the dashboard's link line.
    for _ in 0..20 {
        let mut line = String::new();
        if out.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        said.push_str(&line);
        if line.contains("/?code=") {
            break;
        }
    }
    std::thread::spawn(move || {
        let mut rest = String::new();
        let _ = out.read_to_string(&mut rest);
    });
    (Kill(c), said)
}

#[test]
fn fastpilot_alone_opens_the_dashboard_and_restarts_an_old_one_only_after_yes() {
    let (home, bin) = setup();

    // Not a terminal: the help, as before (MCP and scripts see no change).
    let o = fastpilot(&home, &bin, false).output().expect("run");
    assert!(String::from_utf8_lossy(&o.stderr).contains("usage") && o.stdout.is_empty(), "{o:?}");
    assert!(!home.join(".fastpilot/ui/pid").exists(), "no dashboard started");

    // In a terminal: the dashboard starts and opens in the browser.
    let (first, said) = start(&home, &bin, "");
    assert!(said.contains("opened in your browser"), "{said}");
    let link = opened(&home, 0);
    assert!(lets_in(&link), "{link}");
    let p = pidfile(&home);
    let pid = p["pid"].as_u64().expect("pid") as u32;
    assert_eq!(pid, first.0.id());
    assert_eq!(p["version"], env!("CARGO_PKG_VERSION"));
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(home.join(".fastpilot/ui/pid")).expect("pid").permissions().mode() & 0o777, 0o600);
    }

    // Typed again: the same dashboard gives a fresh link; nothing new starts.
    let o = fastpilot(&home, &bin, true).output().expect("again");
    let said = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success() && said.contains("already open"), "{said} {}", String::from_utf8_lossy(&o.stderr));
    let fresh = opened(&home, 1);
    assert!(fresh.starts_with(&format!("http://127.0.0.1:{}/?code=", p["port"])) && fresh != link, "{fresh}");
    assert!(lets_in(&fresh), "the fresh link lets the browser in");
    assert!(alive(pid) && pidfile(&home)["pid"] == p["pid"], "the same dashboard");

    // After an update: the running one is older. "n" leaves it alone.
    let mut older = p.clone();
    older["version"] = "0.0.1".into();
    std::fs::write(home.join(".fastpilot/ui/pid"), older.to_string()).expect("older");
    let mut c = fastpilot(&home, &bin, true).spawn().expect("run");
    c.stdin.take().expect("stdin").write_all(b"n\n").expect("n");
    let o = c.wait_with_output().expect("wait");
    let said = String::from_utf8_lossy(&o.stdout);
    assert!(said.contains("An older fastpilot is still running (dashboard, version 0.0.1). Restart it? (Y/n)") && said.contains("Left the older one running"), "{said}");
    assert!(alive(pid), "not stopped without a yes");

    // Enter (yes): it is stopped (SIGTERM) and this one starts. (Collect it as its shell would.)
    let mut first = first;
    let mut child = std::mem::replace(&mut first.0, Command::new("true").spawn().expect("true"));
    let reaper = std::thread::spawn(move || child.wait());
    let (second, said) = start(&home, &bin, "\n");
    assert!(said.contains("Stopped the older fastpilot") && said.contains("/?code="), "{said}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = reaper.join();
    assert_eq!(pidfile(&home)["pid"].as_u64(), Some(u64::from(second.0.id())), "the new one is the dashboard now");
    drop(second);

    // A pidfile that names another program (here `sleep`, same user): never signalled.
    let mut other = Command::new("sleep").arg("30").spawn().expect("sleep");
    let lstart = String::from_utf8_lossy(&Command::new("ps").args(["-o", "lstart=", "-p", &other.id().to_string()]).output().expect("ps").stdout).trim().to_owned();
    std::fs::write(home.join(".fastpilot/ui/pid"), serde_json::json!({ "pid": other.id(), "version": "0.0.1", "started": lstart, "port": 1 }).to_string()).expect("other");
    let (third, said) = start(&home, &bin, "y\n");
    assert!(!said.contains("older fastpilot") && said.contains("/?code="), "{said}");
    assert!(alive(other.id()), "another program is left alone");
    drop(third);
    let _ = other.kill();
    let _ = other.wait();
    let _ = std::fs::remove_dir_all(&home);
}

/// The `.mcp.json` files under `dir`.
fn mcp_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() && !p.is_symlink() {
            out.extend(mcp_files(&p));
        } else if p.file_name().is_some_and(|n| n == ".mcp.json") {
            out.push(p);
        }
    }
    out
}

/// The `claude mcp …` calls the stand-in saw.
fn mcp_calls(home: &Path) -> Vec<serde_json::Value> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(home.join("claude-log")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with("-mcp.json")).collect();
    files.sort();
    files.iter().map(|f| serde_json::from_str(&std::fs::read_to_string(f).expect("log")).expect("json")).collect()
}

/// QA #5980 bug 3 (E6): the first `fastpilot` adds fastpilot to Claude Code for every folder (user
/// scope, as `init --global`), once, and never writes a project `.mcp.json`. Without Claude Code it
/// says and writes nothing. `doctor` names a double registration (user scope plus a repo copy).
#[test]
fn the_first_fastpilot_adds_it_to_claude_code_for_every_folder_once() {
    let (home, bin) = setup_in("one-reg");

    // No Claude Code: nothing said, nothing written; a later start tries again.
    let (run, said) = start_with(&home, &bin, "", &[("FASTPILOT_CLAUDE", "/nonexistent/claude")]);
    opened(&home, 0);
    drop(run);
    assert!(!said.contains("Claude Code"), "{said}");
    assert!(!home.join(".claude.json").exists() && !home.join(".fastpilot/claude-registered").exists());

    // A failed add is not marked done: the next start tries again.
    let (run, said) = start_with(&home, &bin, "", &[("FAKE_CLAUDE_MCP_FAIL", "1")]);
    opened(&home, 1);
    drop(run);
    assert!(said.contains("Could not add fastpilot to Claude Code") && !home.join(".fastpilot/claude-registered").exists(), "{said}");
    std::fs::remove_dir_all(home.join("claude-log")).expect("log");

    // With Claude Code: added at user scope, from the home folder.
    let (run, said) = start(&home, &bin, "");
    opened(&home, 2);
    drop(run);
    assert!(said.contains("Added fastpilot to Claude Code, for every folder."), "{said}");
    let calls = mcp_calls(&home);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0]["argv"], serde_json::json!(["mcp", "add", "fastpilot", "--scope", "user", "--", "fastpilot", "mcp"]));
    assert_eq!(std::fs::canonicalize(calls[0]["cwd"].as_str().expect("cwd")).expect("cwd"), std::fs::canonicalize(&home).expect("home"));
    let user: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).expect("user scope")).expect("json");
    assert_eq!((user["mcpServers"]["fastpilot"]["command"].as_str(), user["mcpServers"]["fastpilot"]["args"].clone()), (Some("fastpilot"), serde_json::json!(["mcp"])));
    assert!(mcp_files(&home).is_empty(), "no project .mcp.json: {:?}", mcp_files(&home));

    // Once: a person who removed it later is not overruled on the next start.
    std::fs::remove_file(home.join(".claude.json")).expect("remove");
    let (run, said) = start(&home, &bin, "");
    opened(&home, 3);
    drop(run);
    assert!(!said.contains("Claude Code") && mcp_calls(&home).len() == 1, "{said}");

    // doctor: registered for every folder and again in a repo's .mcp.json.
    std::fs::write(home.join(".claude.json"), user.to_string()).expect("user");
    let repo = home.join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    std::fs::write(repo.join(".mcp.json"), r#"{"mcpServers":{"fastpilot":{"type":"stdio","command":"fastpilot","args":["mcp"]}}}"#).expect("repo copy");
    let o = fastpilot(&home, &bin, false).args(["doctor", "run"]).current_dir(&repo).output().expect("doctor");
    let out = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
    assert!(out.contains("registered twice") && out.contains("claude mcp remove fastpilot --scope project"), "{out}");
    let o = fastpilot(&home, &bin, false).args(["doctor", "run"]).output().expect("doctor");
    assert!(String::from_utf8_lossy(&o.stdout).contains("for every folder"), "{o:?}");
    let _ = std::fs::remove_dir_all(&home);
}
