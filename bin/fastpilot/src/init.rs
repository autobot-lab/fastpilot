//! `fastpilot init` (F4): the five manual steps of the README in one command, run in the folder of
//! the project to test (design todo-v032 #s1):
//!   1. check the machine (`doctor`, short; it fixes nothing)
//!   2. register fastpilot with Claude Code (`claude mcp add`, project scope; `--global`: user)
//!   3. install the fastpilot Skill (built into the binary) for Claude Code
//!   4. an example test case, `tests/contoh.md`, when there is none
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

/// The fastpilot Skill, as in `skills/fastpilot/SKILL.md`.
pub const SKILL: &str = include_str!("../../../skills/fastpilot/SKILL.md");
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `tests/contoh.md`: a test case to start from (its target is to be changed).
pub const EXAMPLE: &str = "# Contoh test
id: contoh
platform: web
target: https://contoh.com
- Buka halaman utama
- Pastikan judul halaman tampil dan tidak ada pesan error
";

const USAGE: &str = "pemakaian: fastpilot init [--yes] [--global]
  siapkan folder ini untuk test dengan Claude Code: cek mesin, daftarkan fastpilot,
  pasang Skill, buat tests/contoh.md, tampilkan Batasan. Ditanya sebelum tiap perubahan.
  --yes     setujui semua langkah (juga di luar terminal)
  --global  daftarkan fastpilot dan Skill untuk semua folder (scope user Claude Code)";

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
            println!("  {question} (akan ditanya saat dijalankan di terminal)");
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
                eprintln!("fastpilot init: opsi `{other}` tidak dikenal\n\n{USAGE}");
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
    println!("fastpilot {VERSION} · menyiapkan folder ini untuk test");
    if mode == Mode::Plan {
        println!("(bukan di terminal: hanya rencana, tidak ada yang diubah; jalankan di terminal atau tambah --yes)");
    }

    println!("\n1/5 Cek mesin");
    machine().await;

    let scope = if global { "user" } else { "project" };
    println!("\n2/5 Daftarkan fastpilot ke Claude Code ({})", if global { "semua folder" } else { "proyek ini" });
    register(mode, scope);

    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let skill_dir = if global { home.join(".claude/skills/fastpilot") } else { PathBuf::from(".claude/skills/fastpilot") };
    println!("\n3/5 Pasang Skill fastpilot (mengajari Claude cara memakai fastpilot)");
    skill(mode, &skill_dir.join("SKILL.md"), global);

    println!("\n4/5 Contoh test");
    example(mode);

    println!("\n5/5 Batasan aman");
    limits();

    println!("\nSiap. Langkah berikutnya:\n  1. buka claude di folder ini\n  2. ketik: \"Pakai fastpilot, jalankan tests/contoh.md\"\n  Mau coba demo dulu? fastpilot demo");
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
                let later = if area == "run" { "pasang Claude Code: https://claude.com/claude-code".to_owned() } else { format!("nanti: fastpilot setup {area}") };
                println!("  ! {label:<9} {} → {later}", gap(c));
            }
        }
    }
}

/// A ready platform in a few words.
fn ready(area: &str, checks: &[&Check]) -> String {
    let detail = |name: &str| checks.iter().find(|c| c.name == name).map(|c| c.detail.clone()).unwrap_or_default();
    match area {
        "web" => "Chrome bisa dinyalakan headless".into(),
        "android" => format!("device: {}", clip(&detail("device"), 40)),
        "ios" => clip(&format!("{} · {}", detail("Xcode"), crate::ui::in_indonesian(&detail("simulator"))), 50),
        _ => format!("Claude Code {}", detail("Claude Code").replace(" (Claude Code)", "")),
    }
}

/// What is missing, in a few words (the full story is in `fastpilot doctor`).
fn gap(c: &Check) -> String {
    match c.name.as_str() {
        "device" => "tidak ada device".into(),
        "adb" => "adb belum ada atau tidak bisa dihubungi".into(),
        "agent" => "agent Android tidak ikut di build ini".into(),
        "Chrome" => "Chrome belum bisa dinyalakan".into(),
        "Xcode" => "Xcode belum ada".into(),
        "simulator" => "belum ada simulator iPhone".into(),
        "runner" => "runner belum ada".into(),
        "Claude Code" => "Claude Code belum terpasang".into(),
        _ => clip(&crate::ui::in_indonesian(&c.detail), 45),
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
        println!("  ✓ sudah terdaftar{}", if scope == "project" { " (.mcp.json)" } else { "" });
        return;
    }
    let installed = std::process::Command::new(&claude).arg("--version").stdin(std::process::Stdio::null()).output().is_ok_and(|o| o.status.success());
    if !installed {
        println!("  ! Claude Code tidak ditemukan. Setelah dipasang, jalankan:\n    {shown}");
        return;
    }
    println!("  {shown}");
    if !ask(mode, "Jalankan?") {
        return;
    }
    match std::process::Command::new(&claude).args(&args).stdin(std::process::Stdio::null()).output() {
        Ok(o) if o.status.success() => println!("  ✓ terdaftar{}", if scope == "project" { " (.mcp.json)" } else { "" }),
        Ok(o) => println!(
            "  ✗ gagal: {}\n    jalankan sendiri: {shown}",
            clip(format!("{}{}", String::from_utf8_lossy(&o.stderr), String::from_utf8_lossy(&o.stdout)).trim(), 200)
        ),
        Err(e) => println!("  ✗ gagal: {e}\n    jalankan sendiri: {shown}"),
    }
}

