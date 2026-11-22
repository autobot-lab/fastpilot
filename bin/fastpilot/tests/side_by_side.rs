//! PF5 "Cross-platform side by side", end to end: `fastpilot run tests/login-side-by-side.md`
//! (`platform: web, android, ios`) runs the demo login once per platform with the stand-in Claude
//! Code (`tests/fake-claude/claude`), which reports each numbered step with `step`.
//!
//! Web runs real flows in Chrome. Android and iPhone are stand-ins too, so no device is touched: a
//! fake adb server lists one emulator (`emulator-5590`, booted), and a fake `xcrun` on PATH lists
//! one booted iPhone simulator (or none). The stand-in reports the iPhone's link as "Lupa kata
//! sandi?" (the label planted in examples/demo-ios/DemoApp.swift) where web and Android say "Lupa
//! password?". Skips without Chrome or python3.

use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude");
const SIM: &str = "11111111-2222-3333-4444-555555555555";

fn ready() -> bool {
    let python = std::process::Command::new("python3").arg("--version").output().is_ok_and(|o| o.status.success());
    let chrome = ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().any(|p| Path::new(p).exists());
    python && chrome
}

/// A fake adb server: one booted emulator, `emulator-5590`; it answers the listing, the transport
/// and `getprop sys.boot_completed` only.
fn fake_adb() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    std::thread::spawn(move || {
        for sock in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut sock = sock;
                loop {
                    let mut len = [0u8; 4];
                    if sock.read_exact(&mut len).is_err() {
                        return;
                    }
                    let n = usize::from_str_radix(std::str::from_utf8(&len).unwrap_or("0"), 16).unwrap_or(0);
                    let mut req = vec![0u8; n];
                    if sock.read_exact(&mut req).is_err() {
                        return;
                    }
                    let req = String::from_utf8_lossy(&req).into_owned();
                    if req == "host:devices" {
                        let list = "emulator-5590\tdevice\n";
                        let _ = sock.write_all(format!("OKAY{:04x}{list}", list.len()).as_bytes());
                        return;
                    } else if req.starts_with("host:transport:") {
                        let _ = sock.write_all(b"OKAY");
                    } else if req.starts_with("shell:getprop sys.boot_completed") {
                        let _ = sock.write_all(b"OKAY1\n");
                        return;
                    } else {
                        let _ = sock.write_all(b"FAIL0004nope");
                        return;
                    }
                }
            });
        }
    });
    addr
}

