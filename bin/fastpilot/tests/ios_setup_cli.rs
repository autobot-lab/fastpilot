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
