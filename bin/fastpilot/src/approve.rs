//! `fastpilot config approve` (D9, M13): the user approves, one by one, the requests the AI filed
//! through the `config` tool to loosen a limit. Only a person at a terminal: stdin must be a
//! terminal and the answer is read from /dev/tty. The request file is untrusted input: every
//! request is checked again, "Now" comes from the live settings, and every word shown is
//! fastpilot's own (there is no AI text in a request). Dangerous ones need the word typed (Pay).

use std::error::Error;
use std::io::{BufRead, IsTerminal, Write};

use fastpilot_core::change::{self, Change};
use fastpilot_core::config;

type Res<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub const NOT_A_PERSON: &str = "Needs a person. This command only works in a terminal you are typing in, so a script or the AI cannot approve a request.\nNothing has changed.\nRun it yourself in a terminal.";

pub fn run() -> Res<()> {
    if !std::io::stdin().is_terminal() {
        eprintln!("{NOT_A_PERSON}");
        std::process::exit(1);
    }
    let tty = std::fs::File::open("/dev/tty").map_err(|e| format!("{NOT_A_PERSON} ({e})"))?;
    let mut tty = std::io::BufReader::new(tty);
    let list = change::load_pending();
    if list.is_empty() {
        println!("No requests are waiting.");
        return Ok(());
    }
    println!("{} request{} waiting.\n", list.len(), if list.len() == 1 { " is" } else { "s are" });
    for p in list {
        change::remove_pending(&p.id)?;
        let now = match change::raw(&p.key) {
            Ok(v) => v,
            Err(e) => {
                println!("  Skipped: {e}. Nothing changed.\n");
                continue;
            }
        };
        let c = Change { key: p.key.clone(), before: p.before.clone(), after: p.after.clone() };
        if now != p.before || change::check(&c).is_err() || change::level(&c) != change::Level::Loosen {
            change::audit(&c.key, now.as_deref(), c.after.as_deref(), "terminal", "refused");
            println!("  Skipped: {} changed since the AI asked. Nothing changed.\n", change::label(&c.key));
            continue;
        }
        let card = change::card(&c);
        let minutes = change::now_ms().saturating_sub(p.created_ms) / 60_000;
        let ago = if minutes == 0 { "just now".to_owned() } else { format!("{minutes} min ago") };
        println!("  {:<44} LOOSENS A LIMIT", card.title);
        println!("  Now:     {}", card.now);
        println!("  After:   {}. {}", card.after, card.consequence);
        for n in &card.notes {
            println!("           {n}");
        }
        println!("  Asked:   {ago}, by the AI (Claude Code). Did you ask for this?");
        println!("  Starts:  at the next run\n");
        let prompt = match &card.typed {
            Some(w) => format!("Type {w} to allow it, or press Enter to keep it as it is: "),
            None => "Type yes to allow it, or press Enter to keep it as it is: ".to_owned(),
        };
        print!("{prompt}");
        std::io::stdout().flush()?;
        let mut line = String::new();
        tty.read_line(&mut line)?;
        let want = card.typed.as_deref().unwrap_or("yes");
        if line.trim().eq_ignore_ascii_case(want) {
            config::write(&c.key, c.after.as_deref())?;
            change::audit(&c.key, c.before.as_deref(), c.after.as_deref(), "terminal", "applied");
            let undo = match &c.before {
                Some(b) => format!("fastpilot config set {} \"{b}\"", c.key),
                None => format!("fastpilot config unset {}", c.key),
            };
            println!("Allowed. It starts from the next run. Saved in the activity log.\nTo undo it: {undo}\n");
        } else {
            change::audit(&c.key, c.before.as_deref(), c.after.as_deref(), "terminal", "declined");
            println!("Kept as it is. Nothing changed.\n");
        }
    }
    Ok(())
}
