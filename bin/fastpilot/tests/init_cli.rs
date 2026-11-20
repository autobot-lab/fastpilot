//! F4 `fastpilot init` in an empty project folder with an isolated HOME and the stand-in Claude
//! Code (tests/fake-claude): `--yes` registers fastpilot with the right `claude mcp add` arguments,
//! installs the Skill and writes tests/contoh.md; a second run changes nothing; outside a terminal
//! without `--yes` nothing changes; without Claude Code the command to run is printed; `--global`
//! uses the user scope; a different Skill is only replaced when agreed.

use std::path::{Path, PathBuf};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude");

struct Box_ {
    root: PathBuf,
}
impl Box_ {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("fp-init-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home")).expect("home");
        std::fs::create_dir_all(root.join("proyek")).expect("project");
        Self { root }
    }
    fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    fn project(&self) -> PathBuf {
        self.root.join("proyek")
    }
    fn log(&self) -> PathBuf {
        self.root.join("claude-log")
    }
    /// `fastpilot init <args>` in the project, stdin not a terminal.
    fn init(&self, args: &[&str], claude: &str) -> (Option<i32>, String) {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
            .arg("init")
            .args(args)
            .current_dir(self.project())
            .env("HOME", self.home())
            .env("FASTPILOT_CLAUDE", claude)
            .env("FAKE_CLAUDE_LOG", self.log())
            .env("FASTPILOT_ADB_SERVER", "127.0.0.1:1")
            .env_remove("FASTPILOT_CONFIG")
            .env_remove("FASTPILOT_VIA_NPX")
            .env_remove("npm_command")
            .env_remove("FASTPILOT_BLOCK_ACTIONS")
            .env_remove("FASTPILOT_CONFIRM_ACTIONS")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("init");
        (out.status.code(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }
    /// `claude mcp add` calls seen by the stand-in.
    fn adds(&self) -> Vec<Vec<String>> {
        let Ok(dir) = std::fs::read_dir(self.log()) else { return Vec::new() };
        let mut calls: Vec<(PathBuf, Vec<String>)> = dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with("-mcp.json"))
            .map(|p| {
                let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).expect("log")).expect("json");
                (p, v["argv"].as_array().expect("argv").iter().filter_map(|a| a.as_str().map(str::to_owned)).collect())
            })
            .collect();
        calls.sort();
        calls.into_iter().map(|(_, a)| a).filter(|a| a.get(1).is_some_and(|x| x == "add")).collect()
    }
}
impl Drop for Box_ {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
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

#[test]
fn init_sets_up_a_folder_once_and_leaves_it_alone_after() {
    let b = Box_::new("main");
    let skill = std::fs::read_to_string(format!("{ROOT}/skills/fastpilot/SKILL.md")).expect("skill");

    // Outside a terminal without --yes: the plan only.
    let (code, out) = b.init(&[], FAKE);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("hanya rencana, tidak ada yang diubah") && out.contains("claude mcp add fastpilot --scope project -- fastpilot mcp"), "{out}");
    assert!(files(&b.project()).is_empty() && files(&b.home()).is_empty(), "nothing written without a terminal");
    assert!(b.adds().is_empty(), "claude mcp add not run");

