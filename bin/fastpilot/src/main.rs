//! fastpilot: single-binary MCP server for AI-driven QA on web (Android/iOS later).

mod approve;
mod bench;
mod chat;
mod cli;
mod demo;
mod doctor;
mod fix;
mod history_api;
mod memory_api;
mod init;
mod ios_setup;
mod github;
mod keystore;
mod provider;
mod pool;
mod report;
mod run;
mod schedule;
mod stability;
mod ui;

/// Tests that change the process environment hold this.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

use std::error::Error;

use fastpilot_web::LaunchOptions;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: fastpilot <command> [--headful]

commands:
  doctor [web|android|ios|run|all]   is this machine ready? each gap with its fix
  setup [web|android|ios] [--yes]    apply the fixes doctor found, asking before each
  config [list|get K|set K V|unset K|path]   settings in ~/.fastpilot/config.toml (env wins)
  config approve                    approve the limits the AI asked to loosen (a person, in a terminal)
  config set-key|unset-key <provider>        API key of an OpenAI-compatible provider (read from stdin; Keychain)
  config test-provider                       one small request with the configured provider
  init [--yes] [--global]            set up this folder: register fastpilot with Claude Code, install the
                                     Skill, add tests/contoh.md, show the limits (asks before each change)
  demo [folder] [--android]          demo pages, example tests and .mcp.json in a new folder: a first test
                                     without the repository (--android: install the demo app)
  ui [--no-open] [--port N]          local settings and device page in the browser (127.0.0.1, one-time link)
  schedule [list|run ID|remove ID]   scheduled runs (made in `fastpilot ui` › Schedules; run by launchd/systemd/cron)
  open, snapshot, click, fill, ...   shell CLI: one command per step, sessions kept by a background
                                     daemon (`fastpilot help` lists them all)
  mcp     serve MCP over stdio (what MCP clients run); --reset: mobile sessions reset their app first
  bench [web|android|ios]   print snapshot and tap->diff latency against the demo login
          [--iterations N]      (android: booted device with examples/demo-android installed)
  bench tokens [web|android|ios]   tokens per snapshot, per 1-step diff and per login run_flow
                                   (offline approximation)
  stability <flow.yaml> [--runs N]   repeat a flow in fresh sessions: pass rate, pass^k,
                                     95% upper bound of the failure rate
  ios doctor [udid]   why a physical iPhone can or cannot run tests yet (Xcode, signing,
                      Developer Mode, pairing, profile expiry), each with the fix
  run <tests dir|file.md> [--workers N] [--record] [--reset] [--report-github[=dry]] [--model ID] [--max-steps N] [--junit out.xml] [--html out.html]
          headless: your Claude Code (`claude -p`, your login, no API key) drives each
          natural-language test case over the same tools; exits non-zero when a test
          outside quarantine fails (FASTPILOT_CLAUDE: path to the claude CLI)

env:
  FASTPILOT_CHROME   Chrome/Chromium executable
  FASTPILOT_HEADFUL  1 to show the browser window
  FASTPILOT_LOG      tracing filter, e.g. debug (logs go to stderr)";

