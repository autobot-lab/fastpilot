//! `fastpilot demo [folder]` (F2): the demo pages, example test cases and an MCP config, written to
//! a new folder, so a first test runs without a checkout of the repository. `--android` installs
//! the demo app (built into the binary) on the attached device or emulator.
//!
//! Everything is embedded at build time: the pages and test cases with `include_str!` (the very
//! files of the repository; test cases point at `demo/` instead of `examples/demo/`), the APK
//! through `build.rs`.

use std::path::{Path, PathBuf};

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The demo pages, as in `examples/demo/`.
pub const PAGES: &[(&str, &str)] = &[
    ("index.html", include_str!("../../../examples/demo/index.html")),
    ("account.html", include_str!("../../../examples/demo/account.html")),
    ("help.html", include_str!("../../../examples/demo/help.html")),
    ("login.html", include_str!("../../../examples/demo/login.html")),
    ("widgets.html", include_str!("../../../examples/demo/widgets.html")),
    ("pin.html", include_str!("../../../examples/demo/pin.html")),
    ("secret.html", include_str!("../../../examples/demo/secret.html")),
    ("maestro.html", include_str!("../../../examples/demo/maestro.html")),
    // S4: pages for practising test design techniques, each with one bug to find (the answers are in
    // docs/demo-planted-bugs.md, which is not written to the demo folder).
    ("tickets.html", include_str!("../../../examples/demo/tickets.html")),
    ("shipping.html", include_str!("../../../examples/demo/shipping.html")),
    ("orders.html", include_str!("../../../examples/demo/orders.html")),
];

/// The technique practice pages and what to try on each.
pub const PRACTICE: &[(&str, &str)] = &[
    ("tickets.html", "boundary values"),
    ("shipping.html", "a decision table"),
    ("orders.html", "state transitions"),
];

/// The example test cases of `tests/`, with the platform shown in the output.
pub const TESTS: &[(&str, &str, &str)] = &[
    ("login.md", include_str!("../../../tests/login.md"), "web"),
    ("login-android.md", include_str!("../../../tests/login-android.md"), "android"),
    ("login-cross-platform.md", include_str!("../../../tests/login-cross-platform.md"), "web + android"),
];

/// The Android demo app (empty when it was not built before this binary).
const APK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fastpilot-demo.apk"));
const APP: &str = "com.fastpilot.demo";

/// The public demo password of the demo pages and app (not a secret).
pub const DEMO_PASS: &str = "rahasia123";

/// "Android emulator (emulator-5556), Android phone (physical) (R58M…)" (QA B9: raw ids only).
fn labelled(serials: &[String]) -> String {
    serials.iter().map(|s| format!("{} ({s})", fastpilot_core::DeviceKind::of_android_serial(s).label())).collect::<Vec<_>>().join(", ")
}

/// A test case as written to the demo folder: pages are under `demo/`.
pub fn test_case(text: &str) -> String {
    text.replace("examples/demo/", "demo/")
}

/// Started through `npx` (the npm launcher sets FASTPILOT_VIA_NPX; npm itself sets npm_command).
pub fn via_npx() -> bool {
    std::env::var("FASTPILOT_VIA_NPX").is_ok_and(|v| v == "1") || std::env::var("npm_command").is_ok_and(|v| v == "exec")
}

/// How an MCP client starts this fastpilot: `fastpilot mcp`, or `npx -y fastpilot mcp`.
pub fn mcp_command() -> (String, Vec<String>) {
    if via_npx() {
        ("npx".into(), vec!["-y".into(), "fastpilot".into(), "mcp".into()])
    } else {
        ("fastpilot".into(), vec!["mcp".into()])
    }
}

/// `.mcp.json` of the demo folder: fastpilot with the demo password.
pub fn mcp_json() -> String {
    let (command, args) = mcp_command();
    let v = serde_json::json!({
        "mcpServers": {
            "fastpilot": {
                "type": "stdio",
                "command": command,
                "args": args,
                "env": { "FASTPILOT_VARS": "DEMO_PASS", "DEMO_PASS": DEMO_PASS },
            }
        }
    });
    format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default())
}

