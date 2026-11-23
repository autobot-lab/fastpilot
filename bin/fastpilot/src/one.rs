//! E1 (v0.7): `fastpilot` alone, typed in a terminal, opens the dashboard. Nothing else to learn.
//!
//! - **None running:** the dashboard starts and opens in the browser (as `fastpilot ui`).
//! - **The same version already running:** it is asked for a fresh one-time link (SIGUSR1; the
//!   link comes back in an owner-only file), and the browser opens it. No second dashboard on
//!   another port.
//! - **An older version running** (after `npm i -g` or `brew upgrade`): one question, "Restart
//!   it? (Y/n)". Yes stops it with SIGTERM and starts this one.
//!
//! Which process is "the dashboard" comes only from its pidfile (`~/.fastpilot/ui/pid`,
//! owner-only), and is believed only when that process belongs to this user, started at the time
//! the pidfile says (so a reused pid is not it), and is a program called `fastpilot`. Nothing is
//! ever looked up by name in the process list, and no other program is signalled (Security,
//! 4874: pidfile only, same uid only, SIGTERM; never SIGKILL).
//!
//! Not on a terminal (scripts, Claude Code's MCP, CI), `fastpilot` alone prints the help as
//! before.

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Pidfile {
    pid: u32,
    version: String,
    /// When the process started, as `ps -o lstart=` says (guards against a reused pid).
    started: String,
    port: u16,
}

fn pidfile() -> PathBuf {
    crate::browsers::dir().join("pid")
}

fn linkfile() -> PathBuf {
    crate::browsers::dir().join("link")
}

/// True when `fastpilot` alone should open the dashboard: both ends are a person's terminal.
/// (Tests, in debug builds only, say so with `FASTPILOT_ASSUME_TTY=1`.)
pub fn interactive() -> bool {
    if cfg!(debug_assertions) && std::env::var("FASTPILOT_ASSUME_TTY").is_ok_and(|v| v == "1") {
        return true;
    }
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn ps(field: &str, pid: u32) -> Option<String> {
    let o = std::process::Command::new("ps").args(["-o", &format!("{field}="), "-p", &pid.to_string()]).stderr(std::process::Stdio::null()).output().ok()?;
    let t = String::from_utf8_lossy(&o.stdout).trim().to_owned();
    (o.status.success() && !t.is_empty()).then_some(t)
}

fn my_uid() -> Option<String> {
    let o = std::process::Command::new("id").arg("-u").output().ok()?;
    Some(String::from_utf8_lossy(&o.stdout).trim().to_owned()).filter(|s| !s.is_empty())
}

/// Write an owner-only file in the owner-only `~/.fastpilot/ui` folder, atomically.
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)?;
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        o.open(&tmp)?.write_all(text.as_bytes())?;
    }
    std::fs::rename(&tmp, path)
}

/// The dashboard says where it is (called once it listens).
pub fn announce(port: u16) {
    let pid = std::process::id();
    let p = Pidfile { pid, version: VERSION.into(), started: ps("lstart", pid).unwrap_or_default(), port };
    let _ = write_private(&pidfile(), &serde_json::to_string(&p).unwrap_or_default());
}

/// The dashboard hands a fresh link to the `fastpilot` that asked for it (SIGUSR1).
pub fn hand_out(url: &str) {
    let _ = write_private(&linkfile(), url);
}

/// The running dashboard of this user, if the pidfile names one that is really it.
fn running() -> Option<Pidfile> {
    let p: Pidfile = serde_json::from_str(&std::fs::read_to_string(pidfile()).ok()?).ok()?;
    if p.pid == std::process::id() || p.pid <= 1 || p.started.is_empty() {
        return None;
    }
    let same_user = ps("uid", p.pid).zip(my_uid()).is_some_and(|(a, b)| a == b);
    let same_start = ps("lstart", p.pid).is_some_and(|s| s == p.started);
    let ours = ps("comm", p.pid).is_some_and(|c| std::path::Path::new(&c).file_name().is_some_and(|n| n == "fastpilot"));
    (same_user && same_start && ours).then_some(p)
}

fn signal(sig: &str, pid: u32) -> bool {
    std::process::Command::new("kill").args([&format!("-{sig}"), &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Ask the running dashboard for a fresh link; `None` when it does not answer within 5 s.
fn fresh_link(p: &Pidfile) -> Option<String> {
    let _ = std::fs::remove_file(linkfile());
    if !signal("USR1", p.pid) {
        return None;
    }
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(url) = std::fs::read_to_string(linkfile()) {
            let _ = std::fs::remove_file(linkfile());
            if url.starts_with(&format!("http://127.0.0.1:{}/?code=", p.port)) {
                return Some(url);
            }
        }
    }
    None
}

/// "(Y/n)": yes unless the person types n or no.
fn yes(question: &str) -> bool {
    print!("{question} (Y/n) ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    // End of input (Ctrl-D) is not a yes (Security, #198 L2).
    if matches!(std::io::stdin().lock().read_line(&mut line), Ok(0) | Err(_)) {
        return false;
    }
    !matches!(line.trim().to_ascii_lowercase().as_str(), "n" | "no")
}

/// Stop it with SIGTERM and wait up to 10 s for it to go.
fn stop(p: &Pidfile) -> bool {
    if !signal("TERM", p.pid) {
        return false;
    }
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(100));
        // Gone, or only a zombie its parent has not collected yet.
        if ps("lstart", p.pid).is_none_or(|s| s != p.started) || ps("stat", p.pid).is_some_and(|s| s.starts_with('Z')) {
            return true;
        }
    }
    false
}

/// `fastpilot` alone, in a terminal.
pub async fn run() -> Res<()> {
    if let Some(p) = running() {
        if p.version == VERSION {
            if let Some(url) = fresh_link(&p) {
                match crate::ui::open_browser(&url) {
                    Ok(()) => println!("fastpilot is already open: it is showing in your browser now. If not, use this link (once, within 10 minutes):\n{url}"),
                    Err(_) => println!("fastpilot is already open. Open this link (once, within 10 minutes):\n{url}"),
                }
                return Ok(());
            }
            if !yes("fastpilot is running but did not answer. Restart it?") {
                println!("Left it running at http://127.0.0.1:{}.", p.port);
                return Ok(());
            }
        } else if !yes(&format!("An older fastpilot is still running (dashboard, version {}). Restart it?", p.version)) {
            println!("Left the older one running at http://127.0.0.1:{}. Close it with Ctrl-C in its Terminal window, then type fastpilot again.", p.port);
            return Ok(());
        }
        if !stop(&p) {
            return Err(format!("the running fastpilot (pid {}) did not stop. Close it with Ctrl-C in its Terminal window, then type fastpilot again.", p.pid).into());
        }
        println!("Stopped the older fastpilot. Starting this one…");
    }
    // L2: what to do with this window, before anything else is printed.
    println!("fastpilot is opening in your browser. Keep this window open: closing it (or Ctrl-C) stops fastpilot.");
    crate::init::register_for_user_once();
    crate::ui::ui(&[]).await
}
