//! v0.7.1 "Full control from Chat": the operations behind the dashboard pages, in one place, for
//! the terminal (`fastpilot app|schedule|secret|agent`), Claude Code (`fastpilot mcp`) and the
//! dashboard Chat. Each one calls the same functions the pages call (apps, schedule, agents,
//! keystore); nothing here is a second implementation.
//!
//! The words are FastUI/UX's `TEXT-terminal-mcp.md` (v0.7.1, FC-TEXT), used as written.

use std::path::{Path, PathBuf};

use crate::apps::{self, App, Targets};
use crate::schedule::{self, Every, Schedule};

pub type Res<T> = Result<T, String>;

/// A failed action: a snake_case code (Claude Code's tools) and the words for the person.
#[derive(Debug, Clone, PartialEq)]
pub struct Fail {
    pub code: &'static str,
    pub msg: String,
}

pub fn fail(code: &'static str, msg: impl Into<String>) -> Fail {
    Fail { code, msg: msg.into() }
}

impl From<String> for Fail {
    fn from(msg: String) -> Self {
        Fail { code: "failed", msg }
    }
}

/// Where a line is shown: a person's terminal, or Claude Code (the words differ only in what to
/// run next).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Cli,
    // Claude Code's tools use it (FC1).
    #[allow(dead_code)]
    Mcp,
}

/// A name in a sentence, in curly quotes.
pub fn q(s: &str) -> String {
    format!("“{s}”")
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The one closest name to `typed`, if any is close: its letters in order, or at most two typos.
fn closest<'a>(typed: &str, names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let t = typed.trim().to_lowercase();
    let in_order = |n: &str| {
        let mut it = n.chars();
        t.chars().all(|c| it.any(|x| x == c))
    };
    let names: Vec<&str> = names.into_iter().collect();
    names.iter().copied().find(|n| !t.is_empty() && in_order(&n.to_lowercase())).or_else(|| names.iter().copied().filter(|n| edits(&t, &n.to_lowercase()) <= 2).min_by_key(|n| edits(&t, &n.to_lowercase())))
}

fn edits(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for j in 0..b.len() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != b[j])).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

// ---- apps ---------------------------------------------------------------------------------------

/// First labels of reverse-domain app ids.
const REVERSE: &[&str] = &["com", "org", "net", "io", "id", "co", "app", "dev", "me", "de", "uk", "nl", "fr", "jp", "br", "in"];

/// Website, Android or iPhone (a folder app in developer mode has none).
pub fn kind_label(a: &App) -> &'static str {
    if a.targets.web.is_some() {
        "Website"
    } else if a.targets.android.is_some() {
        "Android"
    } else if a.targets.ios.is_some() {
        "iPhone"
    } else {
        ""
    }
}

/// The app's address, app id or folder.
pub fn address(a: &App) -> String {
    a.targets.web.clone().or_else(|| a.targets.android.clone()).or_else(|| a.targets.ios.clone()).or_else(|| a.repo.as_ref().map(|r| r.display().to_string())).unwrap_or_default()
}

/// "Shop staging https://stg.shop.example", "Android shop com.shop.app · Android".
pub fn app_what(a: &App) -> String {
    match kind_label(a) {
        "Android" | "iPhone" => format!("{} {} · {}", a.name, address(a), kind_label(a)),
        _ => format!("{} {}", a.name, address(a)),
    }
}

