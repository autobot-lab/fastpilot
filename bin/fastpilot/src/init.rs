//! `fastpilot init` (F4): the five manual steps of the README in one command, run in the folder of
//! the project to test (design todo-v032 #s1):
//!   1. check the machine (`doctor`, short; it fixes nothing)
//!   2. register fastpilot with Claude Code (`claude mcp add`, project scope; `--global`: user)
//!   3. install the fastpilot Skill (built into the binary) for Claude Code
//!   4. an example test case, `tests/example.md`, when there is none (an older `tests/contoh.md` counts)
//!   5. show the limits in force (information only; they are changed in `fastpilot ui`)
//!
//! Every step that changes something asks first ([y/N]); `--yes` agrees to all. Outside a terminal
//! (an agent, CI) without `--yes` it only prints the plan. Nothing is ever overwritten without
//! asking, and a second run finds everything done (✓). No secrets, no settings are written.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::demo::{mcp_command, via_npx};
use crate::doctor::{Check, Status};

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `tests/example.md`: a test case to start from (its target is to be changed).
pub const EXAMPLE: &str = "# Example test
id: example
platform: web
target: https://example.com
- Open the home page
- Check that the page title shows and there is no error message
";

const USAGE: &str = "usage: fastpilot init [--yes] [--global]
  set up this folder for testing with Claude Code: check this computer, register fastpilot,
  install the Skill, add tests/example.md, show the limits. Asks before each change.
  --yes     agree to every step (also outside a terminal)
  --global  register fastpilot and the Skill for every folder (Claude Code user scope)";

/// How each change is decided: ask in a terminal, yes with --yes, else plan only.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Ask,
    Yes,
    Plan,
}

fn ask(mode: Mode, question: &str) -> bool {
    match mode {
        Mode::Yes => {
            println!("  {question} [y/N] y");
            true
        }
        Mode::Plan => {
            println!("  {question} (you will be asked when you run this in a terminal)");
            false
        }
        Mode::Ask => {
            print!("  {question} [y/N] ");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            let _ = std::io::stdin().lock().read_line(&mut line);
            matches!(line.trim().to_lowercase().as_str(), "y" | "ya" | "yes")
        }
    }
}

