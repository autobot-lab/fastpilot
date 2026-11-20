//! D3 (v9 G7): `fastpilot init` and `fastpilot demo` end by offering the dashboard. In a terminal:
//! "Open the dashboard now to chat, schedule and see results? [Y/n]"; Enter or yes starts
//! `fastpilot ui` for that folder in this window. Elsewhere (a script, CI, `init --yes`) one line
//! says how, and nothing starts. `FASTPILOT_NO_BROWSER=1` prints the dashboard's link instead of
//! opening a browser.

use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

/// `ask`: the command may ask (not `--yes`). `cd`: how to get to `folder` from here, if not here.
pub fn dashboard(folder: &Path, ask: bool, cd: Option<&str>) {
    let go = cd.map(|c| format!("{c} && ")).unwrap_or_default();
    if !(ask && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        println!("\nDashboard: run `{go}fastpilot ui` and keep this window open.");
        return;
    }
    print!("\nOpen the dashboard now to chat, schedule and see results?\n[Y/n] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    if !matches!(line.trim().to_lowercase().as_str(), "" | "y" | "yes" | "ya") {
        println!("Later: run `{go}fastpilot ui` and keep this window open.");
        return;
    }
    println!("Starting the dashboard. Keep this window open; press Ctrl-C to stop it.");
    let exe = std::env::current_exe().unwrap_or_else(|_| "fastpilot".into());
    let mut c = std::process::Command::new(exe);
    c.arg("ui").current_dir(folder);
    if std::env::var_os("FASTPILOT_NO_BROWSER").is_some_and(|v| v == "1") {
        c.arg("--no-open");
    }
    if let Err(e) = c.status() {
        println!("The dashboard could not start ({e}). Run `{go}fastpilot ui` yourself.");
    }
}