/// An app by its id or name; a part of a name matching one app is enough.
pub fn resolve_app(typed: &str, s: Surface) -> Result<App, Fail> {
    let all = apps::list();
    let t = typed.trim().to_lowercase();
    if let Some(a) = all.iter().find(|a| a.id == typed.trim() || a.name.to_lowercase() == t) {
        return Ok(a.clone());
    }
    let hits: Vec<&App> = all.iter().filter(|a| !t.is_empty() && a.name.to_lowercase().contains(&t)).collect();
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        [] => {
            let hint = closest(typed, all.iter().map(|a| a.name.as_str())).map(|n| format!(" Did you mean {}?", q(n))).unwrap_or_default();
            Err(fail(
                "not_found",
                match s {
                    Surface::Cli => format!("No app called {}.{hint} Run fastpilot app list to see your apps.", q(typed.trim())),
                    Surface::Mcp => format!("No app called {}.{hint} Use apps list to see the apps.", q(typed.trim())),
                },
            ))
        }
        many => Err(fail("ambiguous", format!("{} matches {} apps: {}. Use the full name.", q(typed.trim()), many.len(), many.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")))),
    }
}

/// What the person gave as an app: a web address (`https://…`, or `shop.example`), or a phone
/// app id (`com.shop.app`; Android unless `ios`).
pub fn parse_target(what: &str, ios: bool) -> Option<Targets> {
    let w = what.trim();
    if w.is_empty() || w.chars().any(char::is_whitespace) {
        return None;
    }
    if w.contains("://") {
        return Some(Targets { web: Some(w.to_owned()), ..Default::default() });
    }
    if apps::app_id(w) && w.split('.').next().is_some_and(|f| REVERSE.contains(&f.to_lowercase().as_str())) {
        return Some(if ios { Targets { ios: Some(w.to_owned()), ..Default::default() } } else { Targets { android: Some(w.to_owned()), ..Default::default() } });
    }
    (w.contains('.') && !w.starts_with('.') && !w.ends_with('.')).then(|| Targets { web: Some(format!("https://{w}")), ..Default::default() })
}

fn bad_target(what: &str, s: Surface) -> Fail {
    fail(
        "bad_target",
        match s {
            Surface::Cli => format!("{} is not a web address or an app id. Use a full address like https://shop.example, or an app id like com.shop.app.", q(what.trim())),
            Surface::Mcp => format!("{} is not a web address or an app id. Ask the user for a full address like https://shop.example, or an app id like com.shop.app.", q(what.trim())),
        },
    )
}

/// Add an app (the name is picked for it). `Ok((app, false))`: that app was there already.
pub fn add_app(what: &str, ios: bool, s: Surface) -> Result<(App, bool), Fail> {
    let t = parse_target(what, ios).ok_or_else(|| bad_target(what, s))?;
    let same = |v: &Option<String>, w: &Option<String>| v.is_some() && v == w;
    // The same address as stored: parsed like `apps::add` does, so "shop.example" finds "https://shop.example/".
    let web = t.web.as_deref().map(|w| url::Url::parse(w).map(|u| u.to_string()).unwrap_or_else(|_| w.to_owned()));
    if let Some(a) = apps::list().into_iter().find(|a| (web.is_some() && a.targets.web.as_deref().map(|x| x.trim_end_matches('/')) == web.as_deref().map(|x| x.trim_end_matches('/'))) || same(&a.targets.android, &t.android) || same(&a.targets.ios, &t.ios)) {
        return Ok((a, false));
    }
    match apps::add("", t) {
        Ok(a) => Ok((a, true)),
        Err(_) => Err(bad_target(what, s)),
    }
}

/// Is this phone app on a running Android emulator / iPhone simulator? `None`: not a phone app.
pub async fn seen_on_device(a: &App) -> Option<bool> {
    let (platform, id) = match (&a.targets.android, &a.targets.ios) {
        (Some(id), _) => ("android", id),
        (None, Some(id)) => ("ios", id),
        _ => return None,
    };
    Some(installed(platform).await.is_ok_and(|list| list.iter().any(|(_, x)| x == id)))
}

/// The apps people installed on the running Android emulator or iPhone simulator, as
/// `(name, app id)`, newest first. `Err` says why there is none to list.
pub async fn installed(platform: &str) -> Result<Vec<(String, String)>, Fail> {
    let name = |id: &str| apps::name_from_id(id);
    match platform {
        "android" => {
            let not_running = || fail("device_not_running", "No Android emulator is running. Start it from Device, then ask again.");
            let serial = fastpilot_android::device::resolve(None).await.map_err(|_| not_running())?;
            let adb = fastpilot_android::Adb::new().await.map_err(|_| not_running())?;
            let out = adb.shell(&serial, "pm list packages -3 -i --show-versioncode; dumpsys package packages | grep -E 'Package \\[|firstInstallTime'").await.map_err(|_| not_running())?;
            // `Package [com.shop.app]` is followed by its `firstInstallTime=2026-11-24 10:12:03`.
            let mut seen: Vec<(String, String)> = Vec::new();
            let mut last = String::new();
            for line in out.lines() {
                let l = line.trim();
                if let Some(rest) = l.strip_prefix("Package [") {
                    last = rest.split(']').next().unwrap_or("").to_owned();
                } else if let Some(t) = l.strip_prefix("firstInstallTime=")
                    && !last.is_empty()
                {
                    seen.push((t.to_owned(), std::mem::take(&mut last)));
                }
            }
            let third: Vec<String> = out.lines().filter_map(|l| l.trim().strip_prefix("package:")).map(|p| p.split_whitespace().next().unwrap_or("").to_owned()).filter(|p| !p.is_empty() && !p.starts_with("com.fastpilot.")).collect();
            seen.retain(|(_, p)| third.contains(p));
            seen.sort_by(|a, b| b.0.cmp(&a.0));
            let mut ids: Vec<String> = seen.into_iter().map(|(_, p)| p).collect();
            for p in third {
                if !ids.contains(&p) {
                    ids.push(p);
                }
            }
            Ok(ids.into_iter().map(|id| (name(&id), id)).collect())
        }
        "ios" => {
            let not_running = || fail("device_not_running", "No iPhone simulator is running. Start it from Device, then ask again.");
            let out = tokio::process::Command::new("xcrun").args(["simctl", "listapps", "booted"]).stdin(std::process::Stdio::null()).output().await.map_err(|_| fail("xcode_missing", "Install Xcode from the App Store, then say “look again”."))?;
            if !out.status.success() {
                return Err(not_running());
            }
            let text = String::from_utf8_lossy(&out.stdout);
            // Old-style plist: `"com.shop.app" = {` blocks, `ApplicationType = User;` for the person's apps.
            let mut apps_found = Vec::new();
            let mut current: Option<String> = None;
            for line in text.lines() {
                let l = line.trim();
                if let Some(id) = l.strip_suffix("= {").map(|x| x.trim().trim_matches('"').to_owned()).filter(|x| x.contains('.') && !x.contains(' ')) {
                    current = Some(id);
                } else if l.starts_with("ApplicationType = User")
                    && let Some(id) = current.take()
                {
                    apps_found.push((name(&id), id));
                }
            }
            Ok(apps_found)
        }
        _ => Err(fail("bad_platform", "Say android or ios.")),
    }
}

/// Make an app the one in use. `Ok((app, false))`: it was in use already.
pub fn switch_app(typed: &str, s: Surface) -> Result<(App, bool), Fail> {
    let app = resolve_app(typed, s)?;
    if apps::current().is_some_and(|c| c.id == app.id) {
        return Ok((app, false));
    }
    Ok((apps::set_current(&app.id)?, true))
}

/// What an app holds, for the line that removes it: "4 tests, 31 runs, 1 password, 1 schedule".
pub fn app_holds(a: &App) -> String {
    let dir = a.dir();
    let tests = crate::ui::test_files(&dir.join("tests")).len();
    let runs = fastpilot_memory::history::History::new(dir.join(".fastpilot")).runs().len();
    let passwords = a.secrets.iter().filter(|n| fastpilot_core::secrets::has(&a.id, n)).count();
    let scheds = app_schedules(a).len();
    [(tests, "test", "tests"), (runs, "run", "runs"), (passwords, "password", "passwords"), (scheds, "schedule", "schedules")].into_iter().filter(|(n, ..)| *n > 0).map(|(n, one, many)| plural(n, one, many)).collect::<Vec<_>>().join(", ")
}

/// The schedules that run an app's tests.
pub fn app_schedules(a: &App) -> Vec<Schedule> {
    let dir = std::fs::canonicalize(a.dir()).unwrap_or_else(|_| a.dir());
    schedule::load().schedules.into_iter().filter(|s| Path::new(&s.tests).starts_with(&dir) || Path::new(&s.tests).starts_with(a.dir())).collect()
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
        "every-2h" | "every-2h-work-hours" | "two-hours" | "every-2-hours" | "2h" => Ok("two-hours"),
        "nightly" | "night" | "every-night" => Ok("night"),
        "before-release" | "release" | "before-each-release" => Ok("release"),
        other => Err(format!("unknown schedule \"{other}\": every_2h_work_hours, nightly or before_release")),
    }
}

/// A schedule's name in sentences: its preset's title ("Every night"), else its own name.
pub fn schedule_title(s: &Schedule) -> String {
    schedule::presets().into_iter().find(|p| p.3 == s.when).map(|p| p.1.to_owned()).unwrap_or_else(|| s.name.clone())
}

/// The next time a schedule runs on this computer's clock (`HH:MM`); `None` for one with no timer.
pub fn next_run(w: &schedule::When) -> Option<String> {
    let slots = schedule::slots(w);
    if slots.is_empty() {
        return None;
    }
    let days = schedule::run_days(w);
    let (h, m, weekday) = fastpilot_core::claim::local_clock(schedule::now_ms());
    let hm = (h * 60 + m) as i32;
    for d in 0..8u8 {
        let day = (weekday - 1 + d) % 7 + 1;
        if days.as_ref().is_some_and(|ds| !ds.contains(&day)) {
            continue;
        }
        if let Some((h, m)) = slots.iter().find(|(h, m)| d > 0 || i32::from(*h) * 60 + i32::from(*m) > hm) {
            return Some(format!("{h:02}:{m:02}"));
        }
    }
    None
}

/// "all tests of Shop staging", or the saved test's title.
fn what_runs(s: &Schedule) -> String {
    if Path::new(&s.tests).is_file() {
        schedule::tests_label(&s.tests)
    } else {
        format!("all tests of {}", apps::current().map(|a| a.name).unwrap_or_else(|| "this app".into()))
    }
}

/// "next run 02:00", or "runs when you check a release".
fn when_next(s: &Schedule) -> String {
    match (s.when.every, next_run(&s.when)) {
        (Every::Release, _) => "runs when you check a release".into(),
        (_, Some(t)) => format!("next run {t}"),
        (_, None) => schedule::describe(&s.when),
    }
}

fn no_app() -> Fail {
    fail("no_current_app", "No app is in use. Add one first: fastpilot app add <address>")
}

/// The saved tests of the app in use, `(file, title)`, with the errors for none.
/// The app in use, its tests folder and its saved tests (`file`, `title`).
type Saved = (App, PathBuf, Vec<(String, String)>);

fn saved_tests(s: Surface) -> Result<Saved, Fail> {
    let app = apps::current().ok_or_else(|| match s {
        Surface::Cli => no_app(),
        Surface::Mcp => fail("no_current_app", "No app is in use. Add one first with apps (a web address or an app id)."),
    })?;
    let none = || fail("no_tests", format!("{} has no saved tests yet. Run something first, then save it as a test.", app.name));
    let dir = tests_dir().map_err(|_| none())?;
    let tests = crate::ui::test_files(&dir);
    if tests.is_empty() {
        return Err(none());
    }
    Ok((app, dir, tests))
}

/// One saved test of the app in use, by its id (file name without .md) or its title.
pub fn find_test(name: &str, s: Surface) -> Result<PathBuf, Fail> {
    let (app, dir, tests) = saved_tests(s)?;
    let n = name.trim().trim_end_matches(".md").to_lowercase();
    tests
        .iter()
        .find(|(p, title)| title.to_lowercase() == n || Path::new(p).file_stem().is_some_and(|f| f.to_string_lossy().to_lowercase() == n))
        .map(|(p, _)| dir.join(Path::new(p).file_name().unwrap_or_default()))
        .ok_or_else(|| fail("not_found", format!("No test called {} in {}. Saved tests: {}.", q(name.trim()), app.name, tests.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(", "))))
}

/// A one-click schedule (the Schedules page's presets) of all the app's tests, or of one saved
/// test. An existing one with the same tests and timing is turned on again. Returns it and its
/// line ("Created schedule …", "Resumed schedule …", or "… is already on. Nothing changed.").
pub fn add_schedule(preset: &str, test: Option<&str>, s: Surface) -> Result<(Schedule, String), Fail> {
    let key = preset_key(preset).map_err(|_| fail("bad_preset", format!("{} is not a preset. Use every_2h_work_hours, nightly or before_release. Custom times are not supported yet.", q(preset.trim()))))?;
    let (_, dir, _) = saved_tests(s)?;
    let path = match test {
        Some(t) => find_test(t, s)?,
        None => dir,
    };
    let path = path.to_string_lossy().into_owned();
    let was = schedule::load().schedules.into_iter().find(|x| x.tests == path && schedule::presets().iter().any(|p| p.0 == key && p.3 == x.when) && x.watch.is_none() && x.agent.is_none());
    let (sched, new) = schedule::preset_on(key, &path, &schedule::tests_label(&path), None)?;
    let line = match (new, was.map(|w| w.enabled)) {
        (true, _) => format!("Created schedule {} · {} · {}", q(&schedule_title(&sched)), what_runs(&sched), when_next(&sched)),
        (false, Some(true)) => format!("{} is already on. Nothing changed.", q(&schedule_title(&sched))),
        (false, _) => format!("Resumed schedule {} · {}", q(&schedule_title(&sched)), when_next(&sched)),
    };
    Ok((sched, line))
}

/// A schedule by its id, its name or its title.
pub fn find_schedule(typed: &str, s: Surface) -> Result<Schedule, Fail> {
    let w = typed.trim();
    schedule::load().schedules.into_iter().find(|x| x.id == w || x.name.eq_ignore_ascii_case(w) || schedule_title(x).eq_ignore_ascii_case(w)).ok_or_else(|| {
        fail(
            "not_found",
            match s {
                Surface::Cli => format!("No schedule with id {}. Run fastpilot schedule list to see them.", q(w)),
                Surface::Mcp => format!("No schedule with id {}. Use schedules list to see them.", q(w)),
            },
        )
    })
}

/// Pause or resume a schedule (it stays saved). Returns it and its line.
pub fn set_schedule(typed: &str, on: bool, s: Surface) -> Result<(Schedule, String), Fail> {
    let sched = find_schedule(typed, s)?;
    let title = q(&schedule_title(&sched));
    if sched.enabled == on {
        return Ok((sched, format!("{title} is already {}. Nothing changed.", if on { "on" } else { "paused" })));
    }
    schedule::set_enabled(&sched.id, on)?;
    let sched = Schedule { enabled: on, ..sched };
    let line = if on {
        match next_run(&sched.when) {
            Some(t) => format!("Resumed schedule {title} · next run {t}"),
            None => format!("Resumed schedule {title}"),
        }
    } else {
        format!("Paused schedule {title} · its history is kept")
    };
    Ok((sched, line))
}

/// Delete a schedule (and its entry in launchd/systemd/cron).
pub fn remove_schedule(typed: &str, s: Surface) -> Result<(Schedule, String), Fail> {
    let sched = find_schedule(typed, s)?;
    schedule::delete(&sched.id)?;
    let line = format!("Removed schedule {}", q(&schedule_title(&sched)));
    Ok((sched, line))
}

/// "watch it in Live: <the dashboard's Live page>" (the address once the dashboard has run here).
pub fn watch_in_live() -> String {
    match std::fs::read_to_string(crate::browsers::dir().join("port")).ok().and_then(|p| p.trim().parse::<u16>().ok()) {
        Some(port) => format!("watch it in Live: http://127.0.0.1:{port}/#/live"),
        None => "watch it in Live".into(),
    }
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

/// The names of the passwords `app` has (saved, or given by the environment).
pub fn secret_names(app: &App) -> Vec<String> {
    secret_rows(Some(app)).into_iter().filter(|(_, saved, env)| *saved || *env).map(|(n, ..)| n).collect()
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

// ---- agents: skills and Stop (FC8, Stop all) ----------------------------------------------------

/// General QA: always on for every agent, never removed (FC8).
pub const GENERAL_QA: &[&str] = &["choose-technique", "use-case-scenarios", "edge-cases", "exploratory-check", "regression", "bug-report", "test-report", "release-check"];

/// The specialist groups, in order, with their words.
const GROUPS: &[(&str, &str)] = &[
    ("black-box", "Black-box · Design tests from rules"),
    ("experience", "Experience-based · Look around and try to break it"),
    ("collaboration", "Collaboration · Start from a ticket or a story"),
    ("management", "Management · Reports and release decisions"),
];

/// A skill's name as the lists show it ("Checklist · Safe checkout").
pub fn skill_name(id: &str) -> String {
    if id == crate::skills_catalog::CHOOSER {
        return "Let fastpilot choose the technique".into();
    }
    crate::skills_catalog::CATALOG.iter().find(|s| s.0 == id).map(|s| s.2.to_owned()).unwrap_or_else(|| id.to_owned())
}

/// A skill's name in sentences ("Safe checkout").
pub fn skill_short(id: &str) -> String {
    let n = skill_name(id);
    n.strip_prefix("Checklist · ").map(str::to_owned).unwrap_or(n)
}

fn jira_skill(id: &str) -> bool {
    id.starts_with("jira-")
}

fn jira_connected() -> bool {
    fastpilot_core::jira::connected_settings().is_some()
}

/// Every specialist skill (not General QA), in catalog order: `(id, group)`.
pub fn specialists() -> Vec<(&'static str, &'static str)> {
    crate::skills_catalog::CATALOG.iter().filter(|s| !GENERAL_QA.contains(&s.0)).map(|s| (s.0, s.1)).collect()
}

/// The specialist skills on for an agent. An agent saved before v0.7.1 with no list used every
/// skill, so it has every specialist on until it is changed.
pub fn specialists_on(a: &crate::agents::Agent) -> Vec<String> {
    let all = specialists();
    if a.skills.is_empty() {
        return all.into_iter().map(|(id, _)| id.to_owned()).collect();
    }
    // In the order they were turned on.
    a.skills.iter().filter(|id| all.iter().any(|(x, _)| x == id)).cloned().collect()
}

/// An agent by its id or name.
pub fn resolve_agent(typed: &str) -> Result<crate::agents::Agent, Fail> {
    let all = crate::agents::list();
    let t = typed.trim().to_lowercase();
    all.iter().find(|a| a.id == typed.trim() || a.name.to_lowercase() == t).cloned().ok_or_else(|| fail("agent_not_found", format!("No agent called {}. Agents: {}.", q(typed.trim()), all.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", "))))
}

/// "Android regression · General QA + 2 specialist skills".
pub fn skills_header(a: &crate::agents::Agent) -> String {
    let on: Vec<String> = specialists_on(a).into_iter().filter(|id| !jira_skill(id) || jira_connected()).collect();
    if on.is_empty() { format!("{} · General QA", a.name) } else { format!("{} · General QA + {}", a.name, plural(on.len(), "specialist skill", "specialist skills")) }
}

/// One change of skills: `+id`, `-id` or `reset`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillChange {
    Add(String),
    Remove(String),
    Reset,
}

/// Change an agent's specialist skills. Returns the line: what was added and removed, or why
/// nothing changed.
pub fn change_skills(agent: &str, changes: &[SkillChange], s: Surface) -> Result<String, Fail> {
    let mut a = resolve_agent(agent)?;
    if changes.is_empty() {
        return Err(fail(
            "usage",
            match s {
                Surface::Cli => "Say which skill: +id to add, -id to remove, or reset. Run fastpilot agent skills <agent> --all to see the ids.".to_owned(),
                Surface::Mcp => "Say which skill: add, remove, or reset. Use skills with no changes to see every skill.".to_owned(),
            },
        ));
    }
    let before = specialists_on(&a);
    let mut on = before.clone();
    let known = |id: &str| -> Result<String, Fail> {
        let id = id.trim().to_lowercase();
        if GENERAL_QA.contains(&id.as_str()) || specialists().iter().any(|(x, _)| *x == id) {
            return Ok(id);
        }
        let all: Vec<&str> = GENERAL_QA.iter().copied().chain(specialists().into_iter().map(|(x, _)| x)).collect();
        let hint = closest(&id, all).map(|n| format!(" Did you mean {n}?")).unwrap_or_default();
        Err(fail(
            "skill_not_found",
            match s {
                Surface::Cli => format!("No skill called {}.{hint} Run fastpilot agent skills <agent> --all to see every skill.", q(&id)),
                Surface::Mcp => format!("No skill called {}.{hint} Use skills with no changes to see every skill.", q(&id)),
            },
        ))
    };
    let mut nothing: Option<String> = None;
    for c in changes {
        match c {
            SkillChange::Reset => on.clear(),
            SkillChange::Add(id) => {
                let id = known(id)?;
                if GENERAL_QA.contains(&id.as_str()) {
                    nothing.get_or_insert(format!("{} is part of General QA and always on. Nothing changed.", skill_short(&id)));
                } else if jira_skill(&id) && !jira_connected() {
                    return Err(fail(
                        "jira_not_connected",
                        match s {
                            Surface::Cli => format!("{} appears after you connect Jira. Connect it in the dashboard: Settings › Integrations › Jira. You can check with fastpilot jira status. Nothing changed.", q(&skill_name(&id))),
                            Surface::Mcp => format!("{} appears after you connect Jira. Tell the user to connect it in the dashboard: Settings › Integrations › Jira. Nothing changed.", q(&skill_name(&id))),
                        },
                    ));
                } else if on.contains(&id) {
                    nothing.get_or_insert(format!("{} is already on for {}. Nothing changed.", skill_short(&id), a.name));
                } else {
                    on.push(id);
                }
            }
            SkillChange::Remove(id) => {
                let id = known(id)?;
                if GENERAL_QA.contains(&id.as_str()) {
                    return Err(fail("general_qa_locked", format!("{} is part of General QA, which is always on. It cannot be removed. Nothing changed.", skill_short(&id))));
                }
                if on.contains(&id) {
                    on.retain(|x| *x != id);
                } else {
                    nothing.get_or_insert(format!("{} is not on for {}. Nothing changed.", skill_short(&id), a.name));
                }
            }
        }
    }
    let added: Vec<String> = on.iter().filter(|x| !before.contains(x)).cloned().collect();
    let removed: Vec<String> = before.iter().filter(|x| !on.contains(x)).cloned().collect();
    if added.is_empty() && removed.is_empty() {
        let reset = changes.contains(&SkillChange::Reset);
        return Ok(if reset { format!("{} is already General QA. Nothing changed.", a.name) } else { nothing.unwrap_or_else(|| format!("{} is already General QA. Nothing changed.", a.name)) });
    }
    a.skills = GENERAL_QA.iter().map(|x| (*x).to_owned()).chain(on).collect();
    let name = a.name.clone();
    crate::agents::update(a)?;
    let parts: Vec<String> = added.iter().map(|x| format!("+ {}", skill_short(x))).chain(removed.iter().map(|x| format!("− {}", skill_short(x)))).collect();
    Ok(if changes.contains(&SkillChange::Reset) && added.is_empty() { format!("{name}: back to General QA ({})", parts.join(", ")) } else { format!("{name}: {}", parts.join(", ")) })
}

/// The fastpilot processes now working for an agent (its Chat's tools, its scheduled runs), as
/// Live lists them: pids of this user's own `fastpilot` programs only.
fn agent_pids(agent: &str) -> Vec<u32> {
    let me = std::process::id();
    let mut pids: Vec<u32> = fastpilot_core::live::read_all().into_iter().filter(|p| p.agent.as_deref() == Some(agent) && p.kind != "ui" && p.pid != me && own_fastpilot(p.pid)).map(|p| p.pid).collect();
    pids.dedup();
    pids
}

/// `pid` is a running `fastpilot` program of this user (never anything else, Security #183).
fn own_fastpilot(pid: u32) -> bool {
    let ps = |field: &str| std::process::Command::new("ps").args(["-o", &format!("{field}="), "-p", &pid.to_string()]).stderr(std::process::Stdio::null()).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    let uid = ps("uid");
    let me = std::process::Command::new("id").arg("-u").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    !uid.is_empty() && uid == me && ps("comm").rsplit('/').next() == Some("fastpilot")
}

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
        && !std::process::Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim_start().starts_with('Z'))
}

