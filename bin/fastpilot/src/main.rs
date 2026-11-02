//! fastpilot: single-binary MCP server for AI-driven QA on web (Android/iOS later).

mod bench;

use std::error::Error;

use fastpilot_web::LaunchOptions;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: fastpilot <command> [--headful]

commands:
  mcp     serve MCP over stdio (what MCP clients run)
  bench   print snapshot and tap->diff latency against the demo page
          [--iterations N]

env:
  FASTPILOT_CHROME   Chrome/Chromium executable
  FASTPILOT_HEADFUL  1 to show the browser window
  FASTPILOT_LOG      tracing filter, e.g. debug (logs go to stderr)";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // stdout belongs to the MCP transport; logs always go to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::try_from_env("FASTPILOT_LOG").unwrap_or_else(|_| EnvFilter::new("warn")))
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let headful = flag("--headful") || std::env::var("FASTPILOT_HEADFUL").is_ok_and(|v| v == "1");
    let web = LaunchOptions { headless: !headful, executable: None };

    match args.first().map(String::as_str) {
        Some("mcp") => fastpilot_mcp::serve_stdio(web).await,
        Some("bench") => {
            let iterations = args
                .iter()
                .position(|a| a == "--iterations")
                .and_then(|i| args.get(i + 1))
                .map(|n| n.parse::<usize>())
                .transpose()?
                .unwrap_or(30);
            bench::run(web, iterations.max(1)).await
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
