//! F3: `fastpilot setup ios` in the words of v9 G7, played with a fake `xcodebuild` on the PATH:
//! Xcode missing, not selected, license not accepted, a build that fails (with the log's tail),
//! and outside a terminal without --yes (nothing written, nothing built). macOS only.

use std::path::Path;

fn fake(dir: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).expect("dir");
    let p = dir.join("xcodebuild");
    std::fs::write(&p, format!("#!/bin/sh\n{script}\n")).expect("write");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn setup(home: &Path, bin: &Path, args: &[&str]) -> (Option<i32>, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"))
        .arg("setup")
        .arg("ios")
        .args(args)
        .env("HOME", home)
        .env_remove("FASTPILOT_CONFIG")
        .env_remove("FASTPILOT_IOS_RUNNER")
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("setup");
    (out.status.code(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn setup_ios_says_what_to_do() {
    if !cfg!(target_os = "macos") {
        return eprintln!("skipping: macOS only");
    }
    let root = std::env::temp_dir().join(format!("fp-f3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (home, bin) = (root.join("home"), root.join("bin"));
    std::fs::create_dir_all(&home).expect("home");

    fake(&bin, "exit 127");
    let (code, out) = setup(&home, &bin, &["--yes"]);
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("Setting up iPhone testing") && out.contains("Could not set up iPhone testing. Xcode is not installed.\nInstall it from the App Store, then run `fastpilot setup ios` again."), "{out}");

    fake(&bin, "echo \"xcode-select: error: tool 'xcodebuild' requires Xcode, but active developer directory '/Library/Developer/CommandLineTools' is a command line tools instance\" >&2; exit 1");
    let (_, out) = setup(&home, &bin, &["--yes"]);
    assert!(out.contains("sudo xcode-select -s /Applications/Xcode.app"), "{out}");

    // Xcode is there: outside a terminal and without --yes, the next step only.
    fake(&bin, "if [ \"$1\" = -version ]; then echo 'Xcode 99.0'; echo 'Build version 99A1'; exit 0; fi; echo 'You have not agreed to the Xcode license agreements.' >&2; exit 69");
    let (code, out) = setup(&home, &bin, &[]);
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("✓ Xcode found (Xcode 99.0)") && out.contains("add --yes"), "{out}");
    assert!(!home.join(".fastpilot/ios-runner").exists(), "nothing written");

    // --yes: the runner's project is written and built; a license problem says what to do.
    let (code, out) = setup(&home, &bin, &["--yes"]);
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("▶ Building the test runner") && out.contains("Xcode's license is not accepted yet"), "{out}");
    assert!(home.join(".fastpilot/ios-runner/src/ios-runner/FastpilotRunner.xcodeproj/project.pbxproj").exists() && home.join(".fastpilot/ios-runner/src/examples/demo-ios/DemoApp.swift").exists());

    // Any other failure: the last lines from Xcode and where the full log is.
    fake(&bin, "if [ \"$1\" = -version ]; then echo 'Xcode 99.0'; exit 0; fi; echo 'RunnerTests.swift:1: error: cannot find type Foo'; echo '** TEST BUILD FAILED **'; exit 65");
    let (code, out) = setup(&home, &bin, &["--yes"]);
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("The test runner did not build.") && out.contains("cannot find type Foo") && out.contains("ios-runner/build.log"), "{out}");
    let _ = std::fs::remove_dir_all(root);
}

/// V042 (from DV1): the first iPhone test of `fastpilot run` with no runner builds it once, with
/// progress on stderr against "about 5 minutes", then goes on; the next run builds nothing. A
/// schedule does not start the build: the test "could not run [setup]" and says to run one by hand
/// first. Xcode missing: the same plain words as `setup ios`. Stand-ins for xcodebuild, xcrun
/// (no simulators) and Claude Code; no real simulator or emulator is touched. macOS only.
#[test]
fn the_first_iphone_test_builds_the_runner_once() {
    if !cfg!(target_os = "macos") {
        return eprintln!("skipping: macOS only");
    }
    let root = std::env::temp_dir().join(format!("fp-v042-ios-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (bin, project) = (root.join("bin"), root.join("project"));
    std::fs::create_dir_all(project.join("tests")).expect("project");
    std::fs::write(project.join("tests/ios.md"), "# iPhone settings\nid: ios-settings\nplatform: ios\n- Open Settings and check that General is shown\n").expect("case");
    let calls = root.join("xcodebuild-calls");
    fake(
        &bin,
        &format!(
            "echo \"$@\" >> '{}'\nif [ \"$1\" = -version ]; then echo 'Xcode 99.0'; echo 'Build version 99A1'; exit 0; fi\nif [ \"$1\" = build-for-testing ]; then\n  while [ $# -gt 0 ]; do if [ \"$1\" = -derivedDataPath ]; then dd=\"$2\"; fi; shift; done\n  sleep 1; mkdir -p \"$dd/Build/Products\"; touch \"$dd/Build/Products/FastpilotRunner_iphonesimulator99.0-arm64.xctestrun\"; exit 0\nfi\nexit 1",
            calls.display()
        ),
    );
    {
        use std::os::unix::fs::PermissionsExt;
        let x = bin.join("xcrun");
        std::fs::write(&x, "#!/bin/sh\nif [ \"$1\" = simctl ]; then echo '{\"devices\":{}}'; exit 0; fi\nexit 1\n").expect("xcrun");
        std::fs::set_permissions(&x, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let run = |home: &Path, env: &[(&str, &str)]| {
        std::fs::create_dir_all(home).expect("home");
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fastpilot"));
        cmd.args(["run", "tests/ios.md"])
            .current_dir(&project)
            .env("HOME", home)
            .env("FASTPILOT_DIR", home.join("project-fastpilot"))
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("FASTPILOT_CLAUDE", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-claude/claude"))
            .env("FAKE_CLAUDE_LOG", home.join("claude-log"))
            .env("FASTPILOT_RETRY_BUDGET", "0");
        for k in ["FASTPILOT_CONFIG", "FASTPILOT_IOS_RUNNER", "FASTPILOT_SCHEDULED_AS", "FASTPILOT_RUN_BY_PERSON", "ANDROID_HOME", "ANDROID_SDK_ROOT", "ANDROID_SERIAL", "FASTPILOT_ANDROID_SERIAL", "FASTPILOT_ADB"] {
            cmd.env_remove(k);
        }
        cmd.envs(env.iter().copied());
        let out = cmd.stdin(std::process::Stdio::null()).output().expect("run");
        (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let builds = || std::fs::read_to_string(&calls).unwrap_or_default().lines().filter(|l| l.starts_with("build-for-testing")).count();

    // A person's run: built once, with progress, then the test goes on (here: no simulator is running).
    let home = root.join("home");
    let (_, out, err) = run(&home, &[]);
    assert!(err.contains("iPhone testing needs a one-time setup  (about 5 minutes)\nfastpilot builds a small helper app that taps and types on the\niPhone simulator. Your own apps are not touched."), "{err}");
    assert!(err.contains("  ✓ Xcode found\nPress Ctrl+C to stop. The test starts when this is done.") && err.contains("  ▶ Building the helper…") && err.contains("  ✓ Helper built. Starting the test."), "{err}");
    assert!(!err.contains('%') && !out.contains("could not run [setup]"), "never a percentage, and the test went on: {out}{err}");
    assert!(out.contains("1 tests, 1 workers, iPhone · 0 devices\n") && !out.contains("android") && !out.contains("emulator-55"), "an iPhone-only run never asks adb: {out}");
    assert_eq!(builds(), 1);
    assert!(home.join(".fastpilot/ios-runner/build/Build/Products/FastpilotRunner_iphonesimulator99.0-arm64.xctestrun").exists());
    // The next run: nothing to build.
    let (_, _, err) = run(&home, &[]);
    assert!(!err.contains("one-time setup"), "{err}");
    assert_eq!(builds(), 1, "built once");

    // A schedule: no 5-minute build; it says what to do, as a setup problem.
    let home = root.join("home-scheduled");
    let (code, out, _) = run(&home, &[("FASTPILOT_SCHEDULED_AS", "iPhone every morning")]);
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("✗ ios-settings could not run [setup]: Could not run: iPhone testing needs a one-time setup (about 5 minutes), and a schedule does not start it. Run one iPhone test by hand first"), "{out}");
    assert_eq!(builds(), 1, "a schedule builds nothing");
    // Run now on the Schedules page is a person's click: it may build.
    let (_, _, err) = run(&home, &[("FASTPILOT_SCHEDULED_AS", "iPhone every morning"), ("FASTPILOT_RUN_BY_PERSON", "1")]);
    assert!(err.contains("✓ Helper built"), "{err}");
    assert_eq!(builds(), 2);

    // No Xcode: the same plain words as `setup ios`, for the test.
    fake(&bin, "exit 127");
    let home = root.join("home-noxcode");
    let (code, out, err) = run(&home, &[]);
    assert_eq!(code, Some(1), "{out}{err}");
    assert!(err.contains("Could not set up iPhone testing. Xcode is not installed.\nInstall it from the App Store, then run the test again."), "{err}");
    assert!(out.contains("✗ ios-settings could not run [setup]: Could not run: iPhone testing could not be set up: Xcode is not installed."), "{out}");
    let _ = std::fs::remove_dir_all(root);
}