/// The title of an agent's newest chat (History), for a stop that did not work.
fn newest_chat_title(a: &crate::agents::Agent) -> String {
    let chats = fastpilot_memory::history::History::from_env().chats();
    a.conversations.iter().rev().find_map(|c| chats.iter().find(|x| &x.id == c).map(|x| x.title.clone())).unwrap_or_else(|| a.name.clone())
}

/// Stop every running chat of one agent: each of its fastpilot processes ends its sessions (the
/// browser closes, the device is freed) and quits; other agents keep running. Returns the line, or
/// the error when some did not stop.
pub fn stop_agent(agent: &str, s: Surface) -> Result<String, Fail> {
    let a = resolve_agent(agent)?;
    let pids = agent_pids(&a.id);
    if pids.is_empty() {
        return Ok(format!("Nothing to stop. {} has no running chats.", a.name));
    }
    for pid in &pids {
        let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while pids.iter().any(|p| alive(*p)) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let left = pids.iter().filter(|p| alive(**p)).count();
    let stopped = pids.len() - left;
    if left > 0 {
        return Err(fail(
            "stop_failed",
            match s {
                Surface::Cli => format!("Stopped {stopped} of {} chats in {}. {} did not stop. Run fastpilot agent stop {} again.", pids.len(), a.name, q(&newest_chat_title(&a)), shell_word(&a.name)),
                Surface::Mcp => format!("Stopped {stopped} of {} chats in {}. {} did not stop. Try stop again.", pids.len(), a.name, q(&newest_chat_title(&a))),
            },
        ));
    }
    Ok(format!("Stopped {} in {}", plural(stopped, "chat", "chats"), a.name))
}

fn shell_word(w: &str) -> String {
    if w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') { w.to_owned() } else { format!("\"{}\"", w.replace('"', "\\\"")) }
}