/// `fastpilot config [list | get KEY | set KEY VALUE | unset KEY | path]`: the user's own
/// settings (no guard-rail limits: people decide).
fn config_cmd(args: &[String]) -> Result<(), Box<dyn Error + Send + Sync>> {
    use fastpilot_core::config;
    let key = || args.get(1).map(|k| k.to_lowercase()).ok_or("missing setting key; see `fastpilot config list`");
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" => {
            println!("{}", config::describe());
            // The provider's key: whether there is one and where, never the key.
            if let Some(p) = config::var("FASTPILOT_PROVIDER").ok().filter(|p| !p.is_empty() && p != "claude") {
                let state = if keystore::has(&p) { format!("set ({})", keystore::backend().describe()) } else { "not set: `fastpilot config set-key <provider>`".into() };
                println!("{:<18} {state}", format!("{p} API key"));
            }
        }
        "set-key" => {
            let p = args.get(1).ok_or("set-key needs a provider: openai, openrouter, groq, ollama or custom")?;
            let key = read_secret(&format!("API key for {p} (input hidden): "))?;
            let place = keystore::set(p, &key)?;
            println!("{p} API key saved in the {place}; it is never shown again (`fastpilot config test-provider` checks it)");
        }
        "unset-key" => {
            let p = args.get(1).ok_or("unset-key needs a provider")?;
            keystore::delete(p)?;
            println!("{p} API key removed from the {}", keystore::backend().describe());
        }
        "path" => println!("{}", config::path().display()),
        "get" => {
            let k = key()?;
            config::setting(&k).ok_or(format!("unknown setting `{k}`"))?;
            match config::effective(&k) {
                config::Source::Env(v) => println!("{v}  (env FASTPILOT_{})", k.to_uppercase()),
                config::Source::File(v) if v.is_empty() && config::LISTS.contains(&k.as_str()) => println!("[] (emptied)"),
                config::Source::File(v) => println!("{}", config::shown(&k, &v)),
                // A provider's preset base URL is a real value, not "nothing set".
                config::Source::Default if k == "provider_base_url" => {
                    let p = config::var("FASTPILOT_PROVIDER").unwrap_or_default();
                    match config::preset_base(p.trim()) {
                        Some(url) => println!("{url}  (preset of {})", p.trim()),
                        None => println!("(default)"),
                    }
                }
                config::Source::Default => match k.as_str() {
                    // v0.3.2: what "not set" means for the built-in lists.
                    "block_actions" => println!("{} (default)", fastpilot_core::policy::DEFAULT_BLOCKED.join(", ")),
                    "confirm_actions" => println!("{} (default)", fastpilot_core::policy::DEFAULT_CONFIRM.join(", ")),
                    _ => println!("(default)"),
                },
            }
        }
        "set" => {
            let k = key()?;
            // `""` empties a built-in list on purpose (v0.3.2); `unset` brings the built-in list back.
            let emptied = matches!(k.as_str(), "block_actions" | "confirm_actions") && args.get(2..).is_some_and(|r| r.len() == 1 && r[0].trim().is_empty());
            let v = args
                .get(2..)
                .map(|r| r.join(" "))
                .filter(|v| emptied || !v.trim().is_empty())
                .ok_or_else(|| format!("set needs a value; to clear `{k}`, use `fastpilot config unset {k}`"))?;
            let previous = (k == "provider").then(|| config::file_values().remove("provider")).flatten();
            let p = config::write(&k, Some(&v))?;
            println!("{k} = {}  ({})", if v.trim().is_empty() { "[]" } else { v.as_str() }, p.display());
            // Another provider starts from its own preset: the previous one's base URL would get
            // the new provider's key.
            if k == "provider" && previous.as_deref() != Some(v.trim()) && config::file_values().contains_key("provider_base_url") {
                config::write("provider_base_url", None)?;
                match config::preset_base(v.trim()) {
                    Some(url) => println!("note: provider_base_url removed; {v} uses {url}"),
                    None => println!("note: provider_base_url removed; set the server: `fastpilot config set provider_base_url https://…/v1`"),
                }
            }
            if std::env::var(format!("FASTPILOT_{}", k.to_uppercase())).is_ok() {
                println!("note: FASTPILOT_{} is set in the environment and wins over the file", k.to_uppercase());
            }
            if config::setting(&k).is_some_and(|s| s.rule == config::Rule::TightenOnly) {
                println!("note: a guard rail; a running fastpilot (MCP server, daemon) only takes a stricter value, a looser one after restart");
            }
            if config::setting(&k).is_some_and(|s| s.rule == config::Rule::HumanOnly) {
                println!("note: a running fastpilot (MCP server, daemon) keeps its value until it restarts");
            }
        }
        "unset" => {
            let k = key()?;
            let p = config::write(&k, None)?;
            println!("{k} removed  ({})", p.display());
        }
        // D9, M13: requests the AI filed to loosen a limit, approved by a person at the keyboard.
        "approve" => approve::run()?,
        other => return Err(format!("unknown `config {other}` (list, get, set, unset, path, set-key, unset-key, test-provider, approve)").into()),
    }
    Ok(())
}

