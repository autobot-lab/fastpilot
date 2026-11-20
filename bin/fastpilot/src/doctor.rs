//! `fastpilot doctor [web|android|ios|all]`: is this machine ready, per platform, and what to do
//! about each gap. `fastpilot setup [...]`: the same checks, then each fix it can run, one by
//! one, only after the user says yes (nothing is installed without asking; without a terminal it
//! only prints the plan, `--yes` accepts every step).

use std::error::Error;
use std::io::{IsTerminal, Write};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// Works, with a limitation (no device attached, optional tool missing).
    Warn,
    /// This platform cannot run tests until fixed.
    Fail,
}

/// A command `setup` may run, after asking.
#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    pub what: String,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub area: &'static str,
    pub name: String,
    pub status: Status,
    pub detail: String,
    /// What a person should do (always present for Warn/Fail).
    pub hint: Option<String>,
    /// What `setup` can do itself.
    pub fixes: Vec<Fix>,
}

fn check(area: &'static str, name: &str, status: Status, detail: impl Into<String>) -> Check {
    Check { area, name: name.to_owned(), status, detail: detail.into(), hint: None, fixes: Vec::new() }
}

impl Check {
    fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }
    fn fix(mut self, what: &str, program: &str, args: &[&str]) -> Self {
        self.fixes.push(Fix { what: what.to_owned(), program: program.to_owned(), args: args.iter().map(|a| (*a).to_owned()).collect() });
        self
    }
}

/// Run `program args` with a timeout; trimmed stdout+stderr when it exited 0.
async fn output(program: &str, args: &[&str]) -> Option<String> {
    let run = tokio::process::Command::new(program).args(args).stdin(std::process::Stdio::null()).output();
    match tokio::time::timeout(Duration::from_secs(20), run).await {
        Ok(Ok(o)) if o.status.success() => Some(format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_owned()),
        _ => None,
    }
}