// ---- the terminal (FC2) -------------------------------------------------------------------------

pub const APP_USAGE: &str = "fastpilot app — the apps fastpilot tests

Usage: fastpilot app <list|add|switch|remove> …

  list                     Show your apps. The one in use is marked.
  add <address|app-id>     Add a website (https://…) or a phone app (com.shop.app).
                           The name is picked for you.
  switch <name>            Use this app from now on.
  remove <name>            Remove an app.";

pub const SECRET_USAGE: &str = "fastpilot secret — passwords your tests use

Usage: fastpilot secret <list|set|remove> [<NAME>] [--app <name>]

  list               Show the names of saved passwords. Values are never shown.
  set <NAME>         Save a password. You type it when asked; it is hidden.
                     To use a pipe: echo … | fastpilot secret set <NAME>
  remove <NAME>      Remove a saved password. This cannot be undone.

Passwords go to the macOS Keychain. They are never part of a command,
a chat or a tool call.";

pub const AGENT_USAGE: &str = "fastpilot agent — skills and Stop for an agent

Usage: fastpilot agent <skills|stop> <agent> …

  skills <agent>                   Show what the agent can do.
  skills <agent> --all             Show every skill, on or off.
  skills <agent> +id [+id …]       Add specialist skills.
  skills <agent> -id [-id …]       Remove specialist skills.
  skills <agent> reset             Back to General QA: turn off every specialist skill.
  stop <agent>                     Stop every running chat of this agent.
                                   Browsers close and devices are freed.
                                   Other agents keep running.

General QA is always on and cannot be removed.
Example: fastpilot agent skills \"Android regression\" +safe-checkout +decision-table";

/// What a terminal command ends with: its exit code (TEXT §0.8: 0 done, 1 failed, 2 usage).
pub fn exit_code(r: &Result<(), Fail>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(f) if f.code == "usage" => 2,
        Err(_) => 1,
    }
}