/// Run a command that hosts sessions while publishing them to the Live panel of `fastpilot ui`.
async fn live<F, Fut, T>(f: F, kind: &str) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    fastpilot_core::live::enable(kind);
    let out = f().await;
    fastpilot_core::live::disable();
    out
}

/// A secret from stdin: hidden while typed in a terminal, else the piped text (`< keyfile`). Never
/// from argv, where other processes could read it.
fn read_secret(prompt: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    use std::io::{BufRead, IsTerminal, Read};
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        eprint!("{prompt}");
        let _ = std::process::Command::new("stty").arg("-echo").stdin(std::process::Stdio::inherit()).status();
        let mut line = String::new();
        let read = stdin.lock().read_line(&mut line);
        let _ = std::process::Command::new("stty").arg("echo").stdin(std::process::Stdio::inherit()).status();
        eprintln!();
        read?;
        Ok(line.trim().to_owned())
    } else {
        let mut all = String::new();
        stdin.lock().read_to_string(&mut all)?;
        Ok(all.trim().to_owned())
    }
}

#[tokio::main]
async fn main() {
    // An error for people in the terminal: one plain line, not Rust's `Error: "…"`.
    if let Err(e) = run_main().await {
        eprintln!("fastpilot: {e}");
        std::process::exit(1);
    }
}

