//! fastpilot: single-binary MCP server for AI-driven QA on web (Android/iOS later).

mod bench;
mod cli;
mod pool;
mod report;
mod run;
mod stability;

use std::error::Error;

use fastpilot_web::LaunchOptions;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: fastpilot <command> [--headful]

commands:
  config [list|get K|set K V|unset K|path]   settings in ~/.fastpilot/config.toml (env wins)
  open, snapshot, click, fill, ...   shell CLI: one command per step, sessions kept by a background
                                     daemon (`fastpilot help` lists them all)
  mcp     serve MCP over stdio (what MCP clients run)
  bench [web|android|ios]   print snapshot and tap->diff latency against the demo login
          [--iterations N]      (android: booted device with examples/demo-android installed)
  bench tokens [web|android|ios]   tokens per snapshot, per 1-step diff and per login run_flow
                                   (offline approximation)
  stability <flow.yaml> [--runs N]   repeat a flow in fresh sessions: pass rate, pass^k,
                                     95% upper bound of the failure rate
  ios doctor [udid]   why a physical iPhone can or cannot run tests yet (Xcode, signing,
                      Developer Mode, pairing, profile expiry), each with the fix
  run <tests dir|file.md> [--workers N] [--record] [--model ID] [--max-steps N] [--junit out.xml] [--html out.html]
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
        "list" => println!("{}", config::describe()),
        "path" => println!("{}", config::path().display()),
        "get" => {
            let k = key()?;
            config::setting(&k).ok_or(format!("unknown setting `{k}`"))?;
            match config::effective(&k) {
                config::Source::Env(v) => println!("{v}  (env FASTPILOT_{})", k.to_uppercase()),
                config::Source::File(v) => println!("{v}"),
                config::Source::Default => println!("(default)"),
            }
        }
        "set" => {
            let k = key()?;
            let v = args.get(2..).map(|r| r.join(" ")).filter(|v| !v.is_empty()).ok_or("set needs a value")?;
            let p = config::write(&k, Some(&v))?;
            println!("{k} = {v}  ({})", p.display());
            if std::env::var(format!("FASTPILOT_{}", k.to_uppercase())).is_ok() {
                println!("note: FASTPILOT_{} is set in the environment and wins over the file", k.to_uppercase());
            }
        }
        "unset" => {
            let k = key()?;
            let p = config::write(&k, None)?;
            println!("{k} removed  ({})", p.display());
        }
        other => return Err(format!("unknown `config {other}` (list, get, set, unset, path)").into()),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // stdout belongs to the MCP transport; logs always go to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::try_from_env("FASTPILOT_LOG").unwrap_or_else(|_| EnvFilter::new("warn")))
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let headful = flag("--headful") || fastpilot_core::config::var("FASTPILOT_HEADFUL").is_ok_and(|v| v == "1");
    let web = LaunchOptions { headless: !headful, executable: None };

    match args.first().map(String::as_str) {
        Some("mcp") => fastpilot_mcp::serve_stdio(web).await,
        Some("config") => config_cmd(&args[1..]),
        Some("daemon") if args.len() == 1 => cli::daemon(web).await,
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
            let Some(path) = args.get(1).filter(|a| !a.starts_with("--")) else {
                eprintln!("usage: fastpilot run <tests dir|file.md> [--workers N] [--model ID] [--max-steps N] [--junit out.xml] [--html out.html]");
                std::process::exit(2);
            };
            let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
            let opts = run::Options {
                model: value("--model").or_else(|| fastpilot_core::config::var("FASTPILOT_MODEL").ok().filter(|m| !m.is_empty())),
                max_steps: value("--max-steps").map(|n| n.parse::<usize>()).transpose()?.unwrap_or(30).max(1),
                junit: value("--junit").map(Into::into),
                html: value("--html").map(Into::into),
                record: flag("--record"),
                workers: value("--workers").map(|n| n.parse::<usize>()).transpose()?.unwrap_or(1).max(1),
                web,
            };
            run::run(std::path::Path::new(path), opts).await
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