/// Print a command's failure as the terminal shows it.
pub fn print_fail(f: &Fail) {
    if f.code == "usage" {
        eprintln!("{}", f.msg);
    } else {
        eprintln!("✗ {}", f.msg);
    }
}

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

fn usage(text: &str) -> Fail {
    fail("usage", text)
}

/// `app list`: a table with the one in use marked.
fn app_table() -> String {
    let all = apps::list();
    if all.is_empty() {
        return "No apps yet. Add one: fastpilot app add https://your-site.example".into();
    }
    let current = apps::current().map(|a| a.id);
    let wn = all.iter().map(|a| a.name.chars().count()).max().unwrap_or(0) + 4;
    let wa = all.iter().map(|a| address(a).chars().count()).max().unwrap_or(0) + 4;
    let mut out = vec!["Apps".to_owned()];
    for a in &all {
        let used = current.as_deref() == Some(a.id.as_str());
        let row = format!("  {} {:<wn$}{:<10}{:<wa$}{}", if used { "●" } else { " " }, a.name, kind_label(a), address(a), if used { "in use" } else { "" });
        out.push(row.trim_end().to_owned());
    }
    out.join("\n")
}

/// `fastpilot app list|add|switch|remove`.
pub async fn app_cli(args: &[String]) -> Result<(), Fail> {
    let s = Surface::Cli;
    let words = plain(args, &["--name"]);
    match (words.first().map(String::as_str), words.get(1)) {
        (Some("list") | None, _) => {
            println!("{}", app_table());
            Ok(())
        }
        (Some("add"), None) => Err(fail("bad_target", "No address or app id was given. Run: fastpilot app add https://shop.example")),
        (Some("add"), Some(what)) => {
            let (app, new) = add_app(what, args.iter().any(|a| a == "--ios"), s)?;
            if !new {
                println!("✓ {} is already added. Nothing changed.", app.name);
                return Ok(());
            }
            let note = if seen_on_device(&app).await == Some(false) { " · not seen on a running device yet" } else { "" };
            println!("✓ Added {}{note}", app_what(&app));
            Ok(())
        }
        (Some("switch"), Some(which)) => {
            let (app, changed) = switch_app(which, s)?;
            println!("{}", if changed { format!("✓ Switched to {}", app.name) } else { format!("✓ {} is already in use. Nothing changed.", app.name) });
            Ok(())
        }
        (Some("remove"), Some(which)) => {
            let app = resolve_app(which, s)?;
            let sure = args.iter().any(|a| a == "--yes" || a == "-y") || ask_yes(&format!("Remove {} from your apps?", q(&app.name)));
            if !sure {
                return Err(fail("not_confirmed", "Nothing removed (answer y, or add --yes)."));
            }
            let holds = app_holds(&app);
            let was_current = apps::current().is_some_and(|c| c.id == app.id);
            apps::remove(&app.id)?;
            println!("✓ Removed app {}{}", q(&app.name), if holds.is_empty() { String::new() } else { format!(" · {holds}") });
            if was_current {
                match apps::current() {
                    Some(next) => println!("  Now using {}.", q(&next.name)),
                    None => println!("  No app is in use now. Add one: fastpilot app add <address>"),
                }
            }
            Ok(())
        }
        _ => Err(usage(APP_USAGE)),
    }
}