async fn run_main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // stdout belongs to the MCP transport; logs always go to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::try_from_env("FASTPILOT_LOG").unwrap_or_else(|_| EnvFilter::new("warn")))
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    // Started by Chrome as the native messaging host of the fastpilot extension (relay token).
    if let Some(origin) = args.first().filter(|a| a.starts_with("chrome-extension://")) {
        return Ok(fastpilot_web::relay_native_host(origin)?);
    }
    let flag = |name: &str| args.iter().any(|a| a == name);
    // A config file that does not parse would switch the guard rails off: stop instead
    // (`config path` still answers, so the user finds the file, and `ui` shows the error).
    let path_only = args.first().is_some_and(|a| a == "config") && args.get(1).is_some_and(|a| a == "path");
    let ui = args.first().is_some_and(|a| a == "ui");
    if !path_only && !ui && let Err(e) = fastpilot_core::config::check() {
        eprintln!("fastpilot: {e}");
        std::process::exit(1);
    }
    // The guard rails in force for this whole process are the ones in the file now (or stricter).
    fastpilot_core::config::pin_guard_rails();
    let headful = flag("--headful") || fastpilot_core::config::flag("headful");
    let web = LaunchOptions { headless: !headful, executable: None };

    match args.first().map(String::as_str) {
        Some("mcp") => live(|| fastpilot_mcp::serve_stdio(web, flag("--reset")), "mcp").await,
        Some("config") if args.get(1).is_some_and(|a| a == "test-provider") => match provider::test_provider().await {
            Ok(msg) => {
                println!("{msg}");
                Ok(())
            }
            Err(e) => Err(e.into()),
        },
        Some("config") => config_cmd(&args[1..]),
        Some("doctor") => doctor::doctor(&args[1..]).await,
        Some("ui") => ui::ui(&args[1..]).await,
        Some("schedule") => schedule::cli(&args[1..]).await,
        Some("demo") => demo::demo(&args[1..]).await,
        Some("init") => init::init(&args[1..]).await,
        Some("setup") => doctor::setup(&args[1..]).await,
        Some("daemon") if args.len() == 1 => live(|| cli::daemon(web), "daemon").await,
        _ if cli::command_of(&args).is_some() => {
            // Blocking client: one request to the daemon (started on demand).
            let argv = args.clone();
            tokio::task::spawn_blocking(move || cli::main(&argv)).await?
        }
        // Internal: the stdio MCP server Claude Code spawns during `fastpilot run`.
        Some("mcp-bridge") => match (args.get(1), args.get(2)) {
            (Some(addr), Some(token)) => run::bridge(addr, token).await,
            _ => {
                eprintln!("usage: fastpilot mcp-bridge <addr> <token> (started by `fastpilot run`)");
                std::process::exit(2);
            }
        },
        Some("bench") => {
            let iterations = args
                .iter()
                .position(|a| a == "--iterations")
                .and_then(|i| args.get(i + 1))
                .map(|n| n.parse::<usize>())
                .transpose()?
                .unwrap_or(30);
            match args.get(1).map(String::as_str) {
                Some("tokens") => bench::tokens(args.get(2).filter(|a| !a.starts_with("--")).map_or("web", String::as_str), web).await,
                Some("android") => bench::android(iterations.max(1)).await,
                Some("ios") => bench::ios(iterations.max(1)).await,
                _ => bench::run(web, iterations.max(1)).await,
            }
        }
        Some("stability") => {
            let Some(path) = args.get(1).filter(|a| !a.starts_with("--")) else {
                eprintln!("usage: fastpilot stability <flow.yaml> [--runs N] [--headful]");
                std::process::exit(2);
            };
            let runs = args
                .iter()
                .position(|a| a == "--runs")
                .and_then(|i| args.get(i + 1))
                .map(|n| n.parse::<usize>())
                .transpose()?
                .unwrap_or(20);
            stability::run(path, runs.max(1), web).await
        }
        Some("run") => {
            const RUN_USAGE: &str = "usage: fastpilot run <tests dir|file.md> [--workers N] [--record] [--reset] [--report-github[=dry]] [--model ID] [--max-steps N] [--junit out.xml] [--html out.html] [--headful]";
            let Some(path) = args.get(1).filter(|a| !a.starts_with("--")) else {
                eprintln!("{RUN_USAGE}");
                std::process::exit(2);
            };
            // Only the options `run` knows: `--reset=1` or a typo is an error, not silently ignored.
            let bad = |msg: String| -> ! {
                eprintln!("{msg}\n{RUN_USAGE}");
                std::process::exit(2);
            };
            let mut rest = args.iter().skip(2);
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--workers" | "--model" | "--max-steps" | "--junit" | "--html" => {
                        if rest.next().is_none() {
                            bad(format!("{a} needs a value"));
                        }
                    }
                    // Any `--report-github=…`: github::Mode::from_args says which values it takes.
                    "--record" | "--reset" | "--headful" | "--report-github" => {}
                    other if other.starts_with("--report-github=") => {}
                    "dry" | "live" if args.iter().any(|x| x == "--report-github") => {}
                    other if other.starts_with("--") && other.contains('=') => {
                        let name = other.split('=').next().unwrap_or(other);
                        if matches!(name, "--record" | "--reset" | "--headful") {
                            bad(format!("`{other}`: {name} is a switch and takes no value (write just `{name}`)"));
                        }
                        bad(format!("`{other}`: write `{name} <value>`, with a space"));
                    }
                    other => bad(format!("unknown argument `{other}` for run")),
                }
            }
            let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
            let opts = run::Options {
                model: value("--model").or_else(|| fastpilot_core::config::var("FASTPILOT_MODEL").ok().filter(|m| !m.is_empty())),
                // --max-steps (the user, on the command line) over the setting, else 30.
                max_steps: value("--max-steps")
                    .map(|n| n.parse::<usize>())
                    .transpose()?
                    .or_else(|| fastpilot_core::config::var("FASTPILOT_MAX_STEPS").ok().and_then(|n| n.trim().parse().ok()))
                    .unwrap_or(30)
                    .max(1),
                junit: value("--junit").map(Into::into),
                html: value("--html").map(Into::into),
                record: flag("--record"),
                reset: flag("--reset"),
                report_github: github::Mode::from_args(&args)?,
                workers: value("--workers").map(|n| n.parse::<usize>()).transpose()?.unwrap_or(1).max(1),
                web,
            };
            live(|| run::run(std::path::Path::new(path), opts), "run").await
        }
        Some("ios") if args.get(1).map(String::as_str) == Some("doctor") => {
            match fastpilot_ios::device::doctor(args.get(2).map(String::as_str)).await {
                Ok(report) => {
                    println!("{report}");
                    Ok(())
                }
                Err(report) => {
                    println!("{report}");
                    std::process::exit(1);
                }
            }
        }
        Some("--version" | "-V") => {
            println!("fastpilot {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}