/// Every file of the demo folder, relative path and contents (`.mcp.json` is written separately).
pub fn files() -> Vec<(PathBuf, Vec<u8>)> {
    let mut out: Vec<(PathBuf, Vec<u8>)> = PAGES.iter().map(|(n, t)| (Path::new("demo").join(n), t.as_bytes().to_vec())).collect();
    out.extend(TESTS.iter().map(|(n, t, _)| (Path::new("tests").join(n), test_case(t).into_bytes())));
    // The Skill, so Claude Code in the demo folder knows fastpilot's steps instead of guessing them.
    // The whole skill library (testing techniques, regression, reports) next to it, as `init` does.
    out.extend(fastpilot_mcp::skills::SKILLS.iter().map(|(n, t)| (Path::new(".claude/skills").join(n).join("SKILL.md"), t.as_bytes().to_vec())));
    out
}

/// Write `base/rel` without following a symlink: `base` (when `check_base`) and every folder below
/// it must not be one, nor the file itself; the bytes go to a new temp file that is renamed over the
/// target (a rename replaces a link, it never writes through it). A folder from an untrusted
/// checkout cannot so redirect `init` or `demo` into ~/.ssh or similar.
pub fn write_safe(base: &Path, rel: &Path, bytes: &[u8], check_base: bool) -> Result<(), String> {
    let link = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink());
    let mut at = base.to_path_buf();
    if check_base && link(&at) {
        return Err(format!("{} is a symlink; fastpilot does not write through symlinks", at.display()));
    }
    let parent = rel.parent().unwrap_or(Path::new(""));
    for part in parent.components() {
        at.push(part);
        if link(&at) {
            return Err(format!("{} is a symlink; fastpilot does not write through symlinks", at.display()));
        }
        if !at.exists() {
            std::fs::create_dir(&at).map_err(|e| format!("cannot create {}: {e}", at.display()))?;
        }
    }
    let path = base.join(rel);
    if link(&path) {
        return Err(format!("{} is a symlink; fastpilot does not write through symlinks", path.display()));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.fastpilot-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| std::io::Write::write_all(&mut f, bytes))
        .and_then(|()| std::fs::rename(&tmp, &path));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write {}: {e}", path.display())
    })
}

/// `.mcp.json` of the demo folder. One that is already there is kept: its other servers stay, the
/// `fastpilot` entry is set, and the old file is saved as `.mcp.json.bak` (or `.bak-2`, ...) first.
/// Returns the backup's name, if one was made.
fn write_mcp_json(dir: &Path) -> Result<Option<String>, String> {
    let path = dir.join(".mcp.json");
    let ours = mcp_json();
    // A symlinked .mcp.json is refused before it is read: the backup would otherwise copy the
    // link's target (any file of the user's) into the demo folder (Security Low 3).
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("{} is a symlink; fastpilot does not write through symlinks", path.display()));
    }
    let Ok(old) = std::fs::read_to_string(&path) else {
        write_safe(dir, Path::new(".mcp.json"), ours.as_bytes(), true)?;
        return Ok(None);
    };
    if old == ours {
        return Ok(None);
    }
    let fresh: serde_json::Value = serde_json::from_str(&ours).unwrap_or_default();
    let merged = match serde_json::from_str::<serde_json::Value>(&old) {
        Ok(mut v) if v["mcpServers"].is_object() => {
            v["mcpServers"]["fastpilot"] = fresh["mcpServers"]["fastpilot"].clone();
            format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default())
        }
        _ => ours,
    };
    if merged == old {
        return Ok(None);
    }
    let backup = (1..)
        .map(|n| if n == 1 { ".mcp.json.bak".to_owned() } else { format!(".mcp.json.bak-{n}") })
        .find(|b| std::fs::symlink_metadata(dir.join(b)).is_err())
        .unwrap_or_else(|| ".mcp.json.bak".into());
    write_safe(dir, Path::new(&backup), old.as_bytes(), true)?;
    write_safe(dir, Path::new(".mcp.json"), merged.as_bytes(), true)?;
    Ok(Some(backup))
}

/// `folder` already holds a demo made by `fastpilot demo` (pages, tests and its .mcp.json).
pub fn is_demo_folder(folder: &Path) -> bool {
    folder.join("demo/login.html").is_file()
        && folder.join("tests/login.md").is_file()
        && std::fs::read_to_string(folder.join(".mcp.json")).is_ok_and(|t| t.contains("\"fastpilot\""))
}