/// `fastpilot secret list|set|remove` for the app in use (or `--app`).
pub fn secret_cli(args: &[String], read_value: impl FnOnce(&str) -> Res<String>) -> Result<(), Fail> {
    let words = plain(args, &["--app"]);
    let app = || match opt(args, "--app") {
        Some(w) => resolve_app(&w, Surface::Cli),
        None => apps::current().ok_or_else(no_app),
    };
    let check_name = |n: &str| fastpilot_core::secrets::check_name(n).map_err(|_| fail("bad_name", format!("{} is not a valid name. Use letters, numbers and underscores, like TEST_PASSWORD.", q(n))));
    match (words.first().map(String::as_str), words.get(1)) {
        (Some("list") | None, _) => {
            let app = app()?;
            let names = secret_names(&app);
            if names.is_empty() {
                println!("No passwords saved for {}.", app.name);
            } else {
                println!("Passwords for {} (names only, values are never shown)", app.name);
                for n in names {
                    println!("  {n}");
                }
            }
            Ok(())
        }
        (Some("set"), Some(name)) => {
            if words.len() > 2 {
                return Err(fail("value_refused", format!("Do not put the password in the command. Run: fastpilot secret set {name} and type it when asked. Nothing was saved.")));
            }
            check_name(name)?;
            let app = app()?;
            let value = read_value(&format!("Password for {name} ({}). It is hidden as you type:\n", app.name)).unwrap_or_default();
            if value.is_empty() {
                return Err(fail("no_value", "No password was given. Run this in a terminal and type it when asked, or pipe it in."));
            }
            if fastpilot_core::secrets::check_value(&value).is_err() {
                return Err(fail("too_short", "That password is too short. Use at least 4 characters. Nothing was saved."));
            }
            let existed = fastpilot_core::secrets::has(&app.id, name);
            set_secret(&app, name, &value, "terminal").map_err(|_| fail("keychain", "Could not save to the Keychain. Nothing was saved. Unlock the Keychain, then run the command again."))?;
            println!("✓ {} password {name} for {}", if existed { "Changed" } else { "Saved" }, app.name);
            Ok(())
        }
        (Some("remove"), Some(name)) => {
            let app = app()?;
            if !fastpilot_core::secrets::has(&app.id, name) {
                let saved = secret_names(&app);
                return Err(fail("not_found", if saved.is_empty() { format!("No password called {} for {}.", q(name), app.name) } else { format!("No password called {} for {}. Saved: {}.", q(name), app.name, saved.join(", ")) }));
            }
            remove_secret(&app, name, "terminal")?;
            println!("✓ Removed password {name} from {} · this cannot be undone. To save it again: fastpilot secret set {name}", app.name);
            Ok(())
        }
        _ => Err(usage(SECRET_USAGE)),
    }
}

