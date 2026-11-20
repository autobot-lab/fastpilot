//! F3: `fastpilot setup ios` prepares iPhone testing without a fastpilot checkout (installs from npm
//! or a GitHub release), in the steps of v9 G7: Xcode found, test runner built, simulator started.
//!
//! How the runner gets there, and why: the runner's Xcode project (six small text files) is built
//! into this binary; `setup ios` writes it to `~/.fastpilot/ios-runner/src` and builds it once with
//! `xcodebuild build-for-testing` into `~/.fastpilot/ios-runner/build`, where the iOS driver looks
//! for it. The alternative, shipping a runner prebuilt by us in the release archive, was not taken:
//! an XCUITest runner is tied to the Xcode and iOS SDK that built it, so a prebuilt one breaks when
//! the user's Xcode is newer or older; it would also have to be signed and notarized to pass
//! Gatekeeper. Building on the user's Mac always matches their Xcode, needs nothing they do not
//! already need for iPhone simulators (Xcode), and takes a few minutes once. It is built again
//! when fastpilot or Xcode changes.

use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The runner's Xcode project, as in the repository (`ios-runner/`, and the demo app it builds from
/// `examples/demo-ios/`), at the same places relative to each other.
pub const SOURCES: &[(&str, &str)] = &[
    ("ios-runner/FastpilotRunner.xcodeproj/project.pbxproj", include_str!("../../../ios-runner/FastpilotRunner.xcodeproj/project.pbxproj")),
    (
        "ios-runner/FastpilotRunner.xcodeproj/xcshareddata/xcschemes/FastpilotRunner.xcscheme",
        include_str!("../../../ios-runner/FastpilotRunner.xcodeproj/xcshareddata/xcschemes/FastpilotRunner.xcscheme"),
    ),
    (
        "ios-runner/FastpilotRunner.xcodeproj/xcshareddata/xcschemes/FastpilotRunnerDevice.xcscheme",
        include_str!("../../../ios-runner/FastpilotRunner.xcodeproj/xcshareddata/xcschemes/FastpilotRunnerDevice.xcscheme"),
    ),
    ("ios-runner/FastpilotRunner/RunnerTests.swift", include_str!("../../../ios-runner/FastpilotRunner/RunnerTests.swift")),
    ("ios-runner/FastpilotHost/HostApp.swift", include_str!("../../../ios-runner/FastpilotHost/HostApp.swift")),
    ("examples/demo-ios/DemoApp.swift", include_str!("../../../examples/demo-ios/DemoApp.swift")),
];

const FAILED: &str = "Could not set up iPhone testing.";

/// What went wrong, in plain words, with the next step (G7).
pub fn explain(output: &str) -> &'static str {
    let o = output.to_ascii_lowercase();
    if o.contains("command line tools instance") || o.contains("requires xcode") {
        "Xcode is installed, but the command line tools are selected instead.\nRun `sudo xcode-select -s /Applications/Xcode.app`, then run `fastpilot setup ios` again."
    } else if o.contains("license") {
        "Xcode's license is not accepted yet.\nOpen Xcode once and accept it (or run `sudo xcodebuild -license accept`), then run `fastpilot setup ios` again."
    } else if o.contains("unable to find a destination") || o.contains("no ios simulator") || o.contains("platform is not installed") || o.contains("ios simulator runtime") {
        "Xcode has no iPhone simulator yet.\nOpen Xcode › Settings › Components, add an iOS simulator, then run `fastpilot setup ios` again."
    } else if o.contains("no space left") {
        "The disk is full.\nFree some space (the runner needs about 1 GB while it builds), then run `fastpilot setup ios` again."
    } else {
        "The test runner did not build."
    }
}

fn run(program: &str, args: &[&str]) -> (bool, String) {
    match std::process::Command::new(program).args(args).stdin(std::process::Stdio::null()).output() {
        Ok(o) => (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))),
        Err(e) => (false, e.to_string()),
    }
}

