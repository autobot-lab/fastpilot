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
/// A change done: its line, and the id that undoes it (FC6), when it can be undone.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub line: String,
    pub undo: Option<String>,
}

impl Done {
    fn plain(line: String) -> Self {
        Done { line, undo: None }
    }

    fn undoable(line: String, item: &str, op: crate::undo::Op) -> Self {
        let undo = crate::undo::record(&line, item, op);
        Done { line, undo }
    }
}

/// The terminal's result: the line, then how to undo it.
pub fn print_done(d: &Done) {
    println!("✓ {}", d.line);
    if let Some(id) = &d.undo {
        println!("  Undo: fastpilot undo {id}");
    }
}

pub fn add_app(what: &str, ios: bool, s: Surface) -> Result<(App, bool, Option<String>), Fail> {
    let t = parse_target(what, ios).ok_or_else(|| bad_target(what, s))?;
    let same = |v: &Option<String>, w: &Option<String>| v.is_some() && v == w;
    // The same address as stored: parsed like `apps::add` does, so "shop.example" finds "https://shop.example/".
    let web = t.web.as_deref().map(|w| url::Url::parse(w).map(|u| u.to_string()).unwrap_or_else(|_| w.to_owned()));
    if let Some(a) = apps::list().into_iter().find(|a| (web.is_some() && a.targets.web.as_deref().map(|x| x.trim_end_matches('/')) == web.as_deref().map(|x| x.trim_end_matches('/'))) || same(&a.targets.android, &t.android) || same(&a.targets.ios, &t.ios)) {
        return Ok((a, false, None));
    }
    match apps::add("", t) {
        Ok(a) => {
            let undo = crate::undo::record(&format!("Added {}", app_what(&a)), &format!("app:{}", a.id), crate::undo::Op::RemoveApp { id: a.id.clone() });
            Ok((a, true, undo))
        }
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

/// E3: rename an app (its id, folder and passwords stay): "Renamed the app to “Shop (test)”", with
/// Undo.
pub fn rename_app(typed: &str, name: &str, s: Surface) -> Result<(App, Done), Fail> {
    let app = resolve_app(typed, s)?;
    let renamed = apps::update(&app.id, Some(name), None).map_err(|e| fail("bad_name", format!("Not renamed: {e}.")))?;
    if renamed.name == app.name {
        return Ok((renamed, Done::plain(format!("The app is already called {}. Nothing changed.", q(&app.name)))));
    }
    let line = format!("Renamed the app to {}", q(&renamed.name));
    let undo = crate::undo::record(&line, &format!("app:{}", app.id), crate::undo::Op::RenameApp { id: app.id.clone(), name: app.name.clone() });
    Ok((renamed, Done { line, undo }))
}

/// Remove an app for 7 days (FC6): "Removed app “X” · 4 tests, … · kept 7 days".
pub fn remove_app(typed: &str, s: Surface) -> Result<(App, Done), Fail> {
    let app = resolve_app(typed, s)?;
    let holds = app_holds(&app);
    let line = format!("Removed app {}{} · kept 7 days", q(&app.name), if holds.is_empty() { String::new() } else { format!(" · {holds}") });
    let undo = crate::undo::remove_app(&app, &line, true)?;
    Ok((app, Done { line, undo }))
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
pub fn what_runs(s: &Schedule) -> String {
    if Path::new(&s.tests).is_file() {
        schedule::tests_label(&s.tests)
    } else {
        format!("all tests of {}", apps::current().map(|a| a.name).unwrap_or_else(|| "this app".into()))
    }
}

/// "next run 02:00", or "runs when you check a release".
pub fn when_next(s: &Schedule) -> String {
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
pub fn add_schedule(preset: &str, test: Option<&str>, s: Surface) -> Result<(Schedule, Done), Fail> {
    let key = preset_key(preset).map_err(|_| fail("bad_preset", format!("{} is not a preset. Use every_2h_work_hours, nightly or before_release. Custom times are not supported yet.", q(preset.trim()))))?;
    let (_, dir, _) = saved_tests(s)?;
    let path = match test {
        Some(t) => find_test(t, s)?,
        None => dir,
    };
    let path = path.to_string_lossy().into_owned();
    let was = schedule::load().schedules.into_iter().find(|x| x.tests == path && schedule::presets().iter().any(|p| p.0 == key && p.3 == x.when) && x.watch.is_none() && x.agent.is_none());
    let (sched, new) = schedule::preset_on(key, &path, &schedule::tests_label(&path), None)?;
    let item = format!("schedule:{}", sched.id);
    let done = match (new, was.map(|w| w.enabled)) {
        (true, _) => Done::undoable(format!("Created schedule {} · {} · {}", q(&schedule_title(&sched)), what_runs(&sched), when_next(&sched)), &item, crate::undo::Op::RemoveSchedule { id: sched.id.clone() }),
        (false, Some(true)) => Done::plain(format!("{} is already on. Nothing changed.", q(&schedule_title(&sched)))),
        (false, _) => Done::undoable(format!("Resumed schedule {} · {}", q(&schedule_title(&sched)), when_next(&sched)), &item, crate::undo::Op::SetSchedule { id: sched.id.clone(), on: false }),
    };
    Ok((sched, done))
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
pub fn set_schedule(typed: &str, on: bool, s: Surface) -> Result<(Schedule, Done), Fail> {
    let sched = find_schedule(typed, s)?;
    let title = q(&schedule_title(&sched));
    if sched.enabled == on {
        return Ok((sched, Done::plain(format!("{title} is already {}. Nothing changed.", if on { "on" } else { "paused" }))));
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
    let done = Done::undoable(line, &format!("schedule:{}", sched.id), crate::undo::Op::SetSchedule { id: sched.id.clone(), on: !on });
    Ok((sched, done))
}

/// Remove a schedule for 7 days (FC6): off the system scheduler, back with Undo or Restore.
pub fn remove_schedule(typed: &str, s: Surface) -> Result<(Schedule, Done), Fail> {
    let sched = find_schedule(typed, s)?;
    let line = format!("Removed schedule {} · kept 7 days", q(&schedule_title(&sched)));
    let undo = crate::undo::remove_schedule(&sched, &line, true)?;
    Ok((sched, Done { line, undo }))
}

/// Give a schedule another of the one-click timings (its tests stay; its name follows the timing).
pub fn change_schedule(typed: &str, preset: &str, s: Surface) -> Result<(Schedule, Done), Fail> {
    let key = preset_key(preset).map_err(|_| fail("bad_preset", format!("{} is not a preset. Use every_2h_work_hours, nightly or before_release. Custom times are not supported yet.", q(preset.trim()))))?;
    let (_, _, _, when) = schedule::presets().into_iter().find(|p| p.0 == key).ok_or_else(|| fail("bad_preset", "Unknown preset"))?;
    let mut sched = find_schedule(typed, s)?;
    if sched.when == when {
        return Ok((sched.clone(), Done::plain(format!("{} is already {}. Nothing changed.", q(&schedule_title(&sched)), schedule::describe(&when)))));
    }
    let before = (sched.name.clone(), sched.when.clone());
    sched.name = schedule::auto_name(&schedule::tests_label(&sched.tests), &when);
    sched.when = when;
    let sched = schedule::save(sched, None)?;
    let line = format!("Changed schedule {} · {}", q(&schedule_title(&sched)), when_next(&sched));
    let done = Done::undoable(line, &format!("schedule:{}", sched.id), crate::undo::Op::SetWhen { id: sched.id.clone(), name: before.0, when: before.1 });
    Ok((sched, done))
}

/// Run the app's saved tests now, in the background (one by its name, or "all"). `agent`: the
/// agent whose Chat asked for it; the run is that agent's, so its Stop all stops it (v0.7.1 M6).
pub fn run_tests(name: &str, s: Surface, agent: Option<&crate::agents::Agent>) -> Result<Done, Fail> {
    let (title, path) = if name.eq_ignore_ascii_case("all") {
        let (_, dir, _) = saved_tests(s)?;
        ("Run all tests".to_owned(), dir)
    } else {
        let p = find_test(name, s)?;
        (schedule::tests_label(&p.to_string_lossy()), p)
    };
    let stem = if name.eq_ignore_ascii_case("all") { "all".to_owned() } else { path.file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default() };
    start_background(&["run", &path.to_string_lossy()], &format!("run-{stem}"), agent)?;
    Ok(Done::plain(format!("Running {} · {}", q(&title), watch_in_live())))
}

/// One release verdict for `urls` (default: the app's website): "Verdict: … · …". Waits for it.
pub fn ship_check(urls: Vec<String>) -> Result<String, Fail> {
    let urls = if urls.is_empty() {
        vec![apps::current().and_then(|a| a.targets.web).ok_or_else(|| fail("no_address", "There is no web address to check. Add a website app first, or give the address to check."))?]
    } else {
        urls
    };
    if let Some(bad) = urls.iter().find(|u| !(u.starts_with("https://") || u.starts_with("http://") || u.starts_with("file://"))) {
        return Err(fail("no_address", format!("{} is not a web address. Add a website app first, or give the address to check.", q(bad))));
    }
    let exe = std::env::current_exe().map_err(|e| fail("failed", e.to_string()))?;
    let out = std::process::Command::new(exe).arg("ship-check").args(&urls).stdin(std::process::Stdio::null()).output().map_err(|e| fail("failed", format!("The ship check could not start: {e}.")))?;
    Ok(verdict_line(&String::from_utf8_lossy(&out.stdout)))
}

/// Release watch on or off for the app in use (FC1, FC3): every 10 minutes for 2 hours.
pub fn release_watch(on: bool, s: Surface) -> Result<Done, Fail> {
    let (app, dir, _) = saved_tests(s)?;
    let tests = dir.to_string_lossy().into_owned();
    let running = schedule::load().schedules.into_iter().any(|x| x.tests == tests && x.enabled && x.watch.as_ref().is_some_and(|w| !w.ended && w.until_ms > schedule::now_ms()));
    if running == on {
        return Ok(Done::plain(format!("Release watch for {} is already {}. Nothing changed.", app.name, if on { "on" } else { "off" })));
    }
    schedule::release_watch(&tests, on)?;
    Ok(Done::undoable(format!("Turned {} release watch for {}", if on { "on" } else { "off" }, app.name), &format!("watch:{tests}"), crate::undo::Op::Watch { tests, on: !on }))
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
pub fn change_skills(agent: &str, changes: &[SkillChange], s: Surface) -> Result<Done, Fail> {
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
        return Ok(Done::plain(if reset { format!("{} is already General QA. Nothing changed.", a.name) } else { nothing.unwrap_or_else(|| format!("{} is already General QA. Nothing changed.", a.name)) }));
    }
    let before_skills = a.skills.clone();
    let agent_id = a.id.clone();
    a.skills = GENERAL_QA.iter().map(|x| (*x).to_owned()).chain(on).collect();
    let name = a.name.clone();
    crate::agents::update(a)?;
    let parts: Vec<String> = added.iter().map(|x| format!("+ {}", skill_short(x))).chain(removed.iter().map(|x| format!("− {}", skill_short(x)))).collect();
    let line = if changes.contains(&SkillChange::Reset) && added.is_empty() { format!("{name}: back to General QA ({})", parts.join(", ")) } else { format!("{name}: {}", parts.join(", ")) };
    Ok(Done::undoable(line, &format!("skills:{agent_id}"), crate::undo::Op::Skills { agent: agent_id.clone(), skills: before_skills }))
}

/// v0.7.1 FC8 migration: an agent saved before v0.7.1 with no skill list (it meant "every skill")
/// becomes General QA (SPEC FC8; fastpilot kept no record of which skills a run used, so none is
/// kept on by guess). Nothing is lost silently: each change is in the Undo list ("fastpilot undo
/// --list"), and its Undo puts the old list back. Run once when the dashboard starts.
pub fn migrate_skills() -> Vec<String> {
    crate::agents::list()
        .into_iter()
        .filter_map(|a| {
            let gqa: Vec<String> = GENERAL_QA.iter().map(|x| (*x).to_owned()).collect();
            if !crate::agents::set_skills_if_empty(&a.id, gqa).ok()? {
                return None;
            }
            crate::undo::record(&format!("{} · General QA", a.name), &format!("skills:{}", a.id), crate::undo::Op::Skills { agent: a.id.clone(), skills: vec![] });
            Some(a.name)
        })
        .collect()
}

/// v0.7.1 FC8: the Skills tab of one agent: General QA (locked) and the specialist skills by group,
/// each with its catalog name and line and whether it is on; Jira skills only when Jira is connected.
pub fn skills_page(a: &crate::agents::Agent) -> serde_json::Value {
    let on = specialists_on(a);
    let jira = jira_connected();
    let line = |id: &str| -> String {
        if id == crate::skills_catalog::CHOOSER {
            return "Picks the test technique that fits each job.".into();
        }
        crate::skills_catalog::CATALOG.iter().find(|s| s.0 == id).map(|s| s.3.to_owned()).unwrap_or_default()
    };
    let general: Vec<serde_json::Value> = GENERAL_QA.iter().map(|id| serde_json::json!({ "id": id, "name": skill_name(id), "line": line(id) })).collect();
    let groups: Vec<serde_json::Value> = GROUPS
        .iter()
        .filter_map(|(g, title)| {
            let skills: Vec<serde_json::Value> = specialists()
                .into_iter()
                .filter(|(id, gr)| gr == g && (!jira_skill(id) || jira))
                .map(|(id, _)| serde_json::json!({ "id": id, "name": skill_name(id), "line": line(id), "on": on.iter().any(|x| x == id) }))
                .collect();
            let (name, about) = title.split_once(" · ").unwrap_or((title, ""));
            (!skills.is_empty()).then(|| serde_json::json!({ "id": g, "name": name, "about": about, "skills": skills }))
        })
        .collect();
    let n = on.iter().filter(|id| !jira_skill(id) || jira).count();
    serde_json::json!({ "agent": a.id, "name": a.name, "specialists_on": n, "general": general, "groups": groups, "jira": jira })
}

/// The fastpilot processes now working for an agent (its Chat's tools, its scheduled runs), as
/// Live lists them: pids of this user's own `fastpilot` programs only.
fn agent_pids(agent: &str) -> Vec<u32> {
    let me = std::process::id();
    let mut pids: Vec<u32> = fastpilot_core::live::read_all().into_iter().filter(|p| p.agent.as_deref() == Some(agent) && p.kind != "ui" && p.pid != me && own_fastpilot(p.pid) && same_process(p.pid, p.started_ms)).map(|p| p.pid).collect();
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

/// Security L1: the live record is this process's, not a reused pid's: the process started before
/// the record was made (`ps -o etime=`, a few seconds of slack). A record without a time is skipped.
fn same_process(pid: u32, record_ms: u64) -> bool {
    let etime = std::process::Command::new("ps").args(["-o", "etime=", "-p", &pid.to_string()]).stderr(std::process::Stdio::null()).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    let Some(secs) = etime_secs(&etime) else { return false };
    let now = fastpilot_core::change::now_ms();
    record_ms != 0 && now.saturating_sub(secs * 1000) <= record_ms + 5000
}

/// `pid` leads its own process group (`ps -o pgid=` is the pid itself).
fn group_leader(pid: u32) -> bool {
    std::process::Command::new("ps").args(["-o", "pgid=", "-p", &pid.to_string()]).stderr(std::process::Stdio::null()).output().ok().is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == pid.to_string())
}

/// `ps` elapsed time (`[[dd-]hh:]mm:ss`) in seconds.
fn etime_secs(t: &str) -> Option<u64> {
    let (days, rest) = match t.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, t),
    };
    let parts: Vec<u64> = rest.split(':').map(|p| p.parse::<u64>().ok()).collect::<Option<_>>()?;
    let (h, m, s) = match parts.as_slice() {
        [m, s] => (0, *m, *s),
        [h, m, s] => (*h, *m, *s),
        _ => return None,
    };
    Some(((days * 24 + h) * 60 + m) * 60 + s)
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
        // A run that leads its own group (started in the background or by launchd): the whole group,
        // so its claude and Chrome end with it (Security #225 L11). Else the process alone.
        let target = if group_leader(*pid) { format!("-{pid}") } else { pid.to_string() };
        let _ = std::process::Command::new("kill").args(["-TERM", "--", &target]).status();
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
  remove <name>            Remove an app. It is kept 7 days: fastpilot undo";

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
            let (app, new, undo) = add_app(what, args.iter().any(|a| a == "--ios"), s)?;
            if !new {
                println!("✓ {} is already added. Nothing changed.", app.name);
                return Ok(());
            }
            let note = if seen_on_device(&app).await == Some(false) { " · not seen on a running device yet" } else { "" };
            print_done(&Done { line: format!("Added {}{note}", app_what(&app)), undo });
            Ok(())
        }
        (Some("switch"), Some(which)) => {
            let (app, changed) = switch_app(which, s)?;
            println!("{}", if changed { format!("✓ Switched to {}", app.name) } else { format!("✓ {} is already in use. Nothing changed.", app.name) });
            Ok(())
        }
        // FC6: no question (Undo replaces it); `--yes` is accepted and ignored for old scripts.
        (Some("remove"), Some(which)) => {
            let was_current = apps::current().map(|c| c.id);
            let (app, done) = remove_app(which, s)?;
            print_done(&done);
            if was_current.as_deref() == Some(app.id.as_str()) {
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
            if let Err(e) = fastpilot_core::secrets::check_value(&value) {
                return Err(fail("bad_value", format!("{e} Nothing was saved.")));
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
            let rows: Vec<(&str, String)> = specialists().into_iter().filter(|(id, g)| g == group && (!jira_skill(id) || jira)).map(|(id, _)| (id, skill_name(id))).collect();
            // FastTest (Low): no empty heading (Management has only Jira skills).
            if rows.is_empty() {
                continue;
            }
            out.push((*title).to_owned());
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
            let done = change_skills(name, &changes, Surface::Cli).map_err(|f| if f.code == "usage" { fail("usage_skill", f.msg) } else { f })?;
            print_done(&done);
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

// ---- Claude Code (FC1): the MCP tools apps, tests, schedules, map, ship_check, secrets, memory,
// history and agents. Who may change things is decided by the MCP server (a person's own Claude Code).

/// The fastpilot program's side of the MCP Control tools.
pub struct McpControl;

type Answer = Result<serde_json::Value, serde_json::Value>;

fn said(message: impl Into<String>) -> Answer {
    Ok(serde_json::json!({ "ok": true, "message": message.into() }))
}

fn said_with(message: impl Into<String>, extra: serde_json::Value) -> Answer {
    let mut v = serde_json::json!({ "ok": true, "message": message.into() });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    Ok(v)
}

/// A change's answer, with how Claude undoes it.
fn said_done(d: Done) -> Answer {
    match d.undo {
        Some(id) => said_with(d.line, serde_json::json!({ "undo": { "tool": "history", "action": "undo", "change_id": id } })),
        None => said(d.line),
    }
}

fn refused(f: Fail) -> serde_json::Value {
    serde_json::json!({ "ok": false, "error": f.code, "message": f.msg })
}

fn arg(args: &serde_json::Value, k: &str) -> Option<String> {
    args[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn no_app_mcp() -> Fail {
    fail("no_current_app", "No app is in use. Add one first with apps (a web address or an app id).")
}

fn unknown_action(action: &str, actions: &[&str]) -> Fail {
    fail("unknown_action", format!("This tool has no action {}. Actions: {}.", q(action), actions.join(", ")))
}

/// Start `fastpilot ARGS` in the background (the log goes to ~/fastpilot/logs/NAME.log), as the
/// dashboard's buttons do. Returns the log's name, the run's id for Claude.
/// `agent`: the run works for this agent (its Live tag and Stop all); `None`: this process's own
/// (`FASTPILOT_AGENT` of a Chat's fastpilot is passed on as it is).
pub fn start_background(args: &[&str], log_name: &str, agent: Option<&crate::agents::Agent>) -> Result<String, Fail> {
    let log = apps::root().join("logs").join(format!("{log_name}.log"));
    let could_not = |e: String| fail("failed", format!("The run could not start: {e}."));
    crate::filestore::ensure_dir(log.parent().unwrap_or(&apps::root()), true).map_err(could_not)?;
    let out = std::fs::OpenOptions::new().create(true).append(true).open(&log).map_err(|e| could_not(e.to_string()))?;
    let err = out.try_clone().map_err(|e| could_not(e.to_string()))?;
    let exe = std::env::current_exe().map_err(|e| could_not(e.to_string()))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args).env("FASTPILOT_RUN_BY_PERSON", "1").stdin(std::process::Stdio::null()).stdout(out).stderr(err);
    if let Some(a) = agent {
        cmd.env("FASTPILOT_AGENT", &a.id).env("FASTPILOT_AGENT_NAME", &a.name);
    }
    // Security #225 L11: its own process group, so Stop all ends what it started too (its claude,
    // their MCP bridge, its Chrome), not only the run.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().map_err(|e| could_not(e.to_string()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(log_name.to_owned())
}

/// "Verdict: DO NOT SHIP · 2 problems: …" from `fastpilot ship-check`'s report.
fn verdict_line(report: &str) -> String {
    let verdict = report.lines().rev().find_map(|l| l.strip_prefix("Verdict for the ").and_then(|r| r.split_once(": ")).map(|(_, v)| v.trim().to_owned())).or_else(|| report.lines().find_map(|l| l.strip_prefix("Verdict: ").map(|v| v.trim().to_owned()))).unwrap_or_else(|| "INCONCLUSIVE".into());
    let problems: Vec<String> = report.lines().filter_map(|l| l.strip_prefix("✗ ")).map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    match (verdict.as_str(), problems.len()) {
        ("SHIP", _) => "Verdict: SHIP · nothing blocking".into(),
        // FastTest (v0.7.1 dry run, Low): never an empty reason: the report's own last line says why.
        (_, 0) => {
            let why = report.lines().find_map(|l| l.strip_prefix("Why: ")).map(str::trim).filter(|w| !w.is_empty()).map(str::to_owned).or_else(|| report.lines().rev().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("Verdict")).map(|l| l.chars().take(200).collect()));
            format!("Verdict: {verdict} · {}", why.unwrap_or_else(|| "the pages could not be checked".into()))
        }
        (_, n) => format!("Verdict: {verdict} · {}: {}", plural(n, "problem", "problems"), problems.join("; ")),
    }
}

/// A run's time, on this computer's clock.
fn when(ms: u64) -> String {
    format!("{} {}", fastpilot_core::claim::local_date(ms), fastpilot_core::claim::local_hm(ms))
}

/// A new agent from one sentence ("only the checkout" → "Checkout QA"): General QA plus the
/// specialist skills the sentence asks for, on the current app. Terminal Chat's `agents create`
/// and the dashboard's "+ New agent" (its first message) both make it here.
pub fn create_agent_from(sentence: &str, source: Option<&str>) -> Result<(crate::agents::Agent, Done), Fail> {
    let app = apps::current();
    let devices: Vec<String> = app.as_ref().map(|a| a.kinds().into_iter().map(|k| match k { "android" => "android_emulator", "ios" => "ios_simulator", _ => "web" }.to_owned()).collect()).filter(|d: &Vec<String>| !d.is_empty()).unwrap_or_else(|| vec!["web".into()]);
    let spec: Vec<String> = crate::skills_catalog::suggest(sentence, app.as_ref().map(address).as_deref().unwrap_or(""), &devices).into_iter().map(|(id, _)| id).filter(|id| specialists().iter().any(|(x, _)| x == id) && (!jira_skill(id) || jira_connected())).collect();
    let base = agent_name_from(sentence);
    let taken: Vec<String> = crate::agents::list().into_iter().map(|a| a.name.to_lowercase()).collect();
    let name = (1..).map(|n| if n == 1 { base.clone() } else { format!("{base} {n}") }).find(|n| !taken.contains(&n.to_lowercase())).unwrap_or(base);
    let agent = crate::agents::Agent { name: name.clone(), job: sentence.chars().take(300).collect(), app: app.as_ref().map(address).unwrap_or_default(), devices, skills: GENERAL_QA.iter().map(|x| (*x).to_owned()).chain(spec.iter().cloned()).collect(), source: source.map(str::to_owned), ..Default::default() };
    let made = crate::agents::create(agent)?;
    let line = if spec.is_empty() { format!("Created agent {} · General QA", q(&name)) } else { format!("Created agent {} · General QA + {}", q(&name), spec.iter().map(|id| skill_name(id)).collect::<Vec<_>>().join(", ")) };
    let done = Done::undoable(line, &format!("agent:{}", made.id), crate::undo::Op::RemoveAgent { id: made.id.clone() });
    Ok((made, done))
}

/// "Checkout QA" from "only the checkout".
fn agent_name_from(sentence: &str) -> String {
    const SKIP: &[&str] = &["only", "the", "a", "an", "my", "our", "this", "that", "test", "tests", "testing", "check", "checks", "of", "for", "and", "to", "in", "on", "it", "just", "please", "should", "agent", "app"];
    let words: Vec<String> = sentence.split(|c: char| !c.is_alphanumeric()).map(str::to_lowercase).filter(|w| !w.is_empty() && !SKIP.contains(&w.as_str())).take(3).collect();
    let mut name = words.join(" ");
    if let Some(c) = name.get(..1) {
        name = c.to_uppercase() + &name[1..];
    }
    if name.is_empty() { "New agent".into() } else { format!("{name} QA").chars().take(40).collect() }
}

/// What an agent tests on, for the agents list.
fn agent_kind(a: &crate::agents::Agent) -> &'static str {
    match a.devices.first().map(String::as_str) {
        Some("android_emulator" | "android_device") => "Android app",
        Some("ios_simulator" | "ios_device") => "iPhone app",
        _ => "Website",
    }
}

impl fastpilot_mcp::Control for McpControl {
    fn call(&self, tool: &str, args: serde_json::Value) -> Answer {
        let s = Surface::Mcp;
        let action = arg(&args, "action").unwrap_or_default();
        let current = || apps::current().ok_or_else(no_app_mcp);
        let rt = tokio::runtime::Handle::current();
        let r: Answer = (|| match (tool, action.as_str()) {
            // ---- apps
            ("apps", "list") => {
                let all = apps::list();
                if all.is_empty() {
                    return said("No apps yet. Add one by giving a web address or an app id.");
                }
                let used = apps::current();
                let items: Vec<serde_json::Value> = all.iter().map(|a| serde_json::json!({ "name": a.name, "kind": kind_label(a), "address": address(a), "in_use": used.as_ref().is_some_and(|u| u.id == a.id) })).collect();
                let msg = match &used {
                    Some(u) => format!("{}. In use: {}.", plural(all.len(), "app", "apps"), u.name),
                    None => format!("{}.", plural(all.len(), "app", "apps")),
                };
                said_with(msg, serde_json::json!({ "items": items }))
            }
            ("apps", "add") => {
                let target = arg(&args, "target").unwrap_or_default();
                let (app, new, undo) = add_app(&target, arg(&args, "platform").as_deref() == Some("ios"), s).map_err(refused)?;
                if !new {
                    return said(format!("{} is already added. Nothing changed.", app.name));
                }
                let note = if rt.block_on(seen_on_device(&app)) == Some(false) { " · not seen on a running device yet" } else { "" };
                said_done(Done { line: format!("Added {}{note}", app_what(&app)), undo })
            }
            ("apps", "installed") => {
                let platform = arg(&args, "platform").unwrap_or_else(|| "android".into());
                let list = rt.block_on(installed(&platform)).map_err(refused)?;
                let place = if platform == "ios" { "the iPhone simulator" } else { "the Android emulator" };
                let names: Vec<&str> = list.iter().map(|(n, _)| n.as_str()).collect();
                let msg = if names.is_empty() { format!("0 apps on {place}.") } else { format!("{} on {place}: {}", plural(names.len(), "app", "apps"), names.join(", ")) };
                said_with(msg, serde_json::json!({ "items": list.iter().map(|(n, id)| serde_json::json!({ "name": n, "id": id })).collect::<Vec<_>>() }))
            }
            ("apps", "switch") => {
                let (app, changed) = switch_app(&arg(&args, "name").unwrap_or_default(), s).map_err(refused)?;
                said(if changed { format!("Switched to {}", app.name) } else { format!("{} is already in use. Nothing changed.", app.name) })
            }
            ("apps", "remove") => {
                // FC6: kept 7 days, with Undo: no question first.
                let (_, done) = remove_app(&arg(&args, "name").unwrap_or_default(), s).map_err(refused)?;
                said_done(done)
            }
            ("apps", other) => Err(refused(unknown_action(other, &["list", "add", "installed", "switch", "remove"]))),
            // ---- tests
            ("tests", "list") => {
                let (app, _, tests) = saved_tests(s).map_err(refused)?;
                let runs = fastpilot_memory::history::History::from_env().runs();
                let items: Vec<serde_json::Value> = tests
                    .iter()
                    .map(|(_, title)| {
                        let last = runs.iter().find(|r| &r.title == title).map(|r| if r.failed > 0 { "failed" } else if r.passed > 0 { "passed" } else { "not run" });
                        serde_json::json!({ "name": title, "last_result": last })
                    })
                    .collect();
                said_with(format!("{} in {}.", plural(tests.len(), "saved test", "saved tests"), app.name), serde_json::json!({ "items": items }))
            }
            ("tests", "run") => {
                let name = arg(&args, "name").unwrap_or_else(|| "all".into());
                let (title, path) = if name.eq_ignore_ascii_case("all") {
                    let (_, dir, _) = saved_tests(s).map_err(refused)?;
                    ("Run all tests".to_owned(), dir)
                } else {
                    let p = find_test(&name, s).map_err(refused)?;
                    (schedule::tests_label(&p.to_string_lossy()), p)
                };
                let id = if args["all_platforms"] == true {
                    let test = path.file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    crate::run_all::start(&test).map_err(|e| refused(e.into()))?;
                    format!("run-all-{test}")
                } else {
                    let stem = if name.eq_ignore_ascii_case("all") { "all".to_owned() } else { path.file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default() };
                    start_background(&["run", &path.to_string_lossy()], &format!("run-{stem}"), None).map_err(refused)?
                };
                said_with(format!("Running {} · {}", q(&title), watch_in_live()), serde_json::json!({ "run_id": id }))
            }
            ("tests", other) => Err(refused(unknown_action(other, &["list", "run"]))),
            // ---- schedules
            ("schedules", "list") => {
                let store = schedule::load();
                if store.schedules.is_empty() {
                    return said_with("0 schedules.", serde_json::json!({ "items": [] }));
                }
                let failed = store.schedules.iter().filter(|x| store.last.get(&x.id).is_some_and(|l| l.passed == Some(false))).count();
                let items: Vec<serde_json::Value> = store
                    .schedules
                    .iter()
                    .map(|x| serde_json::json!({ "id": x.id, "name": schedule_title(x), "state": if x.enabled { "on" } else { "paused" }, "next_run": if x.enabled { next_run(&x.when) } else { None }, "last_result": store.last.get(&x.id).map(|l| l.summary.clone()) }))
                    .collect();
                let msg = if failed > 0 { format!("{}. {failed} failed last time.", plural(items.len(), "schedule", "schedules")) } else { format!("{}.", plural(items.len(), "schedule", "schedules")) };
                said_with(msg, serde_json::json!({ "items": items }))
            }
            ("schedules", "add") => {
                let preset = arg(&args, "preset").unwrap_or_else(|| "every_2h_work_hours".into());
                add_schedule(&preset, arg(&args, "tests").as_deref(), s).map(|(_, d)| said_done(d)).map_err(refused)?
            }
            ("schedules", a @ ("pause" | "resume")) => set_schedule(&arg(&args, "id").unwrap_or_default(), a == "resume", s).map(|(_, d)| said_done(d)).map_err(refused)?,
            ("schedules", "remove") => remove_schedule(&arg(&args, "id").unwrap_or_default(), s).map(|(_, d)| said_done(d)).map_err(refused)?,
            ("schedules", "run_now") => {
                let sched = find_schedule(&arg(&args, "id").unwrap_or_default(), s).map_err(refused)?;
                schedule::start_now(&sched.id).map_err(|e| refused(e.into()))?;
                said(format!("Running {} now · {}", q(&schedule_title(&sched)), watch_in_live()))
            }
            ("schedules", "release_watch") => {
                let on = args["on"].as_bool().ok_or_else(|| refused(fail("usage", "Say on: true or on: false.")))?;
                release_watch(on, s).map(said_done).map_err(refused)?
            }
            ("schedules", other) => Err(refused(unknown_action(other, &["list", "add", "pause", "resume", "remove", "run_now", "release_watch"]))),
            // ---- map
            ("map", "run") => {
                let app = current().map_err(refused)?;
                let (target, flag) = match (&app.targets.web, &app.targets.android, &app.targets.ios) {
                    (Some(w), _, _) => (w.clone(), None),
                    (None, Some(a), _) => (a.clone(), Some("--android")),
                    (None, None, Some(i)) => (i.clone(), Some("--ios")),
                    _ => return Err(refused(no_app_mcp())),
                };
                let screens = args["screens"].as_u64().unwrap_or(30).clamp(1, 200).to_string();
                let minutes = args["minutes"].as_u64().unwrap_or(10).clamp(1, 60).to_string();
                let mut a = vec!["map", target.as_str(), "--screens", screens.as_str(), "--minutes", minutes.as_str()];
                a.extend(flag);
                start_background(&a, &format!("map-{}", app.id), None).map_err(refused)?;
                said(format!("Mapping {} · up to {} or {} · {}", app.name, plural(screens.parse().unwrap_or(30), "screen", "screens"), plural(minutes.parse().unwrap_or(10), "minute", "minutes"), watch_in_live()))
            }
            ("map", "status") => {
                let Some(m) = crate::map::latest() else { return said("No map yet.") };
                said(if m.state == "mapping" { format!("Mapping {} · {} · {}", m.name, plural(m.screens.len(), "screen", "screens"), watch_in_live()) } else { format!("Mapped {} · {}", m.name, plural(m.screens.len(), "screen", "screens")) })
            }
            ("map", other) => Err(refused(unknown_action(other, &["run", "status"]))),
            // ---- ship check (reads pages only)
            ("ship_check", _) => {
                let urls: Vec<String> = args["urls"].as_array().map(|l| l.iter().filter_map(|u| u.as_str().map(str::to_owned)).collect()).unwrap_or_default();
                let urls = if urls.is_empty() { vec![current().map_err(refused)?.targets.web.ok_or_else(|| refused(fail("no_address", "There is no web address to check. Add a website app first, or give the address to check.")))?] } else { urls };
                if let Some(bad) = urls.iter().find(|u| !(u.starts_with("https://") || u.starts_with("http://") || u.starts_with("file://"))) {
                    return Err(refused(fail("no_address", format!("{} is not a web address. Add a website app first, or give the address to check.", q(bad)))));
                }
                let exe = std::env::current_exe().map_err(|e| refused(fail("failed", e.to_string())))?;
                let out = std::process::Command::new(exe).arg("ship-check").args(&urls).stdin(std::process::Stdio::null()).output().map_err(|e| refused(fail("failed", format!("The ship check could not start: {e}."))))?;
                said(verdict_line(&String::from_utf8_lossy(&out.stdout)))
            }
            // ---- secrets: names only
            ("secrets", a) if !args.as_object().is_some_and(|o| o.keys().all(|k| k == "action" || k == "name")) || !matches!(a, "list" | "request") => {
                Err(refused(fail("value_refused", "This tool cannot take a password. To save one, use action “request” and give the user the link or the command.")))
            }
            ("secrets", "list") => {
                let app = current().map_err(refused)?;
                let names = secret_names(&app);
                said(if names.is_empty() { format!("No passwords saved for {}.", app.name) } else { format!("{} saved for {}: {}. Values are never shown.", plural(names.len(), "password", "passwords"), app.name, names.join(", ")) })
            }
            ("secrets", _) => {
                let name = arg(&args, "name").unwrap_or_default();
                fastpilot_core::secrets::check_name(&name).map_err(|_| refused(fail("bad_name", format!("{} is not a valid name. Use letters, numbers and underscores, like TEST_PASSWORD.", q(&name)))))?;
                let app = current().map_err(refused)?;
                let link = std::fs::read_to_string(crate::browsers::dir().join("port")).ok().and_then(|p| p.trim().parse::<u16>().ok()).map(|p| format!("http://127.0.0.1:{p}/#/settings/secrets")).unwrap_or_else(|| "Settings › Secrets".into());
                let command = format!("fastpilot secret set {name}");
                said_with(
                    format!("fastpilot never takes a password through chat. To save {name} for {}, open Secrets in the dashboard: {link}. Or run this in Terminal: {command}", app.name),
                    serde_json::json!({ "dashboard_link": link, "command": command }),
                )
            }
            // ---- memory
            ("memory", "add_note") => {
                let app = current().map_err(refused)?;
                let text = arg(&args, "text").ok_or_else(|| refused(fail("empty_note", "The note is empty. Give the sentence to remember.")))?;
                // Security L2: marked as Claude Code's, so agents never read it as the user's own words.
                let note = fastpilot_memory::notes::Notes::from_env().add_from("how", &text, Some("claude-code")).map_err(|r| refused(fail("refused", r.message)))?;
                said_done(Done::undoable(format!("Remembered for {}: {}", app.name, q(&text)), &format!("note:{}", note.id), crate::undo::Op::RemoveNote { id: note.id }))
            }
            ("memory", "list") => {
                let app = current().map_err(refused)?;
                let notes = fastpilot_memory::notes::Notes::from_env().list();
                // What fastpilot learned from runs (a saved test that never ran has nothing learned yet).
                let learned: Vec<_> = crate::memory_api::rows(&fastpilot_memory::Memory::from_env()).into_iter().filter(|r| r.status != "new").collect();
                let items: Vec<serde_json::Value> = notes.iter().map(|n| serde_json::json!({ "type": "note", "text": n.text })).chain(learned.iter().map(|r| serde_json::json!({ "type": "learned", "text": r.summary }))).collect();
                said_with(format!("{} and {} for {}.", plural(notes.len(), "note", "notes"), plural(learned.len(), "thing learned", "things learned"), app.name), serde_json::json!({ "items": items }))
            }
            ("memory", other) => Err(refused(unknown_action(other, &["add_note", "list"]))),
            // ---- history
            ("history", "runs") => {
                let runs = fastpilot_memory::history::History::from_env().runs();
                let take = args["limit"].as_u64().unwrap_or(10).clamp(1, 50) as usize;
                let shown: Vec<_> = runs.into_iter().take(take).collect();
                let failed = shown.iter().filter(|r| r.failed > 0).count();
                let place = apps::current().map(|a| format!(" in {}", a.name)).unwrap_or_default();
                let items: Vec<serde_json::Value> = shown.iter().map(|r| serde_json::json!({ "id": r.id, "test": r.title, "result": if r.failed > 0 { "failed" } else { "passed" }, "when": when(r.finished_ms) })).collect();
                let msg = if failed > 0 { format!("{}{place}. {failed} failed.", plural(shown.len(), "recent run", "recent runs")) } else { format!("{}{place}.", plural(shown.len(), "recent run", "recent runs")) };
                said_with(msg, serde_json::json!({ "items": items }))
            }
            ("history", "show") => {
                let id = arg(&args, "run_id").unwrap_or_default();
                let r = fastpilot_memory::history::History::from_env().runs().into_iter().find(|r| r.id == id).ok_or_else(|| refused(fail("run_not_found", format!("No run with id {}. Use history runs to see recent runs.", q(&id)))))?;
                said(format!("{} of {} passed · {} · {}", r.passed, r.tests, r.title, when(r.finished_ms)))
            }
            ("history", "removed") => {
                let gone = crate::undo::removed();
                if gone.is_empty() {
                    return said("Nothing was removed in the last 7 days.");
                }
                let parts: Vec<String> = gone.iter().map(|r| format!("{} · {}", crate::undo::removed_label(r), plural(crate::undo::days_left(r) as usize, "day left", "days left"))).collect();
                said_with(
                    format!("{} recently removed: {}.", gone.len(), parts.join("; ")),
                    serde_json::json!({ "items": gone.iter().map(|r| serde_json::json!({ "kind": r.kind, "name": r.name, "days_left": crate::undo::days_left(r) })).collect::<Vec<_>>() }),
                )
            }
            ("history", "undo") => crate::undo::undo(arg(&args, "change_id").as_deref(), s).map(said).map_err(refused)?,
            ("history", other) => Err(refused(unknown_action(other, &["runs", "show", "removed", "undo"]))),
            // ---- agents
            ("agents", "list") => {
                let all = crate::agents::list();
                let live = fastpilot_core::live::read_all();
                let parts: Vec<String> = all
                    .iter()
                    .map(|a| {
                        let n = live.iter().filter(|p| p.agent.as_deref() == Some(a.id.as_str()) && p.kind != "ui").count();
                        if n > 0 { format!("{} ({}, {} running)", a.name, agent_kind(a), plural(n, "chat", "chats")) } else { format!("{} ({})", a.name, agent_kind(a)) }
                    })
                    .collect();
                said(format!("{}: {}.", plural(all.len(), "agent", "agents"), parts.join(", ")))
            }
            ("agents", "create") => {
                let sentence = arg(&args, "sentence").ok_or_else(|| refused(fail("usage", "Say what the new agent should test.")))?;
                // Security L2: made by Claude Code, so its job is never read as the user's own words.
                let (_, done) = create_agent_from(&sentence, Some("claude-code")).map_err(refused)?;
                said_done(done)
            }
            ("agents", "skills") => {
                let agent = arg(&args, "agent").unwrap_or_default();
                let list = |k: &str| args[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect::<Vec<_>>()).unwrap_or_default();
                let mut changes: Vec<SkillChange> = Vec::new();
                if args["reset"] == true {
                    changes.push(SkillChange::Reset);
                }
                changes.extend(list("add").into_iter().map(SkillChange::Add));
                changes.extend(list("remove").into_iter().map(SkillChange::Remove));
                if changes.is_empty() {
                    let a = resolve_agent(&agent).map_err(refused)?;
                    let on = specialists_on(&a);
                    let items: Vec<serde_json::Value> = GENERAL_QA
                        .iter()
                        .map(|id| serde_json::json!({ "name": skill_name(id), "id": id, "on": true, "group": "General QA" }))
                        .chain(specialists().into_iter().filter(|(id, _)| !jira_skill(id) || jira_connected()).map(|(id, g)| serde_json::json!({ "name": skill_name(id), "id": id, "on": on.iter().any(|x| x == id), "group": GROUPS.iter().find(|x| x.0 == g).map(|x| x.1).unwrap_or(g) })))
                        .collect();
                    return said_with(skills_header(&a), serde_json::json!({ "items": items }));
                }
                change_skills(&agent, &changes, s).map(said_done).map_err(refused)?
            }
            ("agents", "stop") => {
                let line = stop_agent(&arg(&args, "agent").unwrap_or_default(), s).map_err(refused)?;
                said(if line.starts_with("Stopped") { format!("{line}\nSay “run it again” to start them again.") } else { line })
            }
            ("agents", other) => Err(refused(unknown_action(other, &["list", "create", "skills", "stop"]))),
            (other, _) => Err(refused(fail("unknown_tool", format!("fastpilot has no tool {}.", q(other))))),
        })();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FastTest (v0.7.1 dry run, Low): an INCONCLUSIVE ship check always says why.
    #[test]
    fn a_verdict_always_has_its_reason() {
        assert_eq!(verdict_line("Verdict: INCONCLUSIVE\nhttps://shop.invalid: could not open the page (connection refused)\n"), "Verdict: INCONCLUSIVE · https://shop.invalid: could not open the page (connection refused)");
        assert_eq!(verdict_line("Verdict: INCONCLUSIVE\nWhy: nothing loaded\n"), "Verdict: INCONCLUSIVE · nothing loaded");
        assert_eq!(verdict_line("Verdict: INCONCLUSIVE\n"), "Verdict: INCONCLUSIVE · the pages could not be checked");
    }

    /// Security L1: `agent stop` signals only the process its live record was made by: a pid that
    /// was reused later (started after the record) is skipped.
    #[test]
    fn a_reused_pid_is_not_the_agents_process() {
        assert_eq!((etime_secs("05:07"), etime_secs("1:02:03"), etime_secs("2-01:00:00"), etime_secs("x")), (Some(307), Some(3723), Some(176400), None));
        let me = std::process::id();
        let now = fastpilot_core::change::now_ms();
        assert!(same_process(me, now), "this process started before a record made now");
        assert!(!same_process(me, now - 24 * 3600 * 1000), "a record from yesterday is not this process's");
        assert!(!same_process(me, 0), "a record without a time is skipped");
    }

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