fn has(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

fn macos() -> bool {
    cfg!(target_os = "macos")
}

/// `!` for an endpoint on another host: nothing protects that connection on the network.
fn not_this_machine(area: &'static str, key: &str, host: &str, what: &str) -> Check {
    check(area, "remote", Status::Warn, format!("{key} points at {host}, not this machine: {what} has no password and no encryption"))
        .hint("reach it through an SSH tunnel and use 127.0.0.1 (README: device di mesin lain)")
}

pub async fn web() -> Vec<Check> {
    let mut out = Vec::new();
    if let Some(endpoint) = fastpilot_web::chrome_endpoint() {
        if let Some(host) = fastpilot_core::policy::remote_host(&endpoint) {
            out.push(not_this_machine("web", "chrome_endpoint", &host, "DevTools"));
        }
        let shown = fastpilot_core::policy::redact_userinfo(&endpoint);
        let connected = tokio::time::timeout(Duration::from_secs(10), fastpilot_web::Browser::connect(Some(&endpoint))).await;
        out.push(match connected {
            Ok(Ok(_)) => check("web", "Chrome", Status::Ok, format!("remote Chrome at {shown} (FASTPILOT_CHROME_ENDPOINT)")),
            Ok(Err(e)) => check("web", "Chrome", Status::Fail, format!("remote Chrome at {shown}: {}", fastpilot_core::policy::redact_userinfo_in(&e.to_string(), &endpoint)))
                .hint("start Chrome there with --remote-debugging-port and open the SSH tunnel (README: device di mesin lain)"),
            Err(_) => check("web", "Chrome", Status::Fail, format!("remote Chrome at {shown} did not answer within 10 s"))
                .hint("is the SSH tunnel open?"),
        });
        return out;
    }
    match fastpilot_web::find_chrome() {
        None => {
            let mut c = check("web", "Chrome", Status::Fail, "no Chrome or Chromium found").hint(if macos() {
                "install Google Chrome (https://www.google.com/chrome/), or set `fastpilot config set chrome /path/to/chrome`"
            } else {
                "install Chromium (e.g. `sudo apt install chromium`), or set `fastpilot config set chrome /path/to/chrome`"
            });
            if macos() && has("brew") {
                c = c.fix("install Google Chrome with Homebrew", "brew", &["install", "--cask", "google-chrome"]);
            }
            out.push(c);
        }
        Some(path) => {
            let launched = tokio::time::timeout(
                Duration::from_secs(25),
                fastpilot_web::Browser::launch_local(fastpilot_web::LaunchOptions { headless: true, executable: Some(path.clone()) }),
            )
            .await;
            out.push(match launched {
                Ok(Ok(_browser)) => check("web", "Chrome", Status::Ok, format!("{} (starts headless)", path.display())),
                Ok(Err(e)) => check("web", "Chrome", Status::Fail, format!("{} does not start: {e}", path.display()))
                    .hint("update Chrome, or point `chrome` to another build: `fastpilot config set chrome /path`"),
                Err(_) => check("web", "Chrome", Status::Fail, format!("{} did not start within 25 s", path.display()))
                    .hint("close other Chrome automation, then retry; or set another `chrome`"),
            });
        }
    }
    out
}

pub async fn android() -> Vec<Check> {
    let mut out = Vec::new();
    if let Some(server) = fastpilot_android::configured_server() {
        return android_remote(server).await;
    }
    let adb = fastpilot_android::sdk_tool("platform-tools/adb", "adb");
    let sdk_hint = if macos() {
        "install the Android SDK (Android Studio, or `brew install --cask android-commandlinetools`), then set ANDROID_HOME"
    } else {
        "install the Android SDK command-line tools and set ANDROID_HOME"
    };
    let Some(version) = output(&adb, &["version"]).await else {
        let mut c = check("android", "adb", Status::Fail, "Android SDK platform-tools (adb) not found").hint(sdk_hint);
        if macos() && has("brew") {
            c = c.fix("install Android platform-tools with Homebrew", "brew", &["install", "--cask", "android-platform-tools"]);
        }
        out.push(c);
        return out;
    };
    out.push(check("android", "adb", Status::Ok, version.lines().next().unwrap_or("adb").to_owned()));
    out.push(if fastpilot_android::agent_embedded() {
        check("android", "agent", Status::Ok, "UiAutomation agent built in")
    } else {
        check("android", "agent", Status::Fail, "this fastpilot build has no Android agent")
            .hint("use a release build (npm / install.sh), or from a checkout run agent-android/build.sh and rebuild")
    });
    let devices = fastpilot_android::device::list().await.unwrap_or_default();
    let ready: Vec<_> = devices.iter().filter(|d| d.state == "device").collect();
    let emulator = fastpilot_android::sdk_tool("emulator/emulator", "emulator");
    let avds: Vec<String> = output(&emulator, &["-list-avds"]).await.map(|s| s.lines().map(|l| l.trim().to_owned()).filter(|l| !l.is_empty() && !l.starts_with("INFO")).collect()).unwrap_or_default();
    if !ready.is_empty() {
        let names: Vec<String> = ready.iter().map(|d| format!("{} ({})", d.id, d.name)).collect();
        out.push(check("android", "device", Status::Ok, names.join(", ")));
    } else if let Some(avd) = avds.first() {
        out.push(
            check("android", "device", Status::Warn, format!("no device or emulator running; emulators: {}", avds.join(", ")))
                .hint(format!("start one: `{emulator} -avd {avd} -no-window &`, or connect a phone with USB debugging on"))
                .fix(&format!("start emulator {avd} in the background (no window)"), &emulator, &["-avd", avd, "-no-window", "-no-audio", "-no-snapshot-save"]),
        );
    } else {
        let abi = if cfg!(target_arch = "aarch64") { "arm64-v8a" } else { "x86_64" };
        let image = format!("system-images;android-35;google_apis;{abi}");
        let sdkmanager = fastpilot_android::sdk_tool("cmdline-tools/latest/bin/sdkmanager", "sdkmanager");
        let avdmanager = fastpilot_android::sdk_tool("cmdline-tools/latest/bin/avdmanager", "avdmanager");
        out.push(
            check("android", "device", Status::Warn, "no device, emulator or emulator image (AVD)")
                .hint("connect a phone with USB debugging on, or create an emulator (`fastpilot setup android`)")
                .fix("download the emulator and an Android 15 image (~1.5 GB)", &sdkmanager, &["emulator", &image])
                .fix("create the emulator fastpilot_api35", &avdmanager, &["create", "avd", "-n", "fastpilot_api35", "-k", &image, "-d", "pixel_6"]),
        );
    }
    out
}

/// Devices behind a configured adb server (another machine): no local SDK needed.
async fn android_remote(server: Result<String, String>) -> Vec<Check> {
    let server = match server {
        Ok(s) => s,
        Err(e) => return vec![check("android", "adb", Status::Fail, e)],
    };
    let warn = fastpilot_core::policy::remote_host(&server).map(|host| not_this_machine("android", "adb_server", &host, "adb"));
    let mut out: Vec<Check> = warn.into_iter().collect();
    out.push(match fastpilot_android::Adb::new().await {
        Ok(_) => check("android", "adb", Status::Ok, format!("adb server {server} (FASTPILOT_ADB_SERVER)")),
        Err(e) => {
            out.push(check("android", "adb", Status::Fail, e.to_string()).hint("open the SSH tunnel to the device machine (README: device di mesin lain)"));
            return out;
        }
    });
    out.push(if fastpilot_android::agent_embedded() {
        check("android", "agent", Status::Ok, "UiAutomation agent built in")
    } else {
        check("android", "agent", Status::Fail, "this fastpilot build has no Android agent")
            .hint("use a release build (npm / install.sh), or from a checkout run agent-android/build.sh and rebuild")
    });
    let devices = fastpilot_android::device::list().await.unwrap_or_default();
    let ready: Vec<String> = devices.iter().filter(|d| d.state == "device").map(|d| format!("{} ({})", d.id, d.name)).collect();
    out.push(if ready.is_empty() {
        check("android", "device", Status::Warn, format!("no device on the adb server {server}")).hint("start the emulator or connect the phone on that machine")
    } else {
        check("android", "device", Status::Ok, ready.join(", "))
    });
    out
}

pub async fn ios() -> Vec<Check> {
    if !macos() {
        return vec![check("ios", "Xcode", Status::Warn, "iOS needs a Mac with Xcode").hint("run fastpilot on a Mac for iOS (web and Android work here)")];
    }
    let mut out = Vec::new();
    let Some(xcode) = output("xcodebuild", &["-version"]).await else {
        out.push(
            check("ios", "Xcode", Status::Fail, "Xcode not found")
                .hint("install Xcode from the App Store, open it once, then: sudo xcode-select -s /Applications/Xcode.app"),
        );
        return out;
    };
    out.push(check("ios", "Xcode", Status::Ok, xcode.lines().next().unwrap_or("Xcode").to_owned()));
    match fastpilot_ios::devices().await {
        Ok(sims) => {
            let phones: Vec<_> = sims.iter().filter(|(_, n, _)| n.starts_with("iPhone") || n.starts_with("iPad")).collect();
            let booted: Vec<String> = phones.iter().filter(|(_, _, s)| s == "Booted").map(|(_, n, _)| n.clone()).collect();
            out.push(match (phones.is_empty(), booted.is_empty()) {
                (true, _) => check("ios", "simulator", Status::Fail, "no iPhone simulator installed")
                    .hint("Xcode > Settings > Components: install an iOS simulator runtime"),
                (false, true) => check("ios", "simulator", Status::Ok, format!("{} available (booted on first use)", phones.len())),
                (false, false) => check("ios", "simulator", Status::Ok, format!("booted: {}", booted.join(", "))),
            });
        }
        Err(e) => out.push(check("ios", "simulator", Status::Fail, format!("simctl failed: {e}")).hint("open Xcode once to finish its setup")),
    }
    out.push(match fastpilot_ios::runner::xctestrun(fastpilot_ios::runner::Target::Simulator) {
        Some(p) => check("ios", "runner", Status::Ok, p.display().to_string()),
        None => {
            let mut c = check("ios", "runner", Status::Fail, "iPhone test runner not set up").hint("fastpilot setup ios (builds it once with your Xcode, about 5 minutes)");
            if let Ok(exe) = std::env::current_exe() {
                c = c.fix("Build the iPhone test runner with your Xcode (about 5 minutes, once)", exe.to_string_lossy().as_ref(), &["setup", "ios", "--yes"]);
            }
            c
        }
    });
    out
}

/// `fastpilot run` drives the user's Claude Code.
pub async fn agent() -> Vec<Check> {
    let claude = fastpilot_core::config::var("FASTPILOT_CLAUDE").ok().filter(|c| !c.is_empty()).unwrap_or_else(|| "claude".into());
    vec![match output(&claude, &["--version"]).await {
        Some(v) => {
            let line = v.lines().next().unwrap_or("").to_owned();
            // QA B1: an old Claude Code started, so init said ✓, then every Chat message failed.
            match crate::run::claude_too_old(&line) {
                Some(why) => check("run", "Claude Code", Status::Fail, format!("{line}: too old")).hint(why),
                None => check("run", "Claude Code", Status::Ok, line),
            }
        }
        None => check("run", "Claude Code", Status::Warn, format!("`{claude}` not found (only `fastpilot run` needs it)"))
            .hint("install Claude Code (https://claude.com/claude-code) and log in once with `claude`"),
    }]
}

pub async fn checks(areas: &[&str]) -> Vec<Check> {
    let mut all = Vec::new();
    for a in areas {
        all.extend(match *a {
            "web" => web().await,
            "android" => android().await,
            "ios" => ios().await,
            "run" => agent().await,
            _ => Vec::new(),
        });
    }
    all
}

pub fn render(checks: &[Check]) -> String {
    let mut out = Vec::new();
    let mut area = "";
    for c in checks {
        if c.area != area {
            area = c.area;
            out.push(format!("{area}:"));
        }
        let mark = match c.status {
            Status::Ok => "✓",
            Status::Warn => "!",
            Status::Fail => "✗",
        };
        out.push(format!("  {mark} {:<12} {}", c.name, c.detail));
        if let (Some(h), false) = (&c.hint, c.status == Status::Ok) {
            out.push(format!("    → {h}"));
        }
    }
    let fixes = checks.iter().map(|c| c.fixes.len()).sum::<usize>();
    if fixes > 0 {
        out.push(format!("\n{fixes} fix(es) can be applied for you: fastpilot setup (asks before each)"));
    }
    out.join("\n")
}

fn areas(args: &[String]) -> Result<Vec<&'static str>, String> {
    let named: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if named.is_empty() || named.iter().any(|a| *a == "all") {
        return Ok(vec!["web", "android", "ios", "run"]);
    }
    named
        .iter()
        .map(|a| match a.as_str() {
            "web" => Ok("web"),
            "android" => Ok("android"),
            "ios" => Ok("ios"),
            "run" => Ok("run"),
            other => Err(format!("unknown platform `{other}` (web, android, ios, run, all)")),
        })
        .collect()
}

