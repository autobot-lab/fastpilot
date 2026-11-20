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
    assert!(out.contains("plan only, nothing is changed") && out.contains("claude mcp add fastpilot --scope project -- fastpilot mcp"), "{out}");
    assert!(files(&b.project()).is_empty() && files(&b.home()).is_empty(), "nothing written without a terminal");
    assert!(b.adds().is_empty(), "claude mcp add not run");

    // D3: no question outside a terminal, one line instead.
    assert!(out.contains("Dashboard: run `fastpilot ui` and keep this window open.") && !out.contains("[Y/n]"), "{out}");

    // --yes: every step.
    let (code, out) = b.init(&["--yes"], FAKE);
    assert_eq!(code, Some(0), "{out}");
    for line in [
        "fastpilot 0.",
        "1/5 Check this computer",
        "✓ web",
        "2/5 Register fastpilot with Claude Code (this project)",
        "✓ registered (.mcp.json)",
        "3/5 Install the fastpilot Skills",
        "✓ installed (version",
        "built into fastpilot): 21 skills",
        "4/5 Example test",
        "✓ created (change its target to your app's URL)",
        "5/5 Safe limits",
        "change them in: fastpilot ui → Settings › Limits & permissions",
        "Ready. Next:",
        "\"Use fastpilot to run tests/example.md\"",
        "Want to try the demo first? fastpilot demo",
    ] {
        assert!(out.contains(line), "missing {line:?}:\n{out}");
    }
    assert_eq!(b.adds(), vec![["mcp", "add", "fastpilot", "--scope", "project", "--", "fastpilot", "mcp"].iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()]);
    let mcp: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(b.project().join(".mcp.json")).expect(".mcp.json")).expect("json");
    assert_eq!(mcp["mcpServers"]["fastpilot"]["command"], "fastpilot");
    assert_eq!(std::fs::read_to_string(b.project().join(".claude/skills/fastpilot/SKILL.md")).expect("skill"), skill);
    // S5 (v0.4): the whole skill library, each the repository's text.
    for dir in std::fs::read_dir(format!("{ROOT}/skills")).expect("skills").flatten() {
        let name = dir.file_name();
        let repo = std::fs::read_to_string(dir.path().join("SKILL.md")).expect("repo skill");
        let got = std::fs::read_to_string(b.project().join(".claude/skills").join(&name).join("SKILL.md")).unwrap_or_default();
        assert_eq!(got, repo, "{name:?} installed");
    }
    let example = std::fs::read_to_string(b.project().join("tests/example.md")).expect("example");
    assert!(example.contains("platform: web") && example.contains("target: https://example.com"), "{example}");
    assert!(!b.project().join("tests/contoh.md").exists(), "only the English example");
    assert!(!b.home().join(".fastpilot/config.toml").exists(), "no settings written");

    // Again: nothing changes, every step ✓, no second registration.
    let before = files(&b.project());
    let (code, out) = b.init(&["--yes"], FAKE);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("✓ already registered (.mcp.json)") && out.contains("✓ already installed (21 skills, version") && out.contains("✓ tests/example.md already exists (not changed)"), "{out}");
    assert!(!out.contains("[y/N]"), "nothing left to ask: {out}");
    assert_eq!(files(&b.project()), before, "a second run changes nothing");
    assert_eq!(b.adds().len(), 1);

    // A different Skill and my own example: kept without agreement, the Skill updated with --yes.
    std::fs::write(b.project().join(".claude/skills/fastpilot/SKILL.md"), "old skill").expect("old");
    std::fs::write(b.project().join("tests/example.md"), "my test").expect("mine");
    let (_, out) = b.init(&[], FAKE);
    assert!(out.contains("1 skill(s) differ from fastpilot") && out.contains(": fastpilot\n") && out.contains("Update them to"), "{out}");
    assert_eq!(std::fs::read_to_string(b.project().join(".claude/skills/fastpilot/SKILL.md")).expect("skill"), "old skill");
    let (_, out) = b.init(&["--yes"], FAKE);
    assert!(out.contains("✓ updated"), "{out}");
    assert_eq!(std::fs::read_to_string(b.project().join(".claude/skills/fastpilot/SKILL.md")).expect("skill"), skill);
    assert_eq!(std::fs::read_to_string(b.project().join("tests/example.md")).expect("mine"), "my test", "an existing test is never replaced");
}

/// E1 (v0.4): the example is tests/example.md in English; a project that already has the
/// tests/contoh.md of an older fastpilot gets no second example, and is pointed at its own.
#[test]
fn init_keeps_an_older_contoh_example() {
    let b = Box_::new("contoh");
    std::fs::create_dir_all(b.project().join("tests")).expect("tests");
    std::fs::write(b.project().join("tests/contoh.md"), "my older example").expect("contoh");
    let (code, out) = b.init(&["--yes"], FAKE);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("✓ tests/contoh.md already exists (not changed)") && out.contains("\"Use fastpilot to run tests/contoh.md\""), "{out}");
    assert!(!b.project().join("tests/example.md").exists(), "no second example");
    assert_eq!(std::fs::read_to_string(b.project().join("tests/contoh.md")).expect("contoh"), "my older example");
}

#[test]
fn init_without_claude_code_prints_the_command_and_global_uses_the_user_scope() {
    let b = Box_::new("noclaude");
    let (code, out) = b.init(&["--yes"], "/nonexistent/claude");
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("Claude Code not found") && out.contains("claude mcp add fastpilot --scope project -- fastpilot mcp"), "{out}");
    assert!(b.project().join(".claude/skills/fastpilot/SKILL.md").exists() && b.project().join("tests/example.md").exists(), "the other steps still run");

    let g = Box_::new("global");
    let (code, out) = g.init(&["--yes", "--global"], FAKE);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("(every folder)"), "{out}");
    assert_eq!(g.adds()[0][..5], ["mcp", "add", "fastpilot", "--scope", "user"]);
    assert!(g.home().join(".claude/skills/fastpilot/SKILL.md").exists() && !g.project().join(".claude").exists(), "Skill for every folder");
    let (_, out) = g.init(&["--yes", "--global"], FAKE);
    assert!(out.contains("✓ already registered") && g.adds().len() == 1, "{out}");
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
    assert!(out.matches("symlink; fastpilot does not write through symlinks").count() >= 2, "{out}");
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

/// QA B1 (real Claude, v0.4): init said "✓ claude Claude Code 2.1.144" and Chat then failed on
/// every message. Now step 1 names the problem and the fix.
#[test]
fn init_says_when_claude_code_is_too_old() {
    let b = Box_::new("oldclaude");
    let old = b.root.join("claude-old");
    std::fs::write(&old, format!("#!/bin/sh\nFAKE_CLAUDE_VERSION=2.1.144 exec {FAKE} \"$@\"\n")).expect("wrapper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let (_, out) = b.init(&["--yes"], &old.to_string_lossy());
    assert!(out.contains("Claude Code is too old: run `claude update`"), "{out}");
    assert!(!out.contains("✓ Claude Code 2.1.144"), "{out}");
    let _ = std::fs::remove_dir_all(&b.root);
}
