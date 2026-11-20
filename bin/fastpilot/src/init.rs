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
    machine().await;

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

    println!("\nReady. Next:\n  1. open claude in this folder\n  2. type: \"Use fastpilot to run {}\"\n  Want to try the demo first? fastpilot demo", example_path().display());
    Ok(())
}

/// One line per platform: ✓ ready, or ! with what to run later.
async fn machine() {
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
}

/// A ready platform in a few words.
fn ready(area: &str, checks: &[&Check]) -> String {
    let detail = |name: &str| checks.iter().find(|c| c.name == name).map(|c| c.detail.clone()).unwrap_or_default();
    match area {
        "web" => "Chrome starts headless".into(),
        "android" => format!("device: {}", clip(&detail("device"), 40)),
        "ios" => clip(&format!("{} · {}", detail("Xcode"), detail("simulator")), 50),
        _ => format!("Claude Code {}", detail("Claude Code").replace(" (Claude Code)", "")),
    }
}

/// What is missing, in a few words (the full story is in `fastpilot doctor`).
fn gap(c: &Check) -> String {
    match c.name.as_str() {
        "device" => "no device".into(),
        "adb" => "adb is missing or cannot be reached".into(),
        "agent" => "this build has no Android agent".into(),
        "Chrome" => "Chrome does not start yet".into(),
        "Xcode" => "Xcode is missing".into(),
        "simulator" => "no iPhone simulator".into(),
        "runner" => "test runner not built yet".into(),
        "Claude Code" => "Claude Code is not installed".into(),
        _ => clip(&c.detail, 45),
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
        return std::fs::read_to_string(".mcp.json")
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .is_some_and(|v| v["mcpServers"]["fastpilot"].is_object());
    }
    std::process::Command::new(claude)
        .args(["mcp", "get", "fastpilot"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn register(mode: Mode, scope: &str) {
    let claude = crate::run::claude_bin();
    let args = add_args(scope);
    let shown = format!("claude {}", args.join(" "));
    if registered(scope, &claude) {
        println!("  ✓ already registered{}", if scope == "project" { " (.mcp.json)" } else { "" });
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
}