/// What to tell the user when the demo app cannot be installed.
pub fn install_hint(serial: &str, error: &str) -> Option<String> {
    let incompatible = ["INSTALL_FAILED_UPDATE_INCOMPATIBLE", "INSTALL_FAILED_VERSION_DOWNGRADE", "signatures do not match"]
        .iter()
        .any(|e| error.contains(e));
    incompatible.then(|| {
        format!(
            "another demo app (different signature or version) is installed on {serial}. Remove it first, then try again:\n    adb -s {serial} uninstall {APP}\n    fastpilot demo --android --device {serial}"
        )
    })
}

const USAGE: &str = "usage: fastpilot demo [folder] [--force] [--android [--device SERIAL]]
  folder     where to write the demo (default ./fastpilot-demo)
  --force    write into a folder that already exists (files with the same name are replaced)
  --android  also install the demo app on the attached Android device or emulator";

pub async fn demo(args: &[String]) -> Res<()> {
    let mut folder: Option<String> = None;
    let (mut force, mut android, mut device) = (false, false, None::<String>);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--force" => force = true,
            "--android" => android = true,
            "--device" => device = Some(it.next().cloned().ok_or("--device needs a device serial, e.g. emulator-5556")?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            s if s.starts_with('-') => {
                eprintln!("fastpilot demo: unknown option `{s}`\n\n{USAGE}");
                std::process::exit(2);
            }
            s => folder = Some(s.to_owned()),
        }
    }
    let shown = folder.clone().unwrap_or_else(|| "fastpilot-demo".into());
    let dir = PathBuf::from(&shown);
    // Run from inside a demo folder: the new one would end up nested in it.
    if is_demo_folder(Path::new(".")) && !(android && dir.exists()) {
        println!("! You are inside a demo folder. A new folder will be made inside it: {}", std::env::current_dir().map(|c| c.join(&dir).display().to_string()).unwrap_or(shown.clone()));
        println!("  Not what you meant? Leave it with `cd ..`, then run this again.\n");
    }
    // `--android` after the folder was made: only the app.
    let write = !(android && dir.exists() && !force);
    if write {
        if dir.exists() && !force {
            eprintln!("The folder {} already exists; nothing was changed.\nUse another folder (fastpilot demo <folder>) or --force to write its files again.", dir.display());
            std::process::exit(1);
        }
        write_folder(&dir, &shown)?;
    }
    let installed = install_android(android, device.as_deref()).await;
    if write {
        let cd = if shown.contains(char::is_whitespace) { format!("cd \"{shown}\"") } else { format!("cd {shown}") };
        println!("\nTry it now:\n  {cd} && claude\n  then type:\n  \"Use fastpilot to run tests/login.md, then report\"");
        println!("\nPractise test design (each page has one hidden bug):");
        for (page, how) in PRACTICE {
            println!("  \"Test demo/{page} with {how}, using the rules on the page\"");
        }
        println!("\nDemo account: a@b.com, password {DEMO_PASS} (not a secret; saved as DEMO_PASS in .mcp.json, tests write ${{DEMO_PASS}})");
    }
    if installed == Some(false) {
        std::process::exit(1);
    }
    if write {
        // D3: the dashboard for the demo folder (its Chat saves tests there).
        let cd = if shown.contains(char::is_whitespace) { format!("cd \"{shown}\"") } else { format!("cd {shown}") };
        crate::offer::dashboard(&dir, true, Some(&cd));
    }
    Ok(())
}