/// A folder with a fake `xcrun`: `simctl list devices available -j` lists one booted iPhone (or
/// none); everything else fails. And a fake built iOS runner, so the run does not build one.
fn fake_xcode(dir: &Path, booted: bool) -> (PathBuf, PathBuf) {
    let bin = dir.join(if booted { "bin-sim" } else { "bin-nosim" });
    std::fs::create_dir_all(&bin).expect("bin");
    let devices = if booted { format!(r#"[{{"udid":"{SIM}","name":"iPhone 16","state":"Booted"}}]"#) } else { "[]".into() };
    let script = format!("#!/bin/sh\nif [ \"$1 $2 $3\" = \"simctl list devices\" ]; then echo '{{\"devices\":{{\"com.apple.CoreSimulator.SimRuntime.iOS-18-1\":{devices}}}}}'; exit 0; fi\nif [ \"$1 $2\" = \"simctl boot\" ] || [ \"$1 $2\" = \"simctl bootstatus\" ]; then exit 0; fi\nexit 1\n");
    let xcrun = bin.join("xcrun");
    std::fs::write(&xcrun, script).expect("xcrun");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&xcrun, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let runner = dir.join("ios-runner");
    std::fs::create_dir_all(runner.join("Build/Products")).expect("runner");
    std::fs::write(runner.join("Build/Products/FastpilotRunner_iphonesimulator18.1-arm64.xctestrun"), "").expect("xctestrun");
    (bin, runner)
}

struct Run {
    ok: bool,
    out: String,
    /// Every side-by-side row the Live page could have shown while it ran.
    live: Vec<Value>,
}

fn run(dir: &Path, booted: bool, html: &Path, env: &[(&str, &str)]) -> Run {
    run_target(dir, booted, "tests/login-side-by-side.md", html, env)
}

/// The same run for a test file of its own (the fake simulator is the booted one).
fn run_case_file(dir: &Path, case: &Path, html: &Path, env: &[(&str, &str)]) -> Run {
    run_target(dir, true, case.to_str().unwrap_or(""), html, env)
}

fn run_target(dir: &Path, booted: bool, target: &str, html: &Path, env: &[(&str, &str)]) -> Run {
    let (bin, runner) = fake_xcode(dir, booted);
    let live_dir = dir.join("live");
    let log = dir.join("claude");
    std::fs::create_dir_all(&log).expect("log");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
    cmd.args(["run", target, "--workers", "3"]).args(env.iter().filter(|(k, _)| *k == "--flag").map(|(_, v)| *v)).args([ "--html", html.to_str().unwrap_or("")])
        .current_dir(ROOT)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANDROID_SERIAL")
        .env_remove("FASTPILOT_ANDROID_SERIAL")
        .env_remove("FASTPILOT_IOS_UDID")
        .env("PATH", path)
        .env("FASTPILOT_CLAUDE", FAKE)
        .env("FAKE_CLAUDE_LOG", &log)
        .env("FASTPILOT_DIR", dir.join("fp"))
        .env("FASTPILOT_CONFIG", dir.join("config.toml"))
        .env("FASTPILOT_LIVE_DIR", &live_dir)
        .env("FASTPILOT_ADB_SERVER", fake_adb())
        .env("FASTPILOT_IOS_RUNNER", &runner)
        .env("FASTPILOT_RETRY_BUDGET", "0")
        .env("DEMO_PASS", "rahasia123")
        .env("FASTPILOT_VARS", "DEMO_PASS")
        .env("FAKE_CLAUDE_DELAY_MS", "150")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in env.iter().filter(|(k, _)| *k != "--flag") {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("run");
    let (mut so, mut se) = (child.stdout.take().expect("stdout"), child.stderr.take().expect("stderr"));
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = so.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = se.read_to_string(&mut s);
        s
    });
    // What the Live page reads: the run's own file in the live folder, while it runs.
    let mut live = Vec::new();
    let until = Instant::now() + Duration::from_secs(240);
    let status = loop {
        if let Some(st) = child.try_wait().expect("wait") {
            break st;
        }
        for e in std::fs::read_dir(&live_dir).into_iter().flatten().flatten() {
            if let Some(v) = std::fs::read_to_string(e.path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
                live.extend(v["side_by_side"].as_array().cloned().unwrap_or_default());
            }
        }
        if Instant::now() > until {
            let _ = child.kill();
            panic!("the run took too long");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = format!("{}{}", out.join().unwrap_or_default(), err.join().unwrap_or_default());
    Run { ok: status.success(), out, live }
}

/// Invocations of the stand-in Claude, by the platform of the run.
fn prompts(log: &Path) -> Vec<Value> {
    std::fs::read_dir(log).into_iter().flatten().flatten().filter_map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()).collect()
}

/// PF5 "Done when": the demo login on web, Android and iPhone produces a 3-column report, the
/// planted iOS-only label difference is highlighted, and Live shows one tile per platform.
#[test]
fn the_demo_login_side_by_side_highlights_the_planted_ios_label() {
    if !ready() {
        return eprintln!("skipping: needs Chrome and python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-pf5-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let html = dir.join("report.html");
    let r = run(&dir, true, &html, &[("FAKE_CLAUDE_SIDE_FOUND_IOS", "Masuk ke Akun|Lupa kata sandi?|Masuk|Budi")]);
    println!("{}", r.out);
    assert!(r.ok, "{}", r.out);
    // One run per platform, each on its own device, all passing.
    for (id, on) in [("login-side-by-side-web", ""), ("login-side-by-side-android", " on emulator-5590"), ("login-side-by-side-ios", &format!(" on {SIM}"))] {
        let line = r.out.lines().find(|l| l.starts_with(&format!("✓ {id} "))).unwrap_or_else(|| panic!("{id}: {}", r.out));
        assert!(line.contains(&format!("turns{on},")), "{line}");
    }
    assert!(r.out.contains("3 tests: 3 passed, 0 failed"), "{}", r.out);
    // The terminal says it in words.
    assert!(r.out.contains("Side by side · Login on web, Android and iPhone · 2 of 3 platforms match (1 platform differs · 4 steps)"), "{}", r.out);
    assert!(r.out.contains("  Web · Chrome: All 4 steps\n  Android emulator: All 4 steps\n  iPhone simulator: Differs at step 2\n"), "{}", r.out);
    assert!(r.out.contains("Step 2 · iPhone simulator shows \"Lupa kata sandi?\", the others show \"Lupa password?\"."), "{}", r.out);
    // Each run was told it is one platform of three, with the numbered steps, and offered `step`.
    let calls = prompts(&dir.join("claude"));
    let ios = calls.iter().find(|c| c["prompt"].as_str().is_some_and(|p| p.contains("Test case `login-side-by-side-ios`"))).expect("ios run");
    let prompt = ios["prompt"].as_str().unwrap_or("");
    assert!(prompt.contains("this test runs on Web · Chrome, Android emulator, iPhone simulator separately, and this run is the iPhone simulator one"), "{prompt}");
    assert!(prompt.contains("platform: ios\ntarget: com.fastpilot.demo\n") && !prompt.contains("login.html"), "{prompt}");
    assert!(prompt.contains("2. Check that there is a link for a forgotten password, and note its exact text"), "{prompt}");
    assert!(ios["tools"].as_array().is_some_and(|t| t.iter().any(|x| x == "step")), "{ios}");
    // The report: a real table, three platform columns, a row per step, the difference marked
    // on step 2 on iPhone in the row header and the cell, with the web pictures (real Chrome).
    let report = std::fs::read_to_string(&html).expect("report");
    assert!(report.contains("<caption>Each step of the test on Web · Chrome, Android emulator and iPhone simulator</caption>"), "{report}");
    assert!(report.contains("<th scope=\"col\">Step</th><th scope=\"col\">Web · Chrome</th><th scope=\"col\">Android emulator</th><th scope=\"col\">iPhone simulator</th>"));
    assert_eq!(report.matches("<th scope=\"row\">").count(), 4);
    assert!(report.contains("<tr class=\"differs\"><th scope=\"row\">2 · Check that there is a link for a forgotten password, and note its exact text<span class=\"lbl\">Differs on iPhone</span></th>"), "{report}");
    assert!(report.contains("<b>Passed</b> · Found &quot;Lupa kata sandi?&quot; instead of &quot;Lupa password?&quot;"), "{report}");
    assert_eq!(report.matches("<tr class=\"differs\">").count(), 1, "only the planted label differs");
    assert!(report.contains("<img alt=\"Web · Chrome, step 2: passed, &quot;Lupa password?&quot; found\" src=\"data:image/jpeg;base64,"), "a real picture per web step");
    assert!(report.contains("<b>2 of 3 platforms match</b>") && report.contains("Where they differ"), "{report}");
    // The web run kept its step pictures in its own run folder too.
    let runs: Vec<PathBuf> = std::fs::read_dir(dir.join("fp/runs")).expect("runs").flatten().map(|e| e.path()).collect();
    let web = runs.iter().find(|p| p.to_string_lossy().ends_with("login-side-by-side-web")).expect("web run");
    assert!((1..=4).all(|n| web.join(format!("step-{n}.jpg")).exists()), "{web:?}");
    // Live: one summary row with a tile per platform, each with its own state and step.
    let ids: Vec<&str> = r.live.iter().filter_map(|x| x["id"].as_str()).collect();
    assert!(!ids.is_empty() && ids.iter().all(|i| *i == "login-side-by-side"), "{:?}", r.live);
    let tiles = |p: &str| -> Vec<Value> { r.live.iter().filter_map(|x| x["tiles"].as_array()?.iter().find(|t| t["platform"] == p).cloned()).collect() };
    for (p, name) in [("web", "Web · Chrome"), ("android", "Android emulator"), ("ios", "iPhone simulator")] {
        let seen = tiles(p);
        assert!(seen.iter().all(|t| t["name"] == name && t["steps"] == 4), "{p}: {seen:?}");
        assert!(seen.iter().any(|t| t["state"] == "running" && t["step"].as_u64().is_some_and(|s| s >= 1) && t["label"].as_str().is_some_and(|l| !l.is_empty())), "{p} showed its step: {seen:?}");
    }
    assert_eq!(r.live[0]["title"], "Login on web, Android and iPhone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// No iPhone simulator: the iPhone column says "Not run: no iPhone simulator" with what to do,
/// never left out; the run is not a pass; Live shows the tile as not run from the start. A failure
/// on one platform only is marked as a difference.
#[test]
fn a_missing_simulator_is_a_not_run_column() {
    if !ready() {
        return eprintln!("skipping: needs Chrome and python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-pf5-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let html = dir.join("report.html");
    let r = run(&dir, false, &html, &[("FAKE_CLAUDE_SIDE_FAIL_ANDROID", "3")]);
    println!("{}", r.out);
    assert!(!r.ok, "{}", r.out);
    assert!(r.out.contains("✗ login-side-by-side-ios could not run [setup]: Not run: no iPhone simulator. Boot an iPhone simulator"), "{}", r.out);
    assert!(r.out.contains("Side by side · Login on web, Android and iPhone · 2 of 3 platforms ran"), "{}", r.out);
    assert!(r.out.contains("  iPhone simulator: Not run: No iPhone simulator"), "{}", r.out);
    assert!(r.out.contains("  Android emulator: Failed at step 3") && r.out.contains("Step 3 · fails on Android emulator, passes on Web · Chrome: field not found."), "{}", r.out);
    // Nobody started a session for the iPhone, and nothing asked to build its runner.
    assert!(prompts(&dir.join("claude")).iter().all(|c| !c["prompt"].as_str().unwrap_or("").contains("login-side-by-side-ios")));
    let report = std::fs::read_to_string(&html).expect("report");
    assert!(report.contains("<b>2 of 3 platforms ran</b>"), "{report}");
    assert_eq!(report.matches("<td class=\"cell none\"><p><b>Not run</b><br>No iPhone simulator<br><span class=\"next\">Boot an iPhone simulator").count(), 4, "{report}");
    assert!(report.contains("<td class=\"cell none\"><p><b>Not run</b><br>an earlier step failed</p></td>"), "Android step 4: {report}");
    let ios: Vec<&Value> = r.live.iter().filter_map(|x| x["tiles"].as_array()?.iter().find(|t| t["platform"] == "ios")).collect();
    assert!(!ios.is_empty() && ios.iter().all(|t| t["state"] == "not_run" && t["note"] == "No iPhone simulator"), "{ios:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// QA re-test Low 3: platforms whose device session does not start are "Not run" with the reason,
/// infrastructure, and never make the passing platform "differ".
#[test]
fn a_platform_that_does_not_start_is_not_run() {
    if !ready() {
        return eprintln!("skipping: needs Chrome and python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-pf5-nostart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let html = dir.join("report.html");
    let r = run(&dir, true, &html, &[("FAKE_CLAUDE_SIDE_NOSTART_ANDROID", "1"), ("FAKE_CLAUDE_SIDE_NOSTART_IOS", "1")]);
    println!("{}", r.out);
    assert!(!r.out.contains("[product]"), "a start failure is not a product failure: {}", r.out);
    assert!(r.out.contains("[infra]: Could not run: the Android emulator session did not start") && r.out.contains("[infra]: Could not run: the iPhone simulator session did not start"), "{}", r.out);
    assert!(r.out.contains("1 of 3 platforms ran") && !r.out.contains("differs"), "{}", r.out);
    let report = std::fs::read_to_string(&html).expect("report");
    assert!(report.contains("<b>1 of 3 platforms ran</b>") && !report.contains("Differs on"), "{report}");
    assert!(report.matches("<td class=\"cell none\"><p><b>Could not run</b><br>The Android emulator session did not start").count() == 4, "{report}");
    assert!(report.matches("<b>Could not run</b><br>The iPhone simulator session did not start").count() == 4, "{report}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// v0.6.0 (QA Tester, bug 2): the iPhone runner never comes up (a busy computer; here a fake
/// `xcodebuild` that prints nothing and sleeps). The run must not hang or retry the start again
/// and again: web passes, the iPhone column says "Infrastructure: iPhone runner did not start
/// (machine busy?)" and counts as Not run (not as a product failure), and the command prints the
/// summary and exits promptly. Before the fix the start waited a fixed 180 s, and the suite
/// retried it (retry budget 3), so the command was silent for about nine minutes.
#[test]
fn an_iphone_runner_that_never_starts_is_not_run_and_the_command_exits() {
    if !ready() {
        return eprintln!("skipping: needs Chrome and python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-pf5-slow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let (bin, _) = fake_xcode(&dir, true);
    // A runner that never says it is ready, and stays up until it is killed.
    let xcodebuild = bin.join("xcodebuild");
    std::fs::write(&xcodebuild, "#!/bin/sh\necho 'Test Suite started'\nexec sleep 600\n").expect("xcodebuild");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&xcodebuild, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let case = dir.join("web-and-iphone.md");
    std::fs::write(
        &case,
        "# Login on web and iPhone\nid: web-and-iphone\nplatform: web, ios\ntarget web: examples/demo/login.html\ntarget ios: com.fastpilot.demo\n\
         - Open the login screen and check that \"Masuk ke Akun\" shows\n- Check that there is a link for a forgotten password, and note its exact text\n",
    )
    .expect("case");
    let html = dir.join("report.html");
    let started = Instant::now();
    let r = run_case_file(&dir, &case, &html, &[("FASTPILOT_IOS_START_TIMEOUT", "3"), ("FASTPILOT_PROGRESS_SECS", "1"), ("FASTPILOT_RETRY_BUDGET", "3"), ("FAKE_CLAUDE_SIDE_REAL", "1")]);
    println!("{}", r.out);
    assert!(started.elapsed() < Duration::from_secs(90), "the run took {:?}", started.elapsed());
    assert!(!r.ok, "a platform that did not run is not a pass: {}", r.out);
    // Web passed; the iPhone did not run, said why in these words, and was started once only.
    assert!(r.out.lines().any(|l| l.starts_with("✓ web-and-iphone-web ")), "{}", r.out);
    let ios = r.out.lines().find(|l| l.starts_with("✗ web-and-iphone-ios ")).unwrap_or_else(|| panic!("{}", r.out));
    assert!(ios.contains("Infrastructure: iPhone runner did not start (machine busy?)") && !ios.contains("[product]") && !ios.contains("attempts"), "{ios}");
    assert_eq!(prompts(&dir.join("claude")).iter().filter(|c| c["prompt"].as_str().is_some_and(|p| p.contains("Test case `web-and-iphone-ios`"))).count(), 1, "the iPhone test is not retried");
    // Progress while it waited, then the summary: Not run, not failed.
    assert!(r.out.contains("web-and-iphone-ios: still starting the iPhone runner (1 s;"), "{}", r.out);
    assert!(r.out.contains("1 passed, 0 failed, 1 did not run"), "{}", r.out);
    assert!(r.out.contains("not a failure of your app"), "{}", r.out);
    assert!(r.out.contains("iPhone simulator: Could not run: Infrastructure: iPhone runner did not start (machine busy?)"), "{}", r.out);
    let report = std::fs::read_to_string(&html).expect("report");
    assert!(report.contains("<b>Could not run</b><br>Infrastructure: iPhone runner did not start (machine busy?)"), "{report}");
    // No xcodebuild left behind holding the simulator.
    let left = std::process::Command::new("pgrep").args(["-f", &format!("{}/xcodebuild", dir.display())]).output().map(|o| o.stdout.len()).unwrap_or(0);
    assert_eq!(left, 0, "the fake xcodebuild was left running");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The simulator is too busy to open the runner ("Busy ... failed preflight checks", seen at a load
/// average above 60): the start is tried again, three times at most, and then reported as "Not run"
/// in the same words, not as a failure of the app.
#[test]
fn a_busy_simulator_gets_three_tries_and_then_says_so() {
    if !ready() {
        return eprintln!("skipping: needs Chrome and python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-pf5-busy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let (bin, _) = fake_xcode(&dir, true);
    let calls = dir.join("xcodebuild-calls");
    let xcodebuild = bin.join("xcodebuild");
    std::fs::write(
        &xcodebuild,
        format!(
            "#!/bin/sh\necho x >> {}\necho 'Testing failed:'\necho 'The request was denied by service delegate (SBMainWorkspace) for reason: Busy (\"Application failed preflight checks\")' >&2\nexit 65\n",
            calls.display()
        ),
    )
    .expect("xcodebuild");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&xcodebuild, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let case = dir.join("web-and-iphone.md");
    std::fs::write(
        &case,
        "# Login on web and iPhone\nid: web-and-iphone\nplatform: web, ios\ntarget web: examples/demo/login.html\ntarget ios: com.fastpilot.demo\n\
         - Open the login screen and check that \"Masuk ke Akun\" shows\n- Check that there is a link for a forgotten password, and note its exact text\n",
    )
    .expect("case");
    let r = run_case_file(&dir, &case, &dir.join("report.html"), &[("FASTPILOT_RETRY_BUDGET", "3"), ("FAKE_CLAUDE_SIDE_REAL", "1")]);
    println!("{}", r.out);
    assert!(!r.ok, "{}", r.out);
    let ios = r.out.lines().find(|l| l.starts_with("✗ web-and-iphone-ios ")).unwrap_or_else(|| panic!("{}", r.out));
    assert!(ios.contains("Infrastructure: iPhone runner did not start (machine busy?)") && !ios.contains("[product]"), "{ios}");
    assert_eq!(std::fs::read_to_string(&calls).unwrap_or_default().lines().count(), 3, "three tries, not more");
    assert!(r.out.contains("1 did not run"), "{}", r.out);
    let _ = std::fs::remove_dir_all(&dir);
}

/// S8 "Run on all platforms": a saved test written for the website only runs on every platform its
/// app has (E6 `targets`), side by side, without editing the file. An app without an iPhone app
/// compares web and Android and says how to add the iPhone one. Stand-in Claude, adb and simctl.
#[test]
fn a_saved_test_runs_on_all_platforms_of_its_app() {
    if !ready() {
        return eprintln!("skipping: needs Chrome and python3");
    }
    let dir = std::env::temp_dir().join(format!("fastpilot-s8-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home");
    let tests = home.join("fastpilot/apps/demo/tests");
    std::fs::create_dir_all(&tests).expect("dir");
    let apps = |ios: bool| {
        let ios = if ios { r#", "ios": "com.fastpilot.demo""# } else { "" };
        format!(r#"{{ "current": "demo", "apps": [ {{ "id": "demo", "name": "Demo shop", "targets": {{ "web": "examples/demo/login.html", "android": "com.fastpilot.demo/.LoginActivity"{ios} }}, "repo": null, "created_ms": 1 }} ] }}"#)
    };
    std::fs::write(home.join("fastpilot/apps.json"), apps(true)).expect("apps");
    // As Chat saves a website test: one platform, one target, no platform list.
    let side = std::fs::read_to_string(format!("{ROOT}/tests/login-side-by-side.md")).expect("case");
    let saved: String = side
        .lines()
        .filter(|l| !l.starts_with("platform:") && !l.starts_with("target "))
        .map(|l| if l.starts_with("id:") { "id: login-saved\ntarget: examples/demo/login.html".to_owned() } else { l.to_owned() })
        .collect::<Vec<_>>()
        .join("\n");
    let file = tests.join("login-saved.md");
    std::fs::write(&file, &saved).expect("test");
    let html = dir.join("report.html");
    let home_s = home.display().to_string();
    let env = [("HOME", home_s.as_str()), ("--flag", "--all-platforms"), ("FAKE_CLAUDE_SIDE_FOUND_IOS", "Masuk ke Akun|Lupa kata sandi?|Masuk|Budi")];
    let r = run_case_file(&dir, &file, &html, &env);
    println!("{}", r.out);
    assert!(r.ok, "{}", r.out);
    for id in ["login-saved-web", "login-saved-android", "login-saved-ios"] {
        assert!(r.out.lines().any(|l| l.starts_with(&format!("✓ {id} "))), "{id}: {}", r.out);
    }
    assert!(r.out.contains("2 of 3 platforms match (1 platform differs · 4 steps)") && r.out.contains("iPhone simulator shows \"Lupa kata sandi?\""), "{}", r.out);
    assert_eq!(std::fs::read_to_string(&file).expect("test"), saved, "the saved file is not edited");
    let report = std::fs::read_to_string(&html).expect("report");
    assert!(report.contains("<th scope=\"col\">Web · Chrome</th><th scope=\"col\">Android emulator</th><th scope=\"col\">iPhone simulator</th>"), "{report}");

    // The app has no iPhone app: web and Android only, and the run says what to add.
    std::fs::write(home.join("fastpilot/apps.json"), apps(false)).expect("apps");
    let r = run_case_file(&dir, &file, &html, &env);
    println!("{}", r.out);
    assert!(r.out.contains("iPhone simulator is not in this comparison: Demo shop has no iPhone yet (Add iPhone app)."), "{}", r.out);
    assert!(r.out.contains("2 tests: 2 passed") && !r.out.contains("login-saved-ios"), "{}", r.out);

    // Without an app: a plain message, nothing runs.
    std::fs::remove_file(home.join("fastpilot/apps.json")).expect("rm");
    let r = run_case_file(&dir, &file, &html, &env);
    assert!(!r.ok && r.out.contains("Run on all platforms needs an app: add one with the App switcher"), "{}", r.out);
    let _ = std::fs::remove_dir_all(&dir);
}