/// Failing exit status: a failed check in a platform the user named, or in web when none was named
/// (web is the one every install should have).
fn failed(checks: &[Check], args: &[String]) -> bool {
    let named = args.iter().any(|a| !a.starts_with("--") && a != "all");
    checks.iter().any(|c| c.status == Status::Fail && (named || c.area == "web"))
}

const DOCTOR_USAGE: &str = "usage: fastpilot doctor [web|android|ios|run|all]
  checks whether this computer can run tests, per platform, and says what to do about each gap.
  Nothing is changed. Exit code 1 when a platform you named (or web, when none is named) is not ready.";

const SETUP_USAGE: &str = "usage: fastpilot setup [web|android|ios] [--yes]
  runs the checks of `fastpilot doctor`, then each fix fastpilot can make itself (install Chrome or
  Android tools, create and start an emulator), asking before each one.
  --yes   agree to every step (also outside a terminal; without it and without a terminal, only the plan is shown)";

fn wants_help(args: &[String]) -> bool {
    args.iter().any(|a| matches!(a.as_str(), "-h" | "--help" | "help"))
}

pub async fn doctor(args: &[String]) -> Result<(), Box<dyn Error + Send + Sync>> {
    if wants_help(args) {
        println!("{DOCTOR_USAGE}");
        return Ok(());
    }
    let list = checks(&areas(args)?).await;
    println!("{}", render(&list));
    if failed(&list, args) {
        std::process::exit(1);
    }
    Ok(())
}