fn write_folder(dir: &Path, shown: &str) -> Res<()> {
    let prefix = if Path::new(shown).is_absolute() || shown.starts_with('.') { shown.to_owned() } else { format!("./{shown}") };
    println!("Creating the demo folder: {prefix}");
    if std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("{} is a symlink; fastpilot does not write through symlinks", dir.display()).into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for (rel, bytes) in files() {
        write_safe(dir, &rel, &bytes, true)?;
    }
    let backup = write_mcp_json(dir)?;
    let names: Vec<String> = PAGES.iter().take(2).map(|(n, _)| format!("demo/{n}")).collect();
    println!("  ✓ {:<42} {} pages", format!("{}, …", names.join(", ")), PAGES.len());
    for (n, _, platform) in TESTS {
        println!("  ✓ {:<42} {platform}", format!("tests/{n}"));
    }
    println!("  ✓ {:<42} fastpilot + DEMO_PASS", ".mcp.json");
    if let Some(b) = backup {
        println!("    the existing .mcp.json was saved as {b}; the other servers in it are kept");
    }
    println!("  ✓ {:<42} {} skills: fastpilot steps, test techniques, regression, reports", ".claude/skills/…", fastpilot_mcp::skills::SKILLS.len());
    println!("  ✓ {:<42} {} practice pages, one hidden bug each", format!("demo/{}, …", PRACTICE[0].0), PRACTICE.len());
    Ok(())
}

/// Whether the demo app may go on `serial`: an emulator always, a physical phone only with
/// `allow_physical_devices` (as for a session; emulators are named `emulator-<port>` by adb).
fn physical_allowed(serial: &str) -> bool {
    serial.starts_with("emulator-") || fastpilot_core::config::flag("allow_physical_devices")
}

