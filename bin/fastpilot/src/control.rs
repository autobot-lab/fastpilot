//! v0.7.1 "Full control from Chat": the operations behind the dashboard pages, in one place, for
//! the terminal (`fastpilot app|schedule|secret`), Claude Code (`fastpilot mcp`) and the dashboard
//! Chat. Each one calls the same functions the pages call (apps, schedule, keystore); nothing here
//! is a second implementation.

use std::path::PathBuf;

use crate::apps::{self, App, Targets};
use crate::schedule::{self, Schedule};

pub type Res<T> = Result<T, String>;

// ---- apps ---------------------------------------------------------------------------------------

/// An app by its id or its name (any case).
pub fn find_app(which: &str) -> Res<App> {
    let w = which.trim();
    apps::list().into_iter().find(|a| a.id == w || a.name.eq_ignore_ascii_case(w)).ok_or_else(|| format!("No app called \"{}\". See the list: fastpilot app list", w.chars().take(80).collect::<String>()))
}

/// The app a command works on: the one named, else the current one.
pub fn app_or_current(which: Option<&str>) -> Res<App> {
    match which {
        Some(w) => find_app(w),
        None => apps::current().ok_or_else(|| "No app yet. Add one first: fastpilot app add https://your-site".to_owned()),
    }
}

/// First labels of reverse-domain app ids.
const REVERSE: &[&str] = &["com", "org", "net", "io", "id", "co", "app", "dev", "me", "de", "uk", "nl", "fr", "jp", "br", "in"];

/// Add an app from what the person gives: a website address, or an Android/iPhone app id with its
/// kind (`android` / `ios`). The name is made from it unless one is given.
pub fn add_app(what: &str, kind: Option<&str>, name: Option<&str>) -> Res<App> {
    let what = what.trim().to_owned();
    let targets = match kind {
        Some("android") => Targets { android: Some(what), ..Default::default() },
        Some("ios") => Targets { ios: Some(what), ..Default::default() },
        Some(other) => return Err(format!("unknown kind \"{other}\": web, android or ios")),
        // A reverse-domain app id (com.shop.app) is not a website: say which kind it is.
        None if !what.contains("://") && apps::app_id(&what) && what.split('.').next().is_some_and(|f| REVERSE.contains(&f)) => {
            return Err(format!("\"{what}\" looks like an app id: say which kind, Android or iPhone (--android or --ios)"));
        }
        None => Targets { web: Some(if what.contains("://") { what } else { format!("https://{what}") }), ..Default::default() },
    };
    apps::add(name.unwrap_or(""), targets)
}

/// Make an app the current one (the dashboard and Claude Code work on it).
pub fn switch_app(which: &str) -> Res<App> {
    let app = find_app(which)?;
    apps::set_current(&app.id)
}

/// Take an app off the list; its folder (tests, memory, map) stays on disk.
pub fn remove_app(which: &str) -> Res<App> {
    let app = find_app(which)?;
    apps::remove(&app.id)?;
    Ok(app)
}

/// One line per app: `* Shop (shop) · https://shop.example` (the current one starred).
pub fn app_lines() -> Vec<String> {
    let current = apps::current().map(|a| a.id);
    apps::list()
        .into_iter()
        .map(|a| {
            let at: Vec<String> = [a.targets.web.clone(), a.targets.android.clone().map(|x| format!("Android {x}")), a.targets.ios.clone().map(|x| format!("iPhone {x}"))]
                .into_iter()
                .flatten()
                .chain(a.repo.as_ref().map(|r| format!("repo {}", r.display())))
                .collect();
            format!("{} {} ({}) · {}", if current.as_deref() == Some(a.id.as_str()) { "*" } else { " " }, a.name, a.id, if at.is_empty() { "no address yet".to_owned() } else { at.join(", ") })
        })
        .collect()
}

// ---- schedules ----------------------------------------------------------------------------------

/// The saved tests of the app being worked on: its `tests` folder (the current app's folder, or
/// this project folder in developer mode).
pub fn tests_dir() -> Res<PathBuf> {
    if std::env::var_os("FASTPILOT_DIR").is_none() {
        apps::enter(false);
    }
    std::env::current_dir().map(|d| d.join("tests")).ok().filter(|d| d.is_dir()).ok_or_else(|| "This app has no saved tests yet. Save a test first, then schedule it.".to_owned())
}