/// `agent skills <agent>` and `--all`.
fn skills_listing(a: &crate::agents::Agent, all: bool) -> String {
    let on = specialists_on(a);
    let jira = jira_connected();
    let mut out = vec![skills_header(a), String::new()];
    if all {
        for (group, title) in GROUPS {
            out.push((*title).to_owned());
            let rows: Vec<(&str, String)> = specialists().into_iter().filter(|(id, g)| g == group && (!jira_skill(id) || jira)).map(|(id, _)| (id, skill_name(id))).collect();
            let w = rows.iter().map(|(_, n)| n.chars().count()).max().unwrap_or(0) + 2;
            for (id, name) in rows {
                let needs = if jira_skill(id) { "      (needs Jira)" } else { "" };
                out.push(format!("  {:<5}{name:<w$}{id}{needs}", if on.iter().any(|x| x == id) { "on" } else { "off" }));
            }
        }
        if !jira {
            out.push("Jira skills appear after you connect Jira.".into());
        }
        return out.join("\n");
    }
    out.push("General QA (always on)".into());
    out.push("  Let fastpilot choose the technique, Use case scenarios, Try to break it,".into());
    out.push("  Explore for a set time, Regression, Bug report, Test report, Release check".into());
    out.push(String::new());
    let shown: Vec<String> = on.into_iter().filter(|id| !jira_skill(id) || jira).collect();
    if shown.is_empty() {
        out.push("No specialist skills on.".into());
    } else {
        out.push("Specialist skills on".into());
        let w = shown.iter().map(|id| skill_name(id).chars().count()).max().unwrap_or(0) + 4;
        for id in shown {
            out.push(format!("  {:<w$}{id}", skill_name(&id)));
        }
    }
    out.join("\n")
}

