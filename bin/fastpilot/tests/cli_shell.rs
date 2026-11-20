//! The shell CLI end to end on real Chrome: one process per command, sessions kept by the
//! background daemon. Each test uses its own socket, so it never touches a user's daemon.
//! Skips without Chrome.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| Path::new(p).exists())
}

struct Shell {
    socket: PathBuf,
    dir: PathBuf,
    /// FASTPILOT_DAEMON_IDLE for the daemon this shell starts.
    idle: Option<&'static str>,
}

impl Shell {
    fn new(name: &str) -> Self {
        // Short path: Unix socket paths are limited to ~100 bytes.
        let dir = std::env::temp_dir().join(format!("fpsh-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        Self { socket: dir.join("d.sock"), dir, idle: None }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_fastpilot"));
        c.args(args)
            .current_dir(ROOT)
            .env("FASTPILOT_SOCKET", &self.socket)
            .env("FASTPILOT_DIR", self.dir.join("fp"))
            .env("FASTPILOT_VARS", "DEMO_PASS")
            .env("DEMO_PASS", "rahasia123")
            .env_remove("FASTPILOT_SESSION");
        if let Some(idle) = self.idle {
            c.env("FASTPILOT_DAEMON_IDLE", idle);
        }
        c
    }

    /// (success, stdout + stderr)
    fn run(&self, args: &[&str]) -> (bool, String) {
        let out = self.cmd(args).output().expect("run");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }

    fn ok(&self, args: &[&str]) -> String {
        let (ok, out) = self.run(args);
        assert!(ok, "fastpilot {args:?} failed:\n{out}");
        assert!(!out.contains("rahasia123"), "secret leaked by {args:?}:\n{out}");
        out
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.cmd(&["daemon", "stop"]).output();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The ref of the first line with `role "name"` in a snapshot.
fn ref_of(snapshot: &str, role_name: &str) -> String {
    let line = snapshot.lines().find(|l| l.contains(role_name)).unwrap_or_else(|| panic!("no {role_name} in\n{snapshot}"));
    format!("@{}", line.split_whitespace().next().unwrap_or(""))
}

#[test]
fn login_through_shell_commands() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let sh = Shell::new("login");
    let out = sh.ok(&["open", "examples/demo/login.html"]);
    assert!(out.contains("session default started (web)") && out.contains("button \"Masuk\""), "{out}");

    // -i: only what can be acted on; refs are the ones `click @eN` takes.
    let snap = sh.ok(&["snapshot", "-i"]);
    assert!(!snap.contains("heading") && snap.contains("textfield \"Email\""), "{snap}");
    let (email, pass, remember, login) =
        (ref_of(&snap, "textfield \"Email\""), ref_of(&snap, "textfield \"Password\""), ref_of(&snap, "checkbox"), ref_of(&snap, "button \"Masuk\""));

    sh.ok(&["fill", &email, "a@b.com"]);
    let out = sh.ok(&["fill", &pass, "${DEMO_PASS}"]);
    assert!(out.contains("ok 1/1"), "{out}");
    assert_eq!(sh.ok(&["get", "text", &email]).trim(), "a@b.com");
    // check/uncheck are idempotent.
    assert!(sh.ok(&["check", &remember]).contains("tapOn"));
    assert!(sh.ok(&["check", &remember]).contains("already checked"));
    assert!(sh.ok(&["uncheck", &remember]).contains("tapOn"));

    assert!(sh.ok(&["click", &login]).contains("tapOn e"));
    sh.ok(&["wait", "Budi", "--timeout", "5000"]);
    assert_eq!(sh.ok(&["is", "visible", "Budi"]).trim(), "true");
    assert_eq!(sh.ok(&["is", "visible", "Tidak ada"]).trim(), "false");
    assert_eq!(sh.ok(&["get", "title"]).trim(), "Dashboard");
    assert!(sh.ok(&["get", "url"]).trim().ends_with("examples/demo/login.html"));
    assert_eq!(sh.ok(&["eval", "1 + 2"]).trim(), "3");
    sh.ok(&["assert", "contains", "role=heading:Dashboard", "Dash"]);

    // A failed check exits non-zero with the reason and the screen.
    let (ok, out) = sh.run(&["assert", "visible", "Tidak ada", "--timeout", "300"]);
    assert!(!ok && out.contains("not_found") && out.contains("screen \"Dashboard\""), "{out}");

    // JSON snapshot: machine-readable entries, secure values never included.
    let json: serde_json::Value = serde_json::from_str(sh.ok(&["snapshot", "--json"]).trim()).expect("json snapshot");
    assert_eq!(json["session"], "default");
    assert!(json["entries"].as_array().is_some_and(|e| e.iter().any(|e| e["name"] == "Budi")), "{json}");

    // Screenshot to a file.
    let shot = sh.dir.join("shot.jpg");
    assert_eq!(sh.ok(&["screenshot", shot.to_str().unwrap_or("")]).lines().next(), shot.to_str());
    assert_eq!(&std::fs::read(&shot).expect("jpeg")[..2], &[0xFF, 0xD8]);

    // A flow from stdin, in the same session.
    let mut child = sh.cmd(&["flow", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("flow");
    std::io::Write::write_all(child.stdin.as_mut().expect("stdin"), b"- tapOn: Keluar\n- assertVisible: \"Masuk ke Akun\"\n").expect("write");
    let out = child.wait_with_output().expect("flow");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));

    // A second, isolated session next to the first.
    sh.ok(&["--session", "two", "open", "examples/demo/login.html"]);
    assert_eq!(sh.ok(&["sessions"]).lines().collect::<Vec<_>>(), ["default", "two (active)"]);
    sh.ok(&["--session", "two", "close"]);
    assert!(sh.ok(&["daemon", "status"]).contains("[\"default\"]"));
    assert!(sh.ok(&["errors"]).contains("no errors"));
    // Visual regression through the shell: stored, then matched.
    assert!(sh.ok(&["diff", "screenshot", "home"]).contains("baseline stored"));
    assert!(sh.ok(&["diff", "screenshot", "home", "--ignore", "Masuk ke Akun"]).contains("matches baseline"));
    // Saved state through the shell (file:// has no cookies; the commands still round-trip).
    assert!(sh.ok(&["state", "save", "demo"]).contains("demo.json"));
    assert!(sh.ok(&["state", "list"]).contains("demo"));
    assert!(sh.ok(&["state", "delete", "demo"]).contains("deleted"));
    sh.ok(&["close", "--all"]);
    let (running, _) = sh.run(&["daemon", "status"]);
    assert!(!running, "daemon still up after close --all");
}

#[test]
fn daemon_exits_when_idle_and_bad_input_is_explained() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let mut sh = Shell::new("idle");
    sh.idle = Some("2");
    let (ok, out) = sh.run(&["click"]);
    assert!(!ok && out.contains("missing a selector"), "{out}");
    let (ok, out) = sh.run(&["frobnicate"]);
    assert!(!ok, "{out}");
    let (ok, out) = sh.run(&["snapshot"]);
    assert!(!ok && out.contains("session"), "no session yet: {out}");
    sh.ok(&["open", "examples/demo/login.html"]);
    assert!(sh.run(&["daemon", "status"]).0);
    std::thread::sleep(std::time::Duration::from_secs(5));
    assert!(!sh.run(&["daemon", "status"]).0, "daemon did not exit after being idle");
    assert!(!sh.socket.exists(), "socket left behind");
}