/// The preset keys, with the words people use for them.
pub fn preset_key(word: &str) -> Res<&'static str> {
    match word.trim().trim_start_matches("--").to_lowercase().replace(['_', ' '], "-").as_str() {
        "every-2h" | "two-hours" | "every-2-hours" | "2h" => Ok("two-hours"),
        "nightly" | "night" | "every-night" => Ok("night"),
        "before-release" | "release" | "before-each-release" => Ok("release"),
        other => Err(format!("unknown schedule \"{other}\": every_2h_work_hours, nightly or before_release")),
    }
}

/// A one-click schedule (the Schedules page's presets) for `tests` (default: all tests of the app).
/// An existing one with the same tests and timing is turned on again. Returns it and a sentence.
pub fn add_schedule(preset: &str, tests: Option<&str>) -> Res<(Schedule, String)> {
    let key = preset_key(preset)?;
    let tests = match tests {
        Some(t) => std::fs::canonicalize(t).map_err(|_| format!("Tests not found: {t}"))?,
        None => tests_dir()?,
    };
    let path = tests.to_string_lossy().into_owned();
    let (s, new) = schedule::preset_on(key, &path, &schedule::tests_label(&path), None)?;
    let said = format!("{} \"{}\" ({}): {}.", if new { "Created" } else { "Turned on" }, s.name, s.id, schedule::describe(&s.when));
    Ok((s, said))
}

/// A schedule by its id or its name (any case).
pub fn find_schedule(which: &str) -> Res<Schedule> {
    let w = which.trim();
    schedule::load().schedules.into_iter().find(|s| s.id == w || s.name.eq_ignore_ascii_case(w)).ok_or_else(|| format!("No schedule called \"{}\". See the list: fastpilot schedule list", w.chars().take(80).collect::<String>()))
}

/// Turn a schedule on or off (it stays saved).
pub fn set_schedule(which: &str, on: bool) -> Res<Schedule> {
    let s = find_schedule(which)?;
    schedule::set_enabled(&s.id, on)?;
    Ok(Schedule { enabled: on, ..s })
}

/// Delete a schedule (and its entry in launchd/systemd/cron).
pub fn remove_schedule(which: &str) -> Res<Schedule> {
    let s = find_schedule(which)?;
    schedule::delete(&s.id)?;
    Ok(s)
}

// ---- passwords (per app, E2) --------------------------------------------------------------------

/// The password names of `app`, with whether each has a value here: `(name, saved, from the
/// environment)`. Never a value.
pub fn secret_rows(app: Option<&App>) -> Vec<(String, bool, bool)> {
    use fastpilot_core::config::{self, Source};
    let listed = match config::current("vars") {
        Source::File(v) | Source::Env(v) => v,
        Source::Default => String::new(),
    };
    // The environment allow-list, then the names saved for the app (Security #205 M1: a saved name
    // is not on the allow-list; an environment value counts only for a listed name).
    let env_names: Vec<String> = listed.split(',').map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).collect();
    let mut names = env_names.clone();
    for n in app.iter().flat_map(|a| a.secrets.iter()) {
        if !names.contains(n) {
            names.push(n.clone());
        }
    }
    names
        .into_iter()
        .map(|n| {
            let saved = app.is_some_and(|a| fastpilot_core::secrets::has(&a.id, &n));
            let env = env_names.contains(&n) && std::env::var_os(&n).is_some();
            (n, saved, env)
        })
        .collect()
}

/// Keep a password for `app` (this Mac's Keychain, or the owner-only key file): the same store,
/// service and account as the dashboard's masked field. `by` is who did it, for the activity log,
/// which says "set", never the value.
pub fn set_secret(app: &App, name: &str, value: &str, by: &str) -> Res<()> {
    use fastpilot_core::secrets;
    secrets::check_name(name)?;
    secrets::check_value(value)?;
    // Kept as the app's own name, never added to the environment allow-list (Security #205 M1).
    fastpilot_core::change::audit_or_refuse(&format!("secret {name} of {}", app.id), None, Some("set"), by, "done")?;
    secrets::set(&app.id, name, value)?;
    apps::note_secret(&app.id, name, true)
}

/// Delete a password of `app`.
pub fn remove_secret(app: &App, name: &str, by: &str) -> Res<()> {
    use fastpilot_core::secrets;
    secrets::check_name(name)?;
    fastpilot_core::change::audit_or_refuse(&format!("secret {name} of {}", app.id), Some("set"), None, by, "done")?;
    secrets::remove(&app.id, name)?;
    apps::note_secret(&app.id, name, false)
}

// ---- the terminal (FC2) -------------------------------------------------------------------------

pub const APP_USAGE: &str = "usage: fastpilot app list
       fastpilot app add <https://address | app-id> [--android | --ios] [--name NAME]
       fastpilot app switch <name>
       fastpilot app remove <name> [--yes]