fn skill(mode: Mode, path: &Path, global: bool) {
    let shown = if global { format!("~/{}", path.strip_prefix(std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()).unwrap_or(path).display()) } else { path.display().to_string() };
    match std::fs::read_to_string(path) {
        Ok(text) if text == SKILL => println!("  ✓ sudah terpasang (versi {VERSION}) · {shown}"),
        Ok(_) => {
            println!("  → {shown} berbeda dari Skill di fastpilot {VERSION}");
            if ask(mode, &format!("Perbarui ke {VERSION}?")) {
                write(path, SKILL, "diperbarui");
            }
        }
        Err(_) => {
            println!("  → {shown}");
            if ask(mode, "Pasang?") {
                write(path, SKILL, &format!("terpasang (versi {VERSION}, ikut di binary)"));
            }
        }
    }
}

fn example(mode: Mode) {
    let path = Path::new("tests/contoh.md");
    if path.exists() {
        println!("  ✓ tests/contoh.md sudah ada (tidak diubah)");
        return;
    }
    println!("  → tests/contoh.md");
    if ask(mode, "Buat?") {
        write(path, EXAMPLE, "dibuat (ubah target ke URL aplikasimu)");
    }
}

/// Write without following a symlink: in the project every folder on the way is checked (a
/// checkout cannot point `tests/` at another place); with `--global` only the file itself, so a
/// symlinked ~/.claude of the user's dotfiles keeps working.
fn write(path: &Path, text: &str, done: &str) {
    let made = if path.is_absolute() {
        let parent = path.parent().unwrap_or(Path::new("/"));
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("tidak bisa membuat {}: {e}", parent.display()))
            .and_then(|()| crate::demo::write_safe(parent, Path::new(path.file_name().unwrap_or_default()), text.as_bytes(), false))
    } else {
        crate::demo::write_safe(Path::new("."), path, text.as_bytes(), false)
    };
    match made {
        Ok(()) => println!("  ✓ {done}"),
        Err(e) => println!("  ✗ {e}"),
    }
}

/// The limits as fastpilot applies them now (settings, environment, built-in defaults).
fn limits() {
    let from = |key: &str| match fastpilot_core::config::current(key) {
        fastpilot_core::config::Source::Env(_) => "dari environment",
        fastpilot_core::config::Source::File(_) => "dari pengaturan",
        fastpilot_core::config::Source::Default => "bawaan",
    };
    let words = |w: Vec<String>| {
        let n = w.len();
        let head: Vec<String> = w.into_iter().take(4).collect();
        if n > 4 { format!("{}, … ({n} kata)", head.join(", ")) } else { head.join(", ") }
    };
    let blocked = fastpilot_core::policy::blocked_words();
    if blocked.is_empty() {
        println!("  ! belum ada tombol yang selalu diblokir (mis. Bayar, Checkout)");
    } else {
        println!("  ✓ tombol yang selalu diblokir: {} ({})", words(blocked), from("block_actions"));
    }
    let confirm = fastpilot_core::policy::risky_words();
    if !confirm.is_empty() {
        println!("  ✓ tombol yang butuh konfirmasi: {} ({})", words(confirm), from("confirm_actions"));
    }
    println!("    ubah di: fastpilot ui → Pengaturan › Batasan & izin");
    if via_npx() {
        println!("    (lewat npx: npx -y fastpilot ui)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_is_the_repository_skill_and_the_command_is_claudes() {
        let repo = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../skills/fastpilot/SKILL.md")).expect("skill");
        assert_eq!(SKILL, repo);
        let args = add_args("project");
        assert_eq!(args[..6], ["mcp", "add", "fastpilot", "--scope", "project", "--"]);
        assert_eq!(args.last().map(String::as_str), Some("mcp"));
        assert!(EXAMPLE.contains("platform: web") && EXAMPLE.contains("target: https://contoh.com") && EXAMPLE.contains("id: contoh"));
    }
}