/// The Android part. `None`: nothing to do (skipped); `Some(true)`: installed; `Some(false)`: failed.
async fn install_android(asked: bool, device: Option<&str>) -> Option<bool> {
    let serials: Vec<String> = match fastpilot_android::devices().await {
        Ok((attached, _)) => attached.into_iter().filter(|(_, st)| st == "device").map(|(s, _)| s).collect(),
        Err(_) => Vec::new(),
    };
    let wanted = device
        .map(str::to_owned)
        .or_else(|| ["FASTPILOT_ANDROID_SERIAL", "ANDROID_SERIAL"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())));
    if !asked {
        if serials.is_empty() {
            println!("\nAndroid (skipped: no device)\n  start an emulator, then: fastpilot demo --android\n  → installs the demo app built into fastpilot");
        } else {
            println!("\nAndroid: connected: {}\n  install the demo app: fastpilot demo --android", labelled(&serials));
        }
        return None;
    }
    let serial = match (wanted, serials.as_slice()) {
        (Some(w), list) if list.contains(&w) => w,
        (Some(w), _) => {
            println!("\nAndroid (skipped: device {w} is not connected)");
            return None;
        }
        (None, []) => {
            println!("\nAndroid (skipped: no device)\n  start an emulator, then run again: fastpilot demo --android");
            return None;
        }
        (None, [one]) => one.clone(),
        (None, many) => {
            println!("\nAndroid: {} devices ({}); pick one: fastpilot demo --android --device <serial>", many.len(), labelled(many));
            return Some(false);
        }
    };
    if APK.is_empty() {
        println!("\nAndroid: this fastpilot build has no demo app.\n  use a release build (npm / install.sh), or from a checkout: agent-android/build.sh, then build again");
        return Some(false);
    }
    if !fastpilot_core::config::flag("allow_app_install") {
        println!("\nAndroid: installing apps is turned off in your limits (allow_app_install = false).\n  change it in: fastpilot ui → Settings › Limits & permissions");
        return Some(false);
    }
    // The same rule as a session: real phones only when the user allows them (T1-T3).
    if !physical_allowed(&serial) {
        println!("\nAndroid: {serial} is a physical phone, and phones are turned off in your limits (allow_physical_devices = false).\n  use an emulator, or change it in: fastpilot ui → Settings › Limits & permissions");
        return Some(false);
    }
    let done = match fastpilot_android::Adb::new().await {
        Ok(adb) => adb.install(&serial, APK).await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match done {
        Ok(()) => {
            println!("  ✓ {serial} · {APP} installed\n  type in Claude: \"Use fastpilot to run tests/login-android.md\"");
            Some(true)
        }
        Err(e) => {
            match install_hint(&serial, &e) {
                Some(hint) => println!("  ✗ {serial}: {hint}"),
                None => println!("  ✗ {serial}: the demo app could not be installed: {e}"),
            }
            Some(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    #[test]
    fn written_files_are_the_repository_files() {
        for (name, text) in PAGES {
            assert_eq!(*text, std::fs::read_to_string(format!("{ROOT}/examples/demo/{name}")).expect("page"), "{name}");
        }
        assert_eq!(PAGES.len(), std::fs::read_dir(format!("{ROOT}/examples/demo")).expect("dir").filter(|e| e.as_ref().is_ok_and(|e| e.path().extension().is_some_and(|x| x == "html"))).count(), "every demo page is embedded");
        for (name, text, _) in TESTS {
            let repo = std::fs::read_to_string(format!("{ROOT}/tests/{name}")).expect("test");
            assert_eq!(*text, repo, "{name}");
            assert_eq!(test_case(text), repo.replace("examples/demo/", "demo/"));
            assert!(!test_case(text).contains("examples/"), "{name} points into the demo folder");
        }
        let apk = std::fs::read(format!("{ROOT}/examples/demo-android/out/fastpilot-demo.apk")).unwrap_or_default();
        assert_eq!(APK, apk.as_slice(), "the APK built into the binary is the demo app");
    }

    #[test]
    fn incompatible_install_says_how_to_fix_it() {
        let raw = "adb: pm install failed: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package com.fastpilot.demo signatures do not match newer version; ignoring!]";
        let hint = install_hint("emulator-5556", raw).expect("hint");
        assert!(hint.contains("adb -s emulator-5556 uninstall com.fastpilot.demo") && hint.contains("fastpilot demo --android --device emulator-5556"), "{hint}");
        assert!(!hint.contains("INSTALL_FAILED"), "no raw adb text: {hint}");
        assert!(install_hint("emulator-5556", "Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE]").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn writes_never_follow_a_symlink() {
        let root = std::env::temp_dir().join(format!("fp-safe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (base, outside) = (root.join("base"), root.join("outside"));
        std::fs::create_dir_all(&base).expect("base");
        std::fs::create_dir_all(&outside).expect("outside");
        // A folder on the way is a link: refused, nothing written outside.
        std::os::unix::fs::symlink(&outside, base.join("tests")).expect("link");
        let e = write_safe(&base, Path::new("tests/a.md"), b"x", true).expect_err("dir link");
        assert!(e.contains("symlink"), "{e}");
        assert!(!outside.join("a.md").exists());
        // The file itself is a link: refused, the target untouched.
        std::fs::write(outside.join("secret"), "keep").expect("secret");
        std::os::unix::fs::symlink(outside.join("secret"), base.join("f.md")).expect("link");
        assert!(write_safe(&base, Path::new("f.md"), b"x", true).is_err());
        assert_eq!(std::fs::read_to_string(outside.join("secret")).expect("secret"), "keep");
        // The base itself is a link (checked when asked).
        std::os::unix::fs::symlink(&outside, root.join("demo-link")).expect("link");
        assert!(write_safe(&root.join("demo-link"), Path::new("x"), b"x", true).is_err());
        // Plain folders: written, no temp file left.
        write_safe(&base, Path::new("a/b/c.md"), b"ok", true).expect("plain");
        assert_eq!(std::fs::read(base.join("a/b/c.md")).expect("c"), b"ok");
        assert_eq!(std::fs::read_dir(base.join("a/b")).expect("dir").count(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mcp_json_runs_fastpilot_with_the_demo_password() {
        let v: serde_json::Value = serde_json::from_str(&mcp_json()).expect("json");
        let s = &v["mcpServers"]["fastpilot"];
        assert_eq!(s["env"]["FASTPILOT_VARS"], "DEMO_PASS");
        assert_eq!(s["env"]["DEMO_PASS"], DEMO_PASS);
        assert!(s["args"].as_array().is_some_and(|a| a.last().is_some_and(|x| x == "mcp")));
    }

    /// T1-T3: `demo --android` followed the session rule for physical phones only after this.
    #[test]
    fn the_demo_app_goes_on_a_phone_only_when_phones_are_allowed() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe { std::env::set_var("FASTPILOT_ALLOW_PHYSICAL_DEVICES", "false") };
        assert!(physical_allowed("emulator-5556"));
        assert!(!physical_allowed("R58M12ABCDE"), "a phone while phones are off");
        unsafe { std::env::set_var("FASTPILOT_ALLOW_PHYSICAL_DEVICES", "true") };
        assert!(physical_allowed("R58M12ABCDE"));
        unsafe { std::env::remove_var("FASTPILOT_ALLOW_PHYSICAL_DEVICES") };
    }

}
