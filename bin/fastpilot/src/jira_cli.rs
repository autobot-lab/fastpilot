//! `fastpilot jira [status | approve]` (v0.6, Security Medium 1). `status` says what is connected
//! (never the token). `approve` lets a person approve, one by one, the Jira issues and comments
//! Claude Code asked for (`jira_create_issue`, `jira_comment` file a request; they send nothing).
//! Only a person at a terminal: stdin must be a terminal and the answer is read from /dev/tty, so
//! a script or the AI cannot approve. Every request is checked again and printed without control
//! characters before it is sent.

use std::error::Error;
use std::io::{BufRead, IsTerminal, Write};

use fastpilot_core::change::printable;
use fastpilot_core::jira::{self, requests};

type Res<T> = Result<T, Box<dyn Error + Send + Sync>>;

const USAGE: &str = "usage: fastpilot jira [status | approve]\n  status    the Jira connected in the dashboard (Integrations › Jira)\n  approve   approve the Jira issues and comments Claude Code asked for (in your own terminal)";

pub async fn run(args: &[String]) -> Res<()> {
    match args.first().map(String::as_str).unwrap_or("status") {
        "status" => {
            match jira::connected_settings() {
                Some(s) => {
                    if s.signed_in() {
                        println!("Jira: connected to {} as {} (Sign in with Atlassian)", s.site, s.display_name);
                    } else {
                        println!("Jira: connected to {} as {}", s.site, s.email);
                    }
                    println!("Default project: {}", if s.project_key.is_empty() { "(none chosen)".to_owned() } else { format!("{} ({})", s.project_key, s.project_name) });
                    if s.needs_attention {
                        println!("Needs attention: Jira refused the saved token. Reconnect in the dashboard: Integrations › Jira.");
                    }
                }
                None => println!("Jira is not connected. Connect it in the dashboard: Integrations › Connect Jira."),
            }
            let n = requests::list().len();
            if n > 0 {
                println!("{n} request{} from Claude Code {} waiting: `fastpilot jira approve`", if n == 1 { "" } else { "s" }, if n == 1 { "is" } else { "are" });
            }
            Ok(())
        }
        "approve" => approve().await,
        "--help" | "-h" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown `jira {other}`\n{USAGE}").into()),
    }
}

async fn approve() -> Res<()> {
    if !std::io::stdin().is_terminal() {
        eprintln!("{}", crate::approve::NOT_A_PERSON);
        std::process::exit(1);
    }
    let tty = std::fs::File::open("/dev/tty").map_err(|e| format!("{} ({e})", crate::approve::NOT_A_PERSON))?;
    let mut tty = std::io::BufReader::new(tty);
    let list = requests::list();
    if list.is_empty() {
        println!("No Jira requests are waiting.");
        return Ok(());
    }
    println!("{} Jira request{} waiting.\n", list.len(), if list.len() == 1 { " is" } else { "s are" });
    for r in list {
        // Taken out first: a request is decided once, whatever happens next.
        let Some(r) = requests::take(&r.id) else { continue };
        let minutes = jira::now_ms().saturating_sub(r.created_ms) / 60_000;
        println!("  {}  (asked by Claude Code {})", printable(&r.title()), if minutes == 0 { "just now".to_owned() } else { format!("{minutes} min ago") });
        println!("  Site:     {}", printable(&r.site));
        if r.kind == "issue" {
            println!("  Summary:  {}", printable(&r.summary));
            println!("  Description:");
        } else {
            println!("  Comment:");
        }
        for line in r.text.lines() {
            println!("    {}", printable(line));
        }
        println!("\n  Did you ask for this? Exactly this text is sent; it never changes a ticket's status.");
        print!("Type yes to send it, or press Enter to drop it: ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        tty.read_line(&mut line)?;
        if !line.trim().eq_ignore_ascii_case("yes") {
            fastpilot_core::change::audit("jira", None, Some(&r.site), "terminal", "declined");
            println!("Dropped. Nothing was sent.\n");
            continue;
        }
        match requests::send(&r, "terminal").await {
            Ok((msg, link)) => println!("{msg} {link}\n"),
            Err(e) => println!("{e}\n"),
        }
    }
    Ok(())
}