    // --yes: every step.
    let (code, out) = b.init(&["--yes"], FAKE);
    assert_eq!(code, Some(0), "{out}");
    for line in [
        "fastpilot 0.",
        "1/5 Cek mesin",
        "✓ web",
        "2/5 Daftarkan fastpilot ke Claude Code (proyek ini)",
        "✓ terdaftar (.mcp.json)",
        "3/5 Pasang Skill fastpilot",
        "✓ terpasang (versi",
        "4/5 Contoh test",
        "✓ dibuat (ubah target ke URL aplikasimu)",
        "5/5 Batasan aman",
        "ubah di: fastpilot ui → Pengaturan › Batasan & izin",
        "Siap. Langkah berikutnya:",
        "\"Pakai fastpilot, jalankan tests/contoh.md\"",
        "Mau coba demo dulu? fastpilot demo",
    ] {
        assert!(out.contains(line), "missing {line:?}:\n{out}");
    }
    assert_eq!(b.adds(), vec![["mcp", "add", "fastpilot", "--scope", "project", "--", "fastpilot", "mcp"].iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()]);
    let mcp: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(b.project().join(".mcp.json")).expect(".mcp.json")).expect("json");
    assert_eq!(mcp["mcpServers"]["fastpilot"]["command"], "fastpilot");
    assert_eq!(std::fs::read_to_string(b.project().join(".claude/skills/fastpilot/SKILL.md")).expect("skill"), skill);
    let example = std::fs::read_to_string(b.project().join("tests/contoh.md")).expect("example");
    assert!(example.contains("platform: web") && example.contains("target: https://contoh.com"), "{example}");
    assert!(!b.home().join(".fastpilot/config.toml").exists(), "no settings written");

    // Again: nothing changes, every step ✓, no second registration.
    let before = files(&b.project());
    let (code, out) = b.init(&["--yes"], FAKE);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("✓ sudah terdaftar (.mcp.json)") && out.contains("✓ sudah terpasang (versi") && out.contains("✓ tests/contoh.md sudah ada (tidak diubah)"), "{out}");
    assert!(!out.contains("[y/N]"), "nothing left to ask: {out}");
    assert_eq!(files(&b.project()), before, "a second run changes nothing");
    assert_eq!(b.adds().len(), 1);

    // A different Skill and my own example: kept without agreement, the Skill updated with --yes.
    std::fs::write(b.project().join(".claude/skills/fastpilot/SKILL.md"), "old skill").expect("old");
    std::fs::write(b.project().join("tests/contoh.md"), "my test").expect("mine");
    let (_, out) = b.init(&[], FAKE);
    assert!(out.contains("berbeda dari Skill di fastpilot") && out.contains("Perbarui ke"), "{out}");
    assert_eq!(std::fs::read_to_string(b.project().join(".claude/skills/fastpilot/SKILL.md")).expect("skill"), "old skill");
    let (_, out) = b.init(&["--yes"], FAKE);
    assert!(out.contains("✓ diperbarui"), "{out}");
    assert_eq!(std::fs::read_to_string(b.project().join(".claude/skills/fastpilot/SKILL.md")).expect("skill"), skill);
    assert_eq!(std::fs::read_to_string(b.project().join("tests/contoh.md")).expect("mine"), "my test", "an existing test is never replaced");
}

#[test]
fn init_without_claude_code_prints_the_command_and_global_uses_the_user_scope() {
    let b = Box_::new("noclaude");
    let (code, out) = b.init(&["--yes"], "/nonexistent/claude");
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("Claude Code tidak ditemukan") && out.contains("claude mcp add fastpilot --scope project -- fastpilot mcp"), "{out}");
    assert!(b.project().join(".claude/skills/fastpilot/SKILL.md").exists() && b.project().join("tests/contoh.md").exists(), "the other steps still run");

    let g = Box_::new("global");
    let (code, out) = g.init(&["--yes", "--global"], FAKE);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("(semua folder)"), "{out}");
    assert_eq!(g.adds()[0][..5], ["mcp", "add", "fastpilot", "--scope", "user"]);
    assert!(g.home().join(".claude/skills/fastpilot/SKILL.md").exists() && !g.project().join(".claude").exists(), "Skill for every folder");
    let (_, out) = g.init(&["--yes", "--global"], FAKE);
    assert!(out.contains("✓ sudah terdaftar") && g.adds().len() == 1, "{out}");
}

/// Security Low 1: a project whose `tests/` or `.claude/` is a symlink: init writes nothing through it.
#[cfg(unix)]
#[test]
fn init_never_writes_through_a_symlink() {
    let b = Box_::new("symlink");
    let outside = b.root.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    std::os::unix::fs::symlink(&outside, b.project().join("tests")).expect("link");
    std::os::unix::fs::symlink(&outside, b.project().join(".claude")).expect("link");
    let (code, out) = b.init(&["--yes"], FAKE);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.matches("symlink; fastpilot tidak menulis lewat symlink").count() >= 2, "{out}");
    assert_eq!(std::fs::read_dir(&outside).expect("outside").count(), 0, "nothing written outside the project");
}

#[test]
fn help_names_init_and_demo() {
    for args in [vec!["help"], vec!["--help"]] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).args(&args).env("HOME", std::env::temp_dir()).output().expect("help");
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(text.contains("fastpilot init") || text.contains("init ["), "{args:?}: {text}");
        assert!(text.contains("demo"), "{args:?}: {text}");
    }
}