pub async fn init(args: &[String]) -> Res<()> {
    let (mut yes, mut global) = (false, false);
    for a in args {
        match a.as_str() {
            "--yes" | "-y" => yes = true,
            "--global" => global = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => {
                eprintln!("fastpilot init: unknown option `{other}`\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let mode = if yes {
        Mode::Yes
    } else if std::io::stdin().is_terminal() {
        Mode::Ask
    } else {
        Mode::Plan
    };
    println!("fastpilot {VERSION} · setting up this folder for tests");
    if mode == Mode::Plan {
        println!("(not in a terminal: plan only, nothing is changed; run it in a terminal or add --yes)");
    }

    println!("\n1/5 Check this computer");
    let ios_setup = machine().await;
    // DV1: iPhone testing is a one-time setup; offered here when Xcode is found (only when asked
    // step by step: it takes about 5 minutes, so --yes and scripts do not start it).
    if ios_setup && mode == Mode::Ask && ask(mode, "Set up iPhone testing now? fastpilot builds a small test runner with your Xcode (about 5 minutes, once).") {
        let ok = std::env::current_exe().ok().and_then(|exe| std::process::Command::new(exe).args(["setup", "ios", "--yes"]).status().ok()).is_some_and(|s| s.success());
        if !ok {
            println!("  iPhone testing is not set up yet: run `fastpilot setup ios` later.");
        }
    }

    let scope = if global { "user" } else { "project" };
    println!("\n2/5 Register fastpilot with Claude Code ({})", if global { "every folder" } else { "this project" });
    register(mode, scope);

    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let skills_dir = if global { home.join(".claude/skills") } else { PathBuf::from(".claude/skills") };
    println!("\n3/5 Install the fastpilot Skills (teach Claude how to use fastpilot and how to test)");
    skills(mode, &skills_dir, global);

    println!("\n4/5 Example test");
    example(mode);

    println!("\n5/5 Safe limits");
    limits();

    // E6: this project folder is one more app in the dashboard ("Repo"); nothing in it moves.
    if !global && mode != Mode::Plan {
        let _ = crate::apps::add_repo(Path::new("."));
    }

    println!("\nReady. Next:\n  1. open claude in this folder\n  2. type: \"Use fastpilot to run {}\"\n  Want to try the demo first? fastpilot demo", example_path().display());
    // D3: only when the user is asked step by step (not --yes, not a script).
    crate::offer::dashboard(Path::new("."), mode == Mode::Ask, None);
    Ok(())
}

/// One line per platform: ✓ ready, or ! with what to run later. `true`: Xcode is here and only
/// the iPhone test runner is missing (the one-time setup can be offered).
async fn machine() -> bool {
    let list = crate::doctor::checks(&["web", "android", "ios", "run"]).await;
    for (area, label) in [("web", "web"), ("android", "android"), ("ios", "ios"), ("run", "claude")] {
        let mine: Vec<&Check> = list.iter().filter(|c| c.area == area).collect();
        if mine.is_empty() {
            continue;
        }
        let bad = mine.iter().find(|c| c.status != Status::Ok);
        match bad {
            None => println!("  ✓ {label:<9} {}", ready(area, &mine)),
            Some(c) => {
                let later = if area == "run" { "install Claude Code: https://claude.com/claude-code".to_owned() } else { format!("later: fastpilot setup {area}") };
                println!("  ! {label:<9} {} → {later}", gap(c));
            }
        }
    }
    ios_setup_needed(&list)
}

/// Xcode works and the runner is the only iPhone gap.
fn ios_setup_needed(list: &[Check]) -> bool {
    let ios: Vec<&Check> = list.iter().filter(|c| c.area == "ios").collect();
    ios.iter().any(|c| c.name == "Xcode" && c.status == Status::Ok) && ios.iter().any(|c| c.name == "runner" && c.status != Status::Ok) && ios.iter().all(|c| c.name == "runner" || c.status == Status::Ok)
}

/// A ready platform in a few words.
fn ready(area: &str, checks: &[&Check]) -> String {
    let detail = |name: &str| checks.iter().find(|c| c.name == name).map(|c| c.detail.clone()).unwrap_or_default();
    match area {
        "web" => "Chrome starts headless".into(),
        "android" => devices(&detail("device")),
        "ios" => clip(&format!("{} · {}", detail("Xcode"), detail("simulator")), 50),
        _ => format!("Claude Code {}", detail("Claude Code").replace(" (Claude Code)", "")),
    }
}

/// What is missing, in a few words (the full story is in `fastpilot doctor`).
fn gap(c: &Check) -> String {
    match c.name.as_str() {
        "device" => "no device".into(),
        "adb" => "adb is missing or cannot be reached".into(),
        "agent" => "this build of fastpilot has no Android helper".into(),
        "Chrome" => "Chrome does not start yet".into(),
        "Xcode" => "Xcode is missing".into(),
        "simulator" => "no iPhone simulator".into(),
        "runner" => crate::fix::IOS_SETUP.into(),
        "Claude Code" if c.detail.ends_with("too old") => "Claude Code is too old: run `claude update`".into(),
        "Claude Code" => "Claude Code is not installed".into(),
        _ => clip(&c.detail, 45),
    }
}

/// Every ready Android device, by kind (T1-T3 listed only the first; FastTest's final report: the
/// kind as on the Device page, no serials). doctor's line: "Android emulator: sdk gphone64 arm64, …".
fn devices(detail: &str) -> String {
    let serials: Vec<String> = detail.split(", ").filter_map(|d| d.rsplit_once(": ").and_then(|(kind, _)| kind.rsplit(": ").next()).map(|k| k.trim().to_owned())).filter(|k| !k.is_empty()).collect();
    match serials.as_slice() {
        [] => "device ready".to_owned(),
        [one] => format!("device: {one}"),
        many => format!("{} devices: {}", many.len(), many.join(", ")),
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_owned() }
}

/// The command line shown and run: `claude mcp add fastpilot --scope project -- fastpilot mcp`.
fn add_args(scope: &str) -> Vec<String> {
    let (cmd, args) = mcp_command();
    let mut v: Vec<String> = ["mcp", "add", "fastpilot", "--scope", scope, "--"].iter().map(|s| (*s).to_owned()).collect();
    v.push(cmd);
    v.extend(args);
    v
}

/// Already registered: project scope reads .mcp.json here; user scope asks Claude Code.
fn registered(scope: &str, claude: &str) -> bool {
    if scope == "project" {
        return project_registered(Path::new("."));
    }
    std::process::Command::new(claude)
        .args(["mcp", "get", "fastpilot"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// QA #5980 bug 3 (E6): the first `fastpilot` adds fastpilot to Claude Code for every folder (user
/// scope, as `init --global` does), once. Without Claude Code nothing is said or written, and a
/// later start tries again (as after a failed add). Never a project `.mcp.json`: it runs in the home folder.
pub fn register_for_user_once() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let done = home.join(".fastpilot").join("claude-registered");
    if done.exists() {
        return;
    }
    let claude = crate::run::claude_bin();
    let quiet = || {
        let mut c = std::process::Command::new(&claude);
        c.current_dir(&home).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        c
    };
    if !quiet().arg("--version").status().is_ok_and(|s| s.success()) {
        return;
    }
    if user_entry(&home).is_none() {
        let args = add_args("user");
        if quiet().args(&args).status().is_ok_and(|s| s.success()) {
            println!("Added fastpilot to Claude Code, for every folder.");
        } else {
            // Not marked: the next start tries again (Security #206 Info).
            println!("Could not add fastpilot to Claude Code. To add it yourself: claude {}", args.join(" "));
            return;
        }
    }
    let _ = std::fs::create_dir_all(home.join(".fastpilot"));
    let _ = std::fs::write(&done, "");
}

/// Claude Code's own "fastpilot" entry for every folder (user scope: `~/.claude.json`'s
/// top-level `mcpServers`, not a project's).
pub fn user_entry(home: &Path) -> Option<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).ok()?).ok()?;
    Some(v["mcpServers"]["fastpilot"].clone()).filter(serde_json::Value::is_object)
}

/// This folder's `.mcp.json` starts fastpilot (a project registration).
pub fn project_registered(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(".mcp.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| runs_fastpilot(&v["mcpServers"]["fastpilot"]))
}

/// A `.mcp.json` server entry that really starts fastpilot's MCP server (`fastpilot mcp`, or
/// `npx -y fastpilot mcp`), not just any entry named "fastpilot" (T1-T3).
fn runs_fastpilot(entry: &serde_json::Value) -> bool {
    let command = entry["command"].as_str().unwrap_or("");
    let args: Vec<&str> = entry["args"].as_array().map(|a| a.iter().filter_map(serde_json::Value::as_str).collect()).unwrap_or_default();
    let name = Path::new(command).file_name().and_then(|n| n.to_str()).unwrap_or("");
    let is_fastpilot = |s: &str| s == "fastpilot" || s.starts_with("fastpilot@");
    let mcp_last = args.last() == Some(&"mcp");
    match name {
        n if is_fastpilot(n) => mcp_last,
        "npx" | "npx.cmd" => mcp_last && args.iter().any(|a| is_fastpilot(a)),
        _ => false,
    }
}

fn register(mode: Mode, scope: &str) {
    let claude = crate::run::claude_bin();
    let args = add_args(scope);
    let shown = format!("claude {}", args.join(" "));
    if registered(scope, &claude) {
        println!("  ✓ already registered{}", if scope == "project" { " (.mcp.json)" } else { "" });
        return;
    }
    // A "fastpilot" entry that runs something else: say so; never replace it without the user.
    let other = (scope == "project")
        .then(|| std::fs::read_to_string(".mcp.json").ok())
        .flatten()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .filter(|v| v["mcpServers"]["fastpilot"].is_object());
    if let Some(v) = other {
        let e = &v["mcpServers"]["fastpilot"];
        println!(
            "  ! .mcp.json has a \"fastpilot\" entry that does not start fastpilot (command: {} {}).\n    Fix or remove that entry, then run fastpilot init again.",
            e["command"].as_str().unwrap_or("?"),
            e["args"].as_array().map(|a| a.iter().filter_map(serde_json::Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default()
        );
        return;
    }
    let installed = std::process::Command::new(&claude).arg("--version").stdin(std::process::Stdio::null()).output().is_ok_and(|o| o.status.success());
    if !installed {
        println!("  ! Claude Code not found. After installing it, run:\n    {shown}");
        return;
    }
    println!("  {shown}");
    if !ask(mode, "Run it?") {
        return;
    }
    match std::process::Command::new(&claude).args(&args).stdin(std::process::Stdio::null()).output() {
        Ok(o) if o.status.success() => println!("  ✓ registered{}", if scope == "project" { " (.mcp.json)" } else { "" }),
        Ok(o) => println!(
            "  ✗ failed: {}\n    run it yourself: {shown}",
            clip(format!("{}{}", String::from_utf8_lossy(&o.stderr), String::from_utf8_lossy(&o.stdout)).trim(), 200)
        ),
        Err(e) => println!("  ✗ failed: {e}\n    run it yourself: {shown}"),
    }
}

/// Every skill of the library (`fastpilot` plus the testing skills) in `dir/<name>/SKILL.md`: one
/// question for the missing ones, one for those that differ (never replaced without asking).
fn skills(mode: Mode, dir: &Path, global: bool) {
    let shown = if global { format!("~/{}", dir.strip_prefix(std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()).unwrap_or(dir).display()) } else { dir.display().to_string() };
    let all = fastpilot_mcp::skills::SKILLS;
    let (mut missing, mut differ) = (Vec::new(), Vec::new());
    for (name, text) in all {
        match std::fs::read_to_string(dir.join(name).join("SKILL.md")) {
            Ok(t) if t == *text => {}
            Ok(_) => differ.push((*name, *text)),
            Err(_) => missing.push((*name, *text)),
        }
    }
    if missing.is_empty() && differ.is_empty() {
        println!("  ✓ already installed ({} skills, version {VERSION}) · {shown}", all.len());
        return;
    }
    let names = |l: &[(&str, &str)]| l.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ");
    if !differ.is_empty() {
        println!("  → {shown}: {} skill(s) differ from fastpilot {VERSION}: {}", differ.len(), names(&differ));
        if ask(mode, &format!("Update them to {VERSION}?")) {
            write_skills(dir, &differ, "updated");
        }
    }
    if !missing.is_empty() {
        println!("  → {shown}: {} skills ({})", missing.len(), names(&missing));
        if ask(mode, "Install them?") {
            write_skills(dir, &missing, &format!("installed (version {VERSION}, built into fastpilot)"));
        }
    }
}

fn write_skills(dir: &Path, list: &[(&str, &str)], done: &str) {
    let mut failed = false;
    for (name, text) in list {
        if let Err(e) = write_file(&dir.join(name).join("SKILL.md"), text) {
            println!("  ✗ {e}");
            failed = true;
        }
    }
    if !failed {
        println!("  ✓ {done}: {} skills", list.len());
    }
}

/// The example test: `tests/example.md`, or the `tests/contoh.md` an older fastpilot made (then
/// no second one is added).
fn example_path() -> &'static Path {
    if Path::new("tests/contoh.md").exists() { Path::new("tests/contoh.md") } else { Path::new("tests/example.md") }
}

fn example(mode: Mode) {
    let path = example_path();
    if path.exists() {
        println!("  ✓ {} already exists (not changed)", path.display());
        return;
    }
    println!("  → {}", path.display());
    if ask(mode, "Create it?") {
        write(path, EXAMPLE, "created (change its target to your app's URL)");
    }
}

/// Write without following a symlink: in the project every folder on the way is checked (a
/// checkout cannot point `tests/` at another place); with `--global` only the file itself, so a
/// symlinked ~/.claude of the user's dotfiles keeps working.
fn write(path: &Path, text: &str, done: &str) {
    match write_file(path, text) {
        Ok(()) => println!("  ✓ {done}"),
        Err(e) => println!("  ✗ {e}"),
    }
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if path.is_absolute() {
        let parent = path.parent().unwrap_or(Path::new("/"));
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))
            .and_then(|()| crate::demo::write_safe(parent, Path::new(path.file_name().unwrap_or_default()), text.as_bytes(), false))
    } else {
        crate::demo::write_safe(Path::new("."), path, text.as_bytes(), false)
    }
}

/// The limits as fastpilot applies them now (settings, environment, built-in defaults).
fn limits() {
    let from = |key: &str| match fastpilot_core::config::current(key) {
        fastpilot_core::config::Source::Env(_) => "from the environment",
        fastpilot_core::config::Source::File(_) => "from your settings",
        fastpilot_core::config::Source::Default => "default",
    };
    let words = |w: Vec<String>| {
        let n = w.len();
        let head: Vec<String> = w.into_iter().take(4).collect();
        if n > 4 { format!("{}, … ({n} words)", head.join(", ")) } else { head.join(", ") }
    };
    let blocked = fastpilot_core::policy::blocked_words();
    if blocked.is_empty() {
        println!("  ! no button is always blocked (e.g. Pay, Checkout)");
    } else {
        println!("  ✓ always blocked: {} ({})", words(blocked), from("block_actions"));
    }
    let confirm = fastpilot_core::policy::risky_words();
    if !confirm.is_empty() {
        println!("  ✓ needs confirmation: {} ({})", words(confirm), from("confirm_actions"));
    }
    println!("    change them in: fastpilot ui → Settings › Limits & permissions");
    if via_npx() {
        println!("    (with npx: npx -y fastpilot ui)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_is_the_repository_skill_and_the_command_is_claudes() {
        let repo = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../skills/fastpilot/SKILL.md")).expect("skill");
        assert_eq!(fastpilot_mcp::skills::get("fastpilot"), Some(repo.as_str()));
        let args = add_args("project");
        assert_eq!(args[..6], ["mcp", "add", "fastpilot", "--scope", "project", "--"]);
        assert_eq!(args.last().map(String::as_str), Some("mcp"));
        assert!(EXAMPLE.contains("platform: web") && EXAMPLE.contains("target: https://example.com") && EXAMPLE.contains("id: example"));
    }

    /// T1-T3: any entry named "fastpilot" counted as registered.
    #[test]
    fn only_an_entry_that_starts_fastpilot_counts_as_registered() {
        use serde_json::json;
        for ok in [
            json!({ "command": "fastpilot", "args": ["mcp"] }),
            json!({ "command": "/usr/local/bin/fastpilot", "args": ["mcp"] }),
            json!({ "command": "npx", "args": ["-y", "fastpilot", "mcp"] }),
            json!({ "command": "npx", "args": ["-y", "fastpilot@0.4.0", "mcp"] }),
        ] {
            assert!(runs_fastpilot(&ok), "{ok}");
        }
        // What init and demo write themselves always counts.
        let (command, args) = crate::demo::mcp_command();
        assert!(runs_fastpilot(&json!({ "command": command, "args": args })));
        for bad in [
            json!({ "command": "node", "args": ["evil.js"] }),
            json!({ "command": "fastpilot", "args": ["ui"] }),
            json!({ "command": "npx", "args": ["-y", "other", "mcp"] }),
            json!({ "command": "sh", "args": ["-c", "fastpilot mcp"] }),
            json!({}),
        ] {
            assert!(!runs_fastpilot(&bad), "{bad}");
        }
    }


    /// DV1: the iPhone runner is a one-time setup, offered only when Xcode works and the runner
    /// is the only thing missing; its line says so, not "not built".
    #[test]
    fn iphone_testing_is_a_one_time_setup() {
        use crate::doctor::check;
        let ok = |n: &str| check("ios", n, Status::Ok, "x");
        let runner = check("ios", "runner", Status::Warn, crate::fix::IOS_SETUP);
        assert!(ios_setup_needed(&[ok("Xcode"), ok("simulator"), runner.clone()]));
        assert!(!ios_setup_needed(&[ok("Xcode"), ok("simulator"), ok("runner")]), "already built");
        assert!(!ios_setup_needed(&[check("ios", "Xcode", Status::Fail, "Xcode not found")]), "no Xcode");
        assert!(!ios_setup_needed(&[ok("Xcode"), check("ios", "simulator", Status::Fail, "none"), runner.clone()]), "no simulator yet");
        assert_eq!(gap(&runner), "iPhone testing needs a one-time setup (about 5 minutes)");
    }

    /// T1-T3: step 1/5 listed only the first device.
    #[test]
    fn every_ready_android_device_is_listed() {
        assert_eq!(devices("Android emulator: Android SDK built for arm64, Android emulator: sdk gphone64 arm64"), "2 devices: Android emulator, Android emulator");
        // QA B9: the kind is named, a phone included; no serials (FastTest, final report).
        assert_eq!(devices("Android phone (physical): SM-A515F"), "device: Android phone (physical)");
        assert_eq!(devices("on the adb server 10.0.0.2:5037: Android emulator: sdk gphone64 arm64"), "device: Android emulator");
    }

}