/// `fastpilot agent skills|stop`. `+id` and `-id` are values here, not options.
pub fn agent_cli(args: &[String]) -> Result<(), Fail> {
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("skills"), Some(name)) => {
            let rest = &args[2..];
            if rest.is_empty() || rest.iter().all(|a| a == "--all") {
                let a = resolve_agent(name)?;
                println!("{}", skills_listing(&a, rest.iter().any(|a| a == "--all")));
                return Ok(());
            }
            let changes: Vec<SkillChange> = rest
                .iter()
                .filter(|a| *a != "--all")
                .filter_map(|a| {
                    if a == "reset" {
                        Some(SkillChange::Reset)
                    } else if let Some(id) = a.strip_prefix('+') {
                        Some(SkillChange::Add(id.to_owned()))
                    } else {
                        a.strip_prefix('-').map(|id| SkillChange::Remove(id.to_owned()))
                    }
                })
                .collect();
            if changes.len() != rest.iter().filter(|a| *a != "--all").count() {
                return Err(fail("usage_skill", "Say which skill: +id to add, -id to remove, or reset. Run fastpilot agent skills <agent> --all to see the ids."));
            }
            let line = change_skills(name, &changes, Surface::Cli).map_err(|f| if f.code == "usage" { fail("usage_skill", f.msg) } else { f })?;
            println!("✓ {line}");
            Ok(())
        }
        (Some("stop"), Some(name)) => {
            let line = stop_agent(name, Surface::Cli)?;
            println!("✓ {line}");
            if line.starts_with("Stopped") {
                println!("  To start them again, say “run it again” in Chat.");
            }
            Ok(())
        }
        _ => Err(usage(AGENT_USAGE)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_and_close_names() {
        assert!(parse_target("shop", false).is_none());
        assert_eq!(parse_target("com.shop.app", false).and_then(|t| t.android), Some("com.shop.app".into()));
        assert_eq!(parse_target("com.shop.app", true).and_then(|t| t.ios), Some("com.shop.app".into()));
        assert_eq!(parse_target("shop.example", false).and_then(|t| t.web), Some("https://shop.example".into()));
        assert_eq!(closest("shp", ["Shop staging", "Android shop"]), Some("Shop staging"));
        assert_eq!(closest("pairwize", ["pairwise", "regression"]), Some("pairwise"));
        assert_eq!(closest("zzz", ["pairwise"]), None);
        // FC8: 8 General QA skills, 15 specialists, together the whole catalog plus the chooser.
        assert_eq!(GENERAL_QA.len(), 8);
        assert_eq!(specialists().len(), 15);
        assert_eq!(skill_short("safe-checkout"), "Safe checkout");
    }
}