/// Built for this fastpilot and this Xcode.
fn stamp(xcode: &str) -> String {
    format!("fastpilot {} · {}", env!("CARGO_PKG_VERSION"), xcode.lines().collect::<Vec<_>>().join(" "))
}

fn write_sources(src: &Path) -> Res<()> {
    let _ = std::fs::remove_dir_all(src);
    for (rel, text) in SOURCES {
        let path = src.join(rel);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&path, text)?;
    }
    Ok(())
}

/// `fastpilot setup ios [--yes]`.
pub async fn setup(yes: bool) -> Res<()> {
    let fail = |why: &str| -> Res<()> {
        println!("\n{FAILED} {why}");
        std::process::exit(1);
    };
    if !cfg!(target_os = "macos") {
        return fail("iPhone simulators need a Mac with Xcode.\nWeb and Android tests work on this computer.");
    }
    println!("Setting up iPhone testing   (about 5 minutes, once)");
    let (ok, xcode) = run("xcodebuild", &["-version"]);
    if !ok {
        if xcode.to_ascii_lowercase().contains("command line tools") || xcode.to_ascii_lowercase().contains("requires xcode") {
            return fail(explain(&xcode));
        }
        return fail("Xcode is not installed.\nInstall it from the App Store, then run `fastpilot setup ios` again.");
    }
    println!("✓ Xcode found ({})", xcode.lines().next().unwrap_or("Xcode").trim());

    let home = fastpilot_ios::runner::setup_dir();
    let build = home.join("build");
    let marker = home.join("built-for");
    let ready = fastpilot_ios::runner::pick_xctestrun(&build.join("Build/Products"), fastpilot_ios::runner::Target::Simulator).is_some()
        && std::fs::read_to_string(&marker).is_ok_and(|m| m.trim() == stamp(&xcode));
    if ready {
        println!("✓ Test runner built (already, for this Xcode)");
    } else {
        if !yes {
            if !std::io::stdin().is_terminal() {
                println!("\nNext: build the test runner with your Xcode in {} (about 5 minutes, once).\nRun `fastpilot setup ios` in a terminal to be asked, or add --yes.", home.display());
                std::process::exit(1);
            }
            print!("Build the test runner now with your Xcode? It takes about 5 minutes, once. [Y/n] ");
            std::io::stdout().flush()?;
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            if !matches!(line.trim().to_lowercase().as_str(), "" | "y" | "yes" | "ya") {
                println!("Skipped. Nothing was changed. Run `fastpilot setup ios` when you are ready.");
                return Ok(());
            }
        }
        let src = home.join("src");
        if let Err(e) = write_sources(&src) {
            return fail(&format!("The runner's files could not be written to {} ({e}).", src.display()));
        }
        println!("▶ Building the test runner… (the long part)");
        std::io::stdout().flush()?;
        let project = src.join("ios-runner/FastpilotRunner.xcodeproj");
        let (project, build_s) = (project.to_string_lossy().into_owned(), build.to_string_lossy().into_owned());
        let (ok, log) = run(
            "xcodebuild",
            &["build-for-testing", "-project", &project, "-scheme", "FastpilotRunner", "-sdk", "iphonesimulator", "-destination", "generic/platform=iOS Simulator", "-derivedDataPath", &build_s, "-quiet"],
        );
        let log_file = home.join("build.log");
        let _ = std::fs::write(&log_file, &log);
        if !ok || fastpilot_ios::runner::pick_xctestrun(&build.join("Build/Products"), fastpilot_ios::runner::Target::Simulator).is_none() {
            let why = explain(&log);
            let tail: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
            let tail = tail[tail.len().saturating_sub(12)..].join("\n");
            let more = if why == "The test runner did not build." { format!("\nThe last lines from Xcode:\n{tail}") } else { String::new() };
            return fail(&format!("{why}{more}\nFull log: {}", log_file.display()));
        }
        let _ = std::fs::write(&marker, stamp(&xcode));
        println!("✓ Test runner built");
    }

    println!("▶ Starting an iPhone simulator…");
    std::io::stdout().flush()?;
    let sims = fastpilot_ios::devices().await.unwrap_or_default();
    let phones: Vec<&(String, String, String)> = sims.iter().filter(|(_, n, _)| n.starts_with("iPhone")).collect();
    if let Some((_, name, _)) = phones.iter().find(|(_, _, s)| s == "Booted") {
        println!("✓ {name} simulator is running");
    } else {
        let want = fastpilot_core::config::var("FASTPILOT_IOS_DEVICE").unwrap_or_else(|_| "iPhone 16".into());
        let Some((udid, name, _)) = phones.iter().find(|(_, n, _)| *n == want).or(phones.last()) else {
            return fail(explain("no ios simulator"));
        };
        let (ok, out) = run("xcrun", &["simctl", "boot", udid]);
        if !ok && !out.contains("current state: Booted") {
            return fail(&format!("The {name} simulator did not start.\n{}", out.trim()));
        }
        let _ = run("xcrun", &["simctl", "bootstatus", udid, "-b"]);
        println!("✓ {name} simulator is running");
    }
    println!("\niPhone testing is ready. In Claude, try: \"Use fastpilot to open Settings on the iPhone simulator and check that General is shown\"");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    /// Every file of `ios-runner/` the build needs is embedded, and the same as in the repository.
    #[test]
    fn the_runner_project_is_embedded_whole() {
        for (rel, text) in SOURCES {
            assert_eq!(*text, std::fs::read_to_string(format!("{ROOT}/{rel}")).expect("file"), "{rel}");
        }
        // Every folder the project points to outside ios-runner/ (`../examples/demo-ios`) is embedded.
        let pbx = SOURCES[0].1;
        for dir in pbx.split("path = \"../").skip(1).filter_map(|r| r.split('"').next()) {
            for e in std::fs::read_dir(format!("{ROOT}/{dir}")).expect("dir").flatten() {
                let rel = format!("{dir}/{}", e.file_name().to_string_lossy());
                assert!(SOURCES.iter().any(|(r, _)| *r == rel), "{rel} is used by the runner project but not embedded");
            }
        }
        let mut files = Vec::new();
        let mut stack = vec![PathBuf::from(format!("{ROOT}/ios-runner"))];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).expect("dir").flatten() {
                let p = e.path();
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if p.is_dir() {
                    if !matches!(name.as_str(), "build" | "build-device" | "xcuserdata" | "project.xcworkspace") {
                        stack.push(p);
                    }
                } else if !matches!(name.as_str(), "build.sh" | "PROTOCOL.md" | ".gitignore" | ".DS_Store") {
                    files.push(format!("ios-runner/{}", p.strip_prefix(format!("{ROOT}/ios-runner/")).expect("prefix").to_string_lossy()));
                }
            }
        }
        files.sort();
        let mut embedded: Vec<String> = SOURCES.iter().map(|(r, _)| (*r).to_owned()).filter(|r| r.starts_with("ios-runner/")).collect();
        embedded.sort();
        assert_eq!(files, embedded, "a new file in ios-runner/ must be added to SOURCES");
    }

    #[test]
    fn xcode_failures_in_plain_words() {
        assert!(explain("xcode-select: error: tool 'xcodebuild' requires Xcode, but active developer directory '/Library/Developer/CommandLineTools' is a command line tools instance").contains("xcode-select -s"));
        assert!(explain("You have not agreed to the Xcode license agreements.").contains("license"));
        assert!(explain("xcodebuild: error: Unable to find a destination matching the provided destination specifier").contains("Settings › Components"));
        assert_eq!(explain("** BUILD FAILED **"), "The test runner did not build.");
        assert!(stamp("Xcode 16.1\nBuild version 16B40").ends_with("Xcode 16.1 Build version 16B40"));
    }
}