pub async fn setup(args: &[String]) -> Result<(), Box<dyn Error + Send + Sync>> {
    if wants_help(args) {
        println!("{SETUP_USAGE}");
        return Ok(());
    }
    let yes = args.iter().any(|a| a == "--yes" || a == "-y");
    // F3: `setup ios` builds the iPhone test runner itself (G7 steps), also without a checkout.
    if areas(args)? == ["ios"] {
        return crate::ios_setup::setup(yes).await;
    }
    let interactive = std::io::stdin().is_terminal();
    let list = checks(&areas(args)?).await;
    println!("{}\n", render(&list));
    let fixes: Vec<&Fix> = list.iter().flat_map(|c| c.fixes.iter()).collect();
    // A ✗ left after setup (nothing to fix it with, skipped, failed, or only planned) fails the run,
    // for the same areas as `doctor`, so CI does not go on with a platform that is not ready.
    if fixes.is_empty() {
        let manual = list.iter().filter(|c| c.status != Status::Ok).count();
        println!("{}", if manual == 0 { "nothing to set up" } else { "nothing fastpilot can install itself: follow the → hints above" });
        if failed(&list, args) {
            std::process::exit(1);
        }
        return Ok(());
    }
    if !interactive && !yes {
        println!("plan (run in a terminal to be asked for each step, or add --yes):");
        for f in &fixes {
            println!("  - {}: {} {}", f.what, f.program, f.args.join(" "));
        }
        if failed(&list, args) {
            std::process::exit(1);
        }
        return Ok(());
    }
    for f in fixes {
        let cmd = format!("{} {}", f.program, f.args.join(" "));
        if !yes {
            print!("{}?\n  {cmd}\n[y/N] ", f.what);
            std::io::stdout().flush()?;
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes" | "ya") {
                println!("  skipped");
                continue;
            }
        }
        // Emulators keep running: start them detached and move on.
        let detached = f.args.iter().any(|a| a == "-no-window");
        let mut command = std::process::Command::new(&f.program);
        command.args(&f.args);
        let status = if detached {
            command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map(|_| None)
        } else {
            command.status().map(Some)
        };
        match status {
            Ok(None) => println!("  started"),
            Ok(Some(s)) if s.success() => println!("  done"),
            Ok(Some(s)) => println!("  failed ({s}); see the output above"),
            Err(e) => println!("  could not run {}: {e}", f.program),
        }
    }
    // What is left after the fixes decides the exit status.
    let after = checks(&areas(args)?).await;
    let left: Vec<&Check> = after.iter().filter(|c| c.status != Status::Ok).collect();
    if left.is_empty() {
        println!("\nall set");
    } else {
        println!("\nstill to do:\n{}", render(&left.into_iter().cloned().collect::<Vec<_>>()));
    }
    if failed(&after, args) {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_and_decides_the_exit_status() {
        let list = vec![
            check("web", "Chrome", Status::Ok, "/x/chrome (starts headless)"),
            check("android", "adb", Status::Fail, "not found").hint("install the SDK").fix("install platform-tools", "brew", &["install", "x"]),
            check("ios", "runner", Status::Warn, "not built").hint("ios-runner/build.sh"),
        ];
        let text = render(&list);
        assert!(text.contains("web:\n  ✓ Chrome") && text.contains("  ✗ adb") && text.contains("→ install the SDK"), "{text}");
        assert!(text.contains("1 fix(es) can be applied"), "{text}");
        let none: Vec<String> = Vec::new();
        assert!(!failed(&list, &none), "android failing does not fail a plain `doctor`");
        assert!(failed(&list, &["android".to_owned()]));
        assert!(failed(&[check("web", "Chrome", Status::Fail, "")], &none));
        assert_eq!(areas(&none).expect("areas"), ["web", "android", "ios", "run"]);
        assert_eq!(areas(&["ios".to_owned(), "--yes".to_owned()]).expect("areas"), ["ios"]);
        assert!(areas(&["windows".to_owned()]).is_err());
    }
}