/// QA #9: `get title` / `get url` ran an evalScript (refused with allow_eval_script = false or
/// block_actions set), and `flow` without an open session failed with "unknown session default".
#[test]
fn get_title_without_eval_and_flow_without_a_session() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let sh = Shell::new("get");
    let run = |args: &[&str]| {
        let out = sh.cmd(args).env("FASTPILOT_ALLOW_EVAL_SCRIPT", "false").env("FASTPILOT_BLOCK_ACTIONS", "Bayar").output().expect("run");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let (ok, out) = run(&["flow", "- tapOn: Masuk\n"]);
    assert!(!ok && out.contains("no session `default` is open") && out.contains("fastpilot open"), "{out}");
    // A flow that starts its own session runs without `open` first (was "unknown session `default`").
    let own = "- session: { start: own1, platform: web, target: \"examples/demo/login.html\" }\n- assertVisible: \"Masuk ke Akun\"\n";
    let (ok, out) = run(&["flow", own]);
    assert!(ok && !out.contains("unknown session"), "{out}");
    let (ok, out) = run(&["close", "--session", "own1"]);
    assert!(ok, "{out}");
    let (ok, out) = run(&["open", "examples/demo/login.html"]);
    assert!(ok, "{out}");
    let (ok, out) = run(&["get", "title"]);
    assert!(ok && out.trim() == "Login", "get title: {out}");
    let (ok, out) = run(&["get", "url"]);
    assert!(ok && out.trim().starts_with("file://") && out.trim().ends_with("examples/demo/login.html"), "get url: {out}");
    // evalScript itself is still refused.
    let (ok, out) = run(&["eval", "document.title"]);
    assert!(!ok && out.contains("blocked"), "{out}");
}