Your apps live in ~/fastpilot, one workspace. The current app is what the dashboard, Claude Code
and `fastpilot schedule add` / `secret` work on. Removing an app keeps its folder on disk.";

pub const SECRET_USAGE: &str = "usage: fastpilot secret list [--app NAME]
       fastpilot secret set NAME [--app NAME]      the value is typed at a hidden prompt, or piped in
       fastpilot secret remove NAME [--app NAME]

A test types a saved password as ${NAME}, only on its app's own website. The value is kept in this
Mac's Keychain (the same place the dashboard keeps it) and is never shown or written in a file.
It is never given on the command line: that would leave it in your shell history.";

fn opt(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned()
}

/// The words after the command that are not options (or an option's value).
fn plain(args: &[String], with_value: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if with_value.contains(&a.as_str()) {
            skip = true;
        } else if !a.starts_with("--") {
            out.push(a.clone());
        }
    }
    out
}

fn ask_yes(question: &str) -> bool {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return false;
    }
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes" | "ya")
}

/// `fastpilot app list|add|switch|remove`.
pub fn app_cli(args: &[String]) -> Res<()> {
    let words = plain(args, &["--name"]);
    match (words.first().map(String::as_str), words.get(1)) {
        (Some("list") | None, _) => {
            let lines = app_lines();
            if lines.is_empty() {
                println!("No apps yet. Add one: fastpilot app add https://your-site");
            }
            for l in lines {
                println!("{l}");
            }
            Ok(())
        }
        (Some("add"), Some(what)) => {
            let kind = if args.iter().any(|a| a == "--android") { Some("android") } else if args.iter().any(|a| a == "--ios") { Some("ios") } else { None };
            let app = add_app(what, kind, opt(args, "--name").as_deref())?;
            let current = apps::current().is_some_and(|c| c.id == app.id);
            println!("Added \"{}\" ({}).{}", app.name, app.id, if current { " It is the current app." } else { " Switch to it: fastpilot app switch" });
            if !current {
                println!("  fastpilot app switch \"{}\"", app.name);
            }
            Ok(())
        }
        (Some("switch"), Some(which)) => {
            let app = switch_app(which)?;
            println!("Now working on \"{}\". The dashboard and Claude Code use it from their next step.", app.name);
            Ok(())
        }
        (Some("remove"), Some(which)) => {
            let app = find_app(which)?;
            let sure = args.iter().any(|a| a == "--yes" || a == "-y") || ask_yes(&format!("Remove \"{}\" from your apps? Its folder (tests, memory, map) stays on disk.", app.name));
            if !sure {
                return Err("Nothing removed (answer y, or add --yes).".into());
            }
            remove_app(&app.id)?;
            println!("Removed \"{}\" from your apps. Its folder stays: {}", app.name, app.dir().display());
            Ok(())
        }
        _ => Err(APP_USAGE.into()),
    }
}

/// `fastpilot secret list|set|remove` for the current app (or `--app`).
pub fn secret_cli(args: &[String], read_value: impl FnOnce(&str) -> Res<String>) -> Res<()> {
    let words = plain(args, &["--app"]);
    let app = || app_or_current(opt(args, "--app").as_deref());
    match (words.first().map(String::as_str), words.get(1)) {
        (Some("list") | None, _) => {
            let app = app()?;
            let rows = secret_rows(Some(&app));
            if rows.is_empty() {
                println!("No passwords for \"{}\" yet. A test asks for one when it needs it, or: fastpilot secret set NAME", app.name);
            }
            for (name, saved, env) in rows {
                println!("{name:<24} {}", if env { "from the environment" } else if saved { "saved" } else { "not set yet" });
            }
            Ok(())
        }
        (Some("set"), Some(name)) => {
            if words.len() > 2 {
                return Err("The value is never given on the command line (it would stay in your shell history). Type it at the hidden prompt, or pipe it in.".into());
            }
            let app = app()?;
            fastpilot_core::secrets::check_name(name)?;
            let value = read_value(&format!("Value of {name} for {} (input hidden): ", app.name))?;
            set_secret(&app, name, &value, "terminal")?;
            println!("{name} is saved for \"{}\" in the {}. A test types it as ${{{name}}}, only on this app's own website.", app.name, fastpilot_core::jira::token::kept_in());
            Ok(())
        }
        (Some("remove"), Some(name)) => {
            let app = app()?;
            remove_secret(&app, name, "terminal")?;
            println!("{name} is removed for \"{}\".", app.name);
            Ok(())
        }
        _ => Err(SECRET_USAGE.into()),
    }
}
