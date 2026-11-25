//! v0.7.1 FC2: what the dashboard's pages do, from the terminal, in FastUI/UX's words
//! (TEXT-terminal-mcp.md): apps (`fastpilot app`), one-click schedules with pause and resume
//! (`fastpilot schedule add|pause|resume|remove`), the app's passwords (`fastpilot secret`, typed
//! hidden or piped in, never an argument, in the dashboard's store), an agent's skills
//! (`fastpilot agent skills`, General QA always on) and Stop (`fastpilot agent stop`: only that
//! agent's running fastpilot processes end; others keep running).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn home() -> PathBuf {
    let h = std::env::temp_dir().join(format!("fp-cli-control-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(&h).expect("home");
    std::fs::canonicalize(&h).expect("home")
}

fn cmd(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    c.current_dir(home)
        .env("HOME", home)
        .env("FASTPILOT_KEYSTORE", "file")
        .env("FASTPILOT_SCHEDULER", "launchd")
        .env("FASTPILOT_SCHEDULER_DIR", home.join("agents"))
        .env("FASTPILOT_LIVE_DIR", home.join("live"))
        // No phone is used: a closed adb port.
        .env("FASTPILOT_ADB_SERVER", "127.0.0.1:1")
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_DIR")
        .env_remove("FASTPILOT_VARS")
        .env_remove("FASTPILOT_AGENT");
    c
}

/// `fastpilot ARGS` in `home` (stdin piped from `input`): (exit code, stdout + stderr).
fn fp(home: &Path, args: &[&str], input: &str) -> (i32, String) {
    let mut c = cmd(home).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("fastpilot");
    c.stdin.take().expect("stdin").write_all(input.as_bytes()).expect("input");
    let o = c.wait_with_output().expect("wait");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    // The Undo id is random: shown as <id> here.
    let text: String = text.lines().map(|l| if l.starts_with("  Undo: fastpilot undo ") { "  Undo: fastpilot undo <id>".to_owned() } else { l.to_owned() }).map(|l| l + "\n").collect();
    (o.status.code().unwrap_or(-1), text)
}

fn apps_json(home: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(home.join("fastpilot/apps.json")).expect("apps.json")).expect("json")
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A `fastpilot mcp` working for `agent`, as the dashboard's Chat starts it, once Live lists it.
fn mcp_for(home: &Path, agent: &str) -> Kill {
    let child = cmd(home).arg("mcp").env("FASTPILOT_AGENT", agent).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("mcp");
    let file = home.join("live").join(format!("{}.json", child.id()));
    for _ in 0..100 {
        if file.is_file() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(file.is_file(), "Live lists the process");
    Kill(child)
}

fn running(k: &mut Kill) -> bool {
    k.0.try_wait().expect("wait").is_none()
}

#[test]
fn apps_passwords_schedules_skills_and_stop_from_the_terminal() {
    let home = home();

    // `--help` lists the new commands.
    let (_, help) = fp(&home, &["--help"], "");
    for line in ["  app        Add, list, switch or remove the apps fastpilot tests", "  schedule   Run tests on a timer: add, pause, resume", "  secret     Save test passwords for an app (values are never shown)", "  agent      Skills, models and Stop for an agent", "  provider   Connect AI providers: Claude Code, OpenAI, OpenRouter, Groq, Ollama, Other"] {
        assert!(help.contains(line), "{line}: {help}");
    }

    // Apps.
    assert_eq!(fp(&home, &["app", "list"], ""), (0, "No apps yet. Add one: fastpilot app add https://your-site.example\n".into()));
    let (code, out) = fp(&home, &["app", "add"], "");
    assert_eq!((code, out.as_str()), (1, "✗ No address or app id was given. Run: fastpilot app add https://shop.example\n"));
    let (code, out) = fp(&home, &["app", "add", "shop"], "");
    assert_eq!((code, out.as_str()), (1, "✗ “shop” is not a web address or an app id. Use a full address like https://shop.example, or an app id like com.shop.app.\n"));
    let (code, out) = fp(&home, &["app", "add", "https://shop.example"], "");
    assert_eq!((code, out.as_str()), (0, "✓ Added Shop https://shop.example/\n  Undo: fastpilot undo <id>\n"));
    assert_eq!(fp(&home, &["app", "add", "shop.example"], "").1, "✓ Shop is already added. Nothing changed.\n");
    let (code, out) = fp(&home, &["app", "add", "com.orders.app"], "");
    assert_eq!((code, out.as_str()), (0, "✓ Added Orders com.orders.app · Android · not seen on a running device yet\n  Undo: fastpilot undo <id>\n"));
    let (_, list) = fp(&home, &["app", "list"], "");
    assert!(list.starts_with("Apps\n  ● Shop") && list.lines().nth(1).is_some_and(|l| l.ends_with("in use") && l.contains("Website")) && list.contains("Orders") && list.contains("com.orders.app"), "{list}");
    assert_eq!(fp(&home, &["app", "switch", "orders"], "").1, "✓ Switched to Orders\n");
    assert_eq!(fp(&home, &["app", "switch", "Orders"], "").1, "✓ Orders is already in use. Nothing changed.\n");
    let (code, out) = fp(&home, &["app", "switch", "shp"], "");
    assert_eq!((code, out.as_str()), (1, "✗ No app called “shp”. Did you mean “Shop”? Run fastpilot app list to see your apps.\n"));
    assert_eq!(apps_json(&home)["current"], "orders");
    assert_eq!(fp(&home, &["app", "switch", "Shop"], "").0, 0);
    let shop = "shop";

    // Passwords: piped in (a person types it hidden), never an argument; the dashboard's store.
    let (code, out) = fp(&home, &["secret", "set", "TEST_PASSWORD", "hunter2-in-history"], "");
    assert_eq!((code, out.as_str()), (1, "✗ Do not put the password in the command. Run: fastpilot secret set TEST_PASSWORD and type it when asked. Nothing was saved.\n"));
    assert_eq!(fp(&home, &["secret", "set", "test password"], "x\n").1, "✗ “test password” is not a valid name. Use letters, numbers and underscores, like TEST_PASSWORD.\n");
    // E5 (#231): a 1-character password is saved too.
    assert_eq!(fp(&home, &["secret", "set", "SHORT"], "7\n").1.lines().last(), Some("✓ Saved password SHORT for Shop"));
    assert_eq!(fp(&home, &["secret", "remove", "SHORT"], "").0, 0);
    assert_eq!(fp(&home, &["secret", "set", "EMPTY"], "").1.lines().last(), Some("✗ No password was given. Run this in a terminal and type it when asked, or pipe it in."));
    let (code, out) = fp(&home, &["secret", "set", "TEST_PASSWORD"], "fp-cli-pass-77\n");
    assert_eq!((code, out.lines().last()), (0, Some("✓ Saved password TEST_PASSWORD for Shop")), "{out}");
    assert!(!out.contains("fp-cli-pass-77"), "piped in: no prompt, never the value: {out}");
    assert_eq!(fp(&home, &["secret", "set", "TEST_PASSWORD"], "fp-cli-pass-78\n").1.lines().last(), Some("✓ Changed password TEST_PASSWORD for Shop"));
    assert_eq!(fp(&home, &["secret", "list"], "").1, "Passwords for Shop (names only, values are never shown)\n  TEST_PASSWORD\n");
    let kept = home.join(format!(".fastpilot/keys/secrets/{shop}/TEST_PASSWORD"));
    assert!(kept.is_file(), "the dashboard's store: {}", kept.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&kept).expect("kept").permissions().mode() & 0o777, 0o600);
    }
    let config = std::fs::read_to_string(home.join(".fastpilot/config.toml")).unwrap_or_default();
    assert!(!config.contains("TEST_PASSWORD"), "not on the environment list: {config}");
    let log = std::fs::read_to_string(home.join(".fastpilot/audit.jsonl")).unwrap_or_default();
    assert!(log.contains("secret TEST_PASSWORD of") && log.contains("terminal") && !log.contains("fp-cli-pass-7"), "{log}");
    assert_eq!(fp(&home, &["secret", "remove", "NOPE"], "").1, "✗ No password called “NOPE” for Shop. Saved: TEST_PASSWORD.\n");

    // FastTest (v0.7.1, Low): an unknown schedule id in TEXT 2.2's words.
    assert_eq!(fp(&home, &["schedule", "run", "nope"], ""), (1, "✗ No schedule with id “nope”. Run fastpilot schedule list to see them.\n".into()));
    // Schedules: no saved tests yet, then the presets, pause, resume, remove.
    let (code, out) = fp(&home, &["schedule", "add", "--nightly"], "");
    assert_eq!((code, out.as_str()), (1, "✗ Shop has no saved tests yet. Run something first, then save it as a test.\n"));
    let tests = home.join(format!("fastpilot/apps/{shop}/tests"));
    std::fs::create_dir_all(&tests).expect("tests");
    std::fs::write(tests.join("login.md"), "# Check the login\nid: login\nplatform: web\ntarget: https://shop.example\n- Open the page\n").expect("test");
    assert_eq!(fp(&home, &["schedule", "add", "--nightly", "--every-2h"], "").1, "✗ Choose one of --every-2h, --nightly or --before-release.\n");
    let (code, out) = fp(&home, &["schedule", "add", "--nightly"], "");
    assert_eq!((code, out.as_str()), (0, "✓ Created schedule “Every night” · all tests of Shop · next run 02:00\n  Undo: fastpilot undo <id>\n"));
    let entries = || std::fs::read_dir(home.join("agents")).map(|d| d.count()).unwrap_or(0);
    assert_eq!(entries(), 1, "installed with the system scheduler");
    assert_eq!(fp(&home, &["schedule", "add", "--nightly"], "").1, "✓ “Every night” is already on. Nothing changed.\n");
    assert_eq!(fp(&home, &["schedule", "pause", "every night"], "").1, "✓ Paused schedule “Every night” · its history is kept\n  Undo: fastpilot undo <id>\n");
    assert_eq!(entries(), 0);
    assert_eq!(fp(&home, &["schedule", "pause", "every night"], "").1, "✓ “Every night” is already paused. Nothing changed.\n");
    assert_eq!(fp(&home, &["schedule", "resume", "every night"], "").1, "✓ Resumed schedule “Every night” · next run 02:00\n  Undo: fastpilot undo <id>\n");
    assert_eq!(entries(), 1);
    assert_eq!(fp(&home, &["schedule", "remove", "Every night"], "").1, "✓ Removed schedule “Every night” · kept 7 days\n  Undo: fastpilot undo <id>\n");
    assert_eq!(entries(), 0);
    let (code, out) = fp(&home, &["schedule", "pause", "9"], "");
    assert_eq!((code, out.as_str()), (1, "✗ No schedule with id “9”. Run fastpilot schedule list to see them.\n"));
    assert_eq!(fp(&home, &["schedule", "add", "--before-release", "--tests", "nope"], "").1, "✗ No test called “nope” in Shop. Saved tests: Check the login.\n");
    assert_eq!(fp(&home, &["schedule", "add", "--before-release", "--tests", "login"], "").1, "✓ Created schedule “Before each release” · Check the login · runs when you check a release\n  Undo: fastpilot undo <id>\n");

    // An agent's skills: General QA always on, specialists by id, in the order given.
    let (code, out) = fp(&home, &["agent", "skills", "andro"], "");
    assert_eq!((code, out.as_str()), (1, "✗ No agent called “andro”. Agents: My tester.\n"));
    assert_eq!(fp(&home, &["agent", "skills", "My tester", "reset"], "").1, "✓ My tester is already General QA. Nothing changed.\n");
    let (_, out) = fp(&home, &["agent", "skills", "My tester"], "");
    assert!(out.starts_with("My tester · General QA\n\nGeneral QA (always on)\n  Let fastpilot choose the technique, Use case scenarios, Try to break it,") && out.ends_with("No specialist skills on.\n"), "{out}");
    assert_eq!(fp(&home, &["agent", "skills", "My tester", "+safe-checkout", "+decision-table"], "").1, "✓ My tester: + Safe checkout, + Decision table\n  Undo: fastpilot undo <id>\n");
    assert_eq!(fp(&home, &["agent", "skills", "My tester", "+safe-checkout"], "").1, "✓ Safe checkout is already on for My tester. Nothing changed.\n");
    assert_eq!(fp(&home, &["agent", "skills", "My tester", "-pairwise"], "").1, "✓ Pairwise is not on for My tester. Nothing changed.\n");
    assert_eq!(fp(&home, &["agent", "skills", "My tester", "+regression"], "").1, "✓ Regression is part of General QA and always on. Nothing changed.\n");
    let (code, out) = fp(&home, &["agent", "skills", "My tester", "-regression"], "");
    assert_eq!((code, out.as_str()), (1, "✗ Regression is part of General QA, which is always on. It cannot be removed. Nothing changed.\n"));
    assert_eq!(fp(&home, &["agent", "skills", "My tester", "+pairwize"], "").1, "✗ No skill called “pairwize”. Did you mean pairwise? Run fastpilot agent skills <agent> --all to see every skill.\n");
    assert_eq!(fp(&home, &["agent", "skills", "My tester", "+jira-bug-report"], "").1, "✗ “Jira · Bug report” appears after you connect Jira. Connect it in the dashboard: Settings › Integrations › Jira. You can check with fastpilot jira status. Nothing changed.\n");
    let (_, out) = fp(&home, &["agent", "skills", "My tester"], "");
    assert!(out.starts_with("My tester · General QA + 2 specialist skills\n") && out.contains("Specialist skills on\n  Checklist · Safe checkout    safe-checkout\n  Decision table               decision-table\n"), "{out}");
    let (_, out) = fp(&home, &["agent", "skills", "My tester", "--all"], "");
    assert!(out.contains("Black-box · Design tests from rules\n  off  Equivalence partitions") && out.contains("  on   Decision table") && out.contains("  on   Checklist · Safe checkout") && !out.contains("Jira ·") && out.ends_with("Jira skills appear after you connect Jira.\n"), "{out}");
    assert_eq!(fp(&home, &["agent", "skills", "My tester", "-decision-table"], "").1, "✓ My tester: − Decision table\n  Undo: fastpilot undo <id>\n");
    assert_eq!(fp(&home, &["agent", "skills", "My tester", "reset"], "").1, "✓ My tester: back to General QA (− Safe checkout)\n  Undo: fastpilot undo <id>\n");

    // Stop: only this agent's running fastpilot processes end; another agent's keeps running.
    assert_eq!(fp(&home, &["agent", "stop", "My tester"], "").1, "✓ Nothing to stop. My tester has no running chats.\n");
    let mut mine = mcp_for(&home, "default");
    let mut other = mcp_for(&home, "android-regression");
    let (code, out) = fp(&home, &["agent", "stop", "My tester"], "");
    assert_eq!((code, out.as_str()), (0, "✓ Stopped 1 chat in My tester\n  To start them again, say “run it again” in Chat.\n"));
    assert!(!running(&mut mine), "this agent's process ended");
    assert!(running(&mut other), "another agent's keeps running");
    assert_eq!(fp(&home, &["agent", "stop", "My tester"], "").1, "✓ Nothing to stop. My tester has no running chats.\n");
    drop((mine, other));

    // FC6: removing asks nothing (it is kept 7 days, with Undo); --yes is accepted and ignored.
    assert_eq!(fp(&home, &["app", "remove", "Orders"], "").1, "✓ Removed app “Orders” · kept 7 days\n  Undo: fastpilot undo <id>\n");
    let (_, out) = fp(&home, &["app", "remove", "Shop", "--yes"], "");
    assert_eq!(out, "✓ Removed app “Shop” · 1 test, 1 password, 1 schedule · kept 7 days\n  Undo: fastpilot undo <id>\n  No app is in use now. Add one: fastpilot app add <address>\n");
    let _ = std::fs::remove_dir_all(&home);
}
