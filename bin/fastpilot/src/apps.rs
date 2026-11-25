//! E6 (v0.7): the apps being tested, for this user, without a project folder.
//!
//! - `~/fastpilot/apps.json`: `{ current, apps: [{ id, name, targets: { web, android, ios }, repo,
//!   created_ms }] }`. One app is one entry; its website, Android and iPhone versions are the keys of
//!   `targets` (each optional, at least one), the same names Save test uses for a test on several
//!   platforms.
//! - `id` is a slug of the name, unique and never renamed: E2 keys an app's passwords on it
//!   (Keychain account `<id>/<NAME>`), and the app's data lives in `~/fastpilot/apps/<id>/`.
//! - `repo` is set only by `fastpilot init` in a code folder (never by the dashboard), so no page can
//!   point an app at a folder of its choosing.
//! - Removing an app takes it off the list; its folder stays.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Targets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ios: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct App {
    pub id: String,
    pub name: String,
    pub targets: Targets,
    #[serde(default)]
    pub repo: Option<PathBuf>,
    #[serde(default)]
    pub created_ms: u64,
    /// Security #205 M1: the names this app has a saved password for (names only, the values are in
    /// the key store). Never the environment allow-list (`vars`): a saved name does not make an
    /// environment value readable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    /// E2 (v0.7.2): the practice shop `fastpilot demo` wrote; its Chat starts with the demo welcome.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub demo: bool,
}

impl App {
    /// The platforms it has: "web", "android", "ios", in that order.
    pub fn kinds(&self) -> Vec<&'static str> {
        [("web", &self.targets.web), ("android", &self.targets.android), ("ios", &self.targets.ios)].into_iter().filter(|(_, t)| t.is_some()).map(|(k, _)| k).collect()
    }

    /// Where its tests, memory and map live: `~/fastpilot/apps/<id>`.
    pub fn dir(&self) -> PathBuf {
        // A repo app's tests and data live in its code folder (developer mode).
        self.repo.clone().unwrap_or_else(|| root().join("apps").join(&self.id))
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    #[serde(default)]
    current: Option<String>,
    #[serde(default)]
    apps: Vec<App>,
    /// Ids of removed apps: never given out again, so a new app never inherits an old one's folder or
    /// passwords (Security, #180 L1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    retired: Vec<String>,
}

/// `~/fastpilot`: the person's workspace (made on first use, no question).
pub fn root() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("fastpilot")
}

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn hold() -> crate::filestore::Held {
    crate::filestore::hold(&LOCK, &root(), ".apps.lock", false)
}

fn load() -> Store {
    std::fs::read_to_string(root().join("apps.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(s: &Store) -> Result<(), String> {
    crate::filestore::write_atomic(&root(), "apps.json", serde_json::to_string_pretty(s).unwrap_or_default().as_bytes(), false)
}

/// Note that app `id` has (`saved`) or no longer has a saved password called `name`.
pub fn note_secret(id: &str, name: &str, saved: bool) -> Result<(), String> {
    let _held = hold();
    let mut s = load();
    let Some(a) = s.apps.iter_mut().find(|a| a.id == id) else { return Err("That app is not listed any more.".into()) };
    a.secrets.retain(|n| n != name);
    if saved {
        a.secrets.push(name.to_owned());
    }
    save(&s)
}

/// All apps, in the order they were added.
pub fn list() -> Vec<App> {
    let _held = hold();
    load().apps
}

/// The app the dashboard and Claude Code work on now: the chosen one, else the only one.
pub fn current() -> Option<App> {
    let _held = hold();
    let s = load();
    s.current.as_deref().and_then(|c| s.apps.iter().find(|a| a.id == c)).or(if s.apps.len() == 1 { s.apps.first() } else { None }).cloned()
}

/// One app by id. Used by S1 (Map) and S8 (Run on all platforms), built on this contract.
pub fn get(id: &str) -> Option<App> {
    let _held = hold();
    load().apps.into_iter().find(|a| a.id == id)
}

/// Add an app; the first one becomes the current one. With no name, it is made from the address
/// (seamless-v13: paste the address only), kept apart from other apps' names with " (2)".
pub fn add(name: &str, targets: Targets) -> Result<App, String> {
    // The address first: it is what the person typed (the name is made from it).
    let targets = check_targets(targets)?;
    let given = !name.trim().is_empty();
    let name = check_name(if given { name.to_owned() } else { name_from(&targets) }.as_str())?;
    let _held = hold();
    let mut s = load();
    let taken = |n: &str| s.apps.iter().any(|a| a.name.eq_ignore_ascii_case(n));
    if given && taken(&name) {
        return Err(format!("there is already an app called \"{name}\": pick another name"));
    }
    let name = (1..).map(|n| if n == 1 { name.clone() } else { format!("{name} ({n})") }).find(|n| !taken(n)).unwrap_or(name);
    let base = slug(&name);
    // A fresh id: not in use, not retired, and no folder of that name left on disk.
    let free = |id: &String| !s.apps.iter().any(|a| &a.id == id) && !s.retired.contains(id) && !root().join("apps").join(id).exists();
    let id = (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(free).unwrap_or(base);
    let app = App { id: id.clone(), name, targets, repo: None, created_ms: now_ms(), secrets: Vec::new(), demo: false };
    // Its own folder for tests, memory and map, owner-only (#180 L3).
    crate::filestore::ensure_dir(&app.dir(), true)?;
    s.apps.push(app.clone());
    if s.current.is_none() {
        s.current = Some(id);
    }
    save(&s)?;
    Ok(app)
}

/// Change an app's name or platforms (its id stays).
pub fn update(id: &str, name: Option<&str>, targets: Option<Targets>) -> Result<App, String> {
    let name = name.map(check_name).transpose()?;
    let targets = targets.map(check_targets).transpose()?;
    let _held = hold();
    let mut s = load();
    if let Some(n) = &name
        && s.apps.iter().any(|a| a.id != id && a.name.eq_ignore_ascii_case(n))
    {
        return Err(format!("there is already an app called \"{n}\": pick another name"));
    }
    let app = s.apps.iter_mut().find(|a| a.id == id).ok_or("no such app")?;
    if let Some(n) = name {
        app.name = n;
    }
    if let Some(t) = targets {
        app.targets = t;
    }
    let out = app.clone();
    save(&s)?;
    Ok(out)
}

/// Take an app off the list; its folder (tests, memory, map) stays on disk.
pub fn remove(id: &str) -> Result<(), String> {
    let _held = hold();
    let mut s = load();
    let before = s.apps.len();
    s.apps.retain(|a| a.id != id);
    if s.apps.len() == before {
        return Err("no such app".into());
    }
    s.retired.push(id.to_owned());
    if s.current.as_deref() == Some(id) {
        s.current = s.apps.first().map(|a| a.id.clone());
    }
    save(&s)
}

/// FC6: bring back a removed app as it was (its id, folder, passwords). A name taken meanwhile
/// gets " (2)". It becomes the current app when none is.
pub fn restore(mut app: App) -> Result<App, String> {
    let _held = hold();
    let mut s = load();
    if s.apps.iter().any(|a| a.id == app.id) {
        return Err(format!("\"{}\" is already there", app.name));
    }
    let base = app.name.clone();
    app.name = (1..).map(|n| if n == 1 { base.clone() } else { format!("{base} ({n})") }).find(|n| !s.apps.iter().any(|a| a.name.eq_ignore_ascii_case(n))).unwrap_or(base);
    s.retired.retain(|r| r != &app.id);
    s.apps.push(app.clone());
    if s.current.is_none() {
        s.current = Some(app.id.clone());
    }
    save(&s)?;
    Ok(app)
}

/// Make `id` the current app.
pub fn set_current(id: &str) -> Result<App, String> {
    let _held = hold();
    let mut s = load();
    let app = s.apps.iter().find(|a| a.id == id).cloned().ok_or("no such app")?;
    s.current = Some(app.id.clone());
    save(&s)?;
    Ok(app)
}

/// A fastpilot project folder (`fastpilot init` ran there): a `.fastpilot` folder, or a `.mcp.json`
/// that starts fastpilot.
pub fn is_project(dir: &Path) -> bool {
    // The home folder's `.fastpilot` is fastpilot's own settings, not a project; `/` is not one either.
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if home.is_some_and(|h| real(&h) == real(dir)) || real(dir) == Path::new("/") {
        return false;
    }
    // A real folder, not a link to somewhere else (Security, #183 L3).
    dir.join(".fastpilot").symlink_metadata().is_ok_and(|m| m.is_dir()) || std::fs::read_to_string(dir.join(".mcp.json")).is_ok_and(|t| t.contains("\"fastpilot\""))
}

/// The app for a project folder, added as a Repo app (named after the folder) if it is not listed
/// yet. Nothing in the folder is moved or changed.
pub fn add_repo(folder: &Path) -> Result<App, String> {
    let folder = std::fs::canonicalize(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let _held = hold();
    let mut s = load();
    if let Some(a) = s.apps.iter().find(|a| a.repo.as_deref() == Some(folder.as_path())) {
        return Ok(a.clone());
    }
    let base = folder.file_name().map(|n| n.to_string_lossy().chars().filter(|c| !c.is_control()).take(50).collect::<String>()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Repo".into());
    let name = (1..).map(|n| if n == 1 { base.clone() } else { format!("{base} ({n})") }).find(|n| !s.apps.iter().any(|a| a.name.eq_ignore_ascii_case(n))).unwrap_or(base);
    let slugged = slug(&name);
    let free = |id: &String| !s.apps.iter().any(|a| &a.id == id) && !s.retired.contains(id) && !root().join("apps").join(id).exists();
    let id = (1..).map(|n| if n == 1 { slugged.clone() } else { format!("{slugged}-{n}") }).find(free).unwrap_or(slugged);
    let app = App { id: id.clone(), name, targets: Targets::default(), repo: Some(folder), created_ms: now_ms(), secrets: Vec::new(), demo: false };
    s.apps.push(app.clone());
    if s.current.is_none() {
        s.current = Some(id);
    }
    save(&s)?;
    Ok(app)
}

/// E2 (v0.7.2): the folder `fastpilot demo` wrote, as the app "fastpilot demo" (a website: its own
/// `demo/index.html`), made the app in use. The same folder again (`--force`) is the same app; a
/// second demo folder elsewhere gets "fastpilot demo (2)". Its tests are the folder's `tests/`.
pub fn add_demo(folder: &Path) -> Result<App, String> {
    let folder = std::fs::canonicalize(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let web = url::Url::from_file_path(folder.join("demo/index.html")).map_err(|()| "the demo folder has no usable path".to_owned())?.to_string();
    let _held = hold();
    let mut s = load();
    let id = match s.apps.iter_mut().find(|a| a.repo.as_deref() == Some(folder.as_path())) {
        Some(a) => {
            a.targets.web = Some(web);
            a.demo = true;
            a.id.clone()
        }
        None => {
            let name = (1..).map(|n| if n == 1 { "fastpilot demo".to_owned() } else { format!("fastpilot demo ({n})") }).find(|n| !s.apps.iter().any(|a| a.name.eq_ignore_ascii_case(n))).unwrap_or_default();
            let slugged = slug(&name);
            let free = |id: &String| !s.apps.iter().any(|a| &a.id == id) && !s.retired.contains(id) && !root().join("apps").join(id).exists();
            let id = (1..).map(|n| if n == 1 { slugged.clone() } else { format!("{slugged}-{n}") }).find(free).unwrap_or(slugged);
            let targets = Targets { web: Some(web), ..Default::default() };
            s.apps.push(App { id: id.clone(), name, targets, repo: Some(folder), created_ms: now_ms(), secrets: Vec::new(), demo: true });
            id
        }
    };
    s.current = Some(id.clone());
    save(&s)?;
    Ok(s.apps.into_iter().find(|a| a.id == id).expect("just saved"))
}

/// E6: start in the right folder, so every "here" (`.fastpilot`, `tests/`, the map, notes) is the
/// app's. In a project folder: that folder, listed as a Repo app (`make_current`: and chosen, for
/// the dashboard). Elsewhere: the current app's folder. `FASTPILOT_DIR` set by the person wins and
/// nothing moves. Returns the app now worked on.
pub fn enter(make_current: bool) -> Option<App> {
    if std::env::var_os("FASTPILOT_DIR").is_some() {
        return None;
    }
    let here = std::env::current_dir().ok()?;
    // A folder that already is a listed app's (its workspace folder, or a Repo): that app. This is
    // where Chat and runs start Claude Code, so its fastpilot works for the same app.
    if let Some(app) = app_at(&here) {
        set_scope(Some(&app));
        return Some(app);
    }
    if is_project(&here) {
        // Claude Code opened in a project works there, but only the person lists it as an app
        // (`fastpilot init`, starting the dashboard there, or the search): a cloned repo never
        // makes itself one, let alone the current one (Security, #183 L1).
        if !make_current {
            return None;
        }
        let app = add_repo(&here).ok()?;
        let _ = set_current(&app.id);
        set_scope(Some(&app));
        return Some(app);
    }
    let app = current()?;
    go(&app);
    Some(app)
}

/// `fastpilot run`: the app whose saved passwords this run may use (the folder's app, else the
/// current one), without moving out of the folder the test paths are relative to. In a project
/// folder that is no listed app's: only the app the dashboard named for this run (F1: the folder
/// `fastpilot ui` was started in keeps Chat's history in `.fastpilot`, so it looks like a project).
pub fn scope_here() {
    let here = std::env::current_dir().ok();
    let app = here.as_deref().and_then(app_at).or_else(|| if here.as_deref().is_some_and(is_project) { named() } else { current() });
    set_scope(app.as_ref());
}

/// QA #6103: `FASTPILOT_APP` (set by the dashboard's Chat for its own fastpilot) names the app
/// whose passwords this process may use. Only a listed app; nothing moves; S-1 still applies.
pub fn scope_named() {
    if let Some(app) = named() {
        set_scope(Some(&app));
    }
}

/// The app `FASTPILOT_APP` names. Security #208 L1: only with `FASTPILOT_APP_KEY` equal to the
/// running dashboard's key file, which a repo's `.mcp.json` cannot know.
fn named() -> Option<App> {
    let (Ok(id), Ok(key)) = (std::env::var("FASTPILOT_APP"), std::env::var("FASTPILOT_APP_KEY")) else { return None };
    let kept = std::fs::read_to_string(crate::browsers::dir().join("app-key")).unwrap_or_default();
    if key.len() < 32 || kept.trim() != key {
        return None;
    }
    get(&id)
}

/// The listed app whose folder `dir` is.
pub fn app_at(dir: &Path) -> Option<App> {
    let real = |p: &Path| std::fs::canonicalize(p).ok();
    let here = real(dir)?;
    list().into_iter().find(|a| real(&a.dir()).as_ref() == Some(&here))
}

/// E2 / S-1: the app whose passwords this process may use, and where (its website, its phones).
pub fn set_scope(app: Option<&App>) {
    fastpilot_core::secrets::set_scope(app.map(|a| fastpilot_core::secrets::Scope {
        app: a.id.clone(),
        web: a.targets.web.clone(),
        // D07-L3: the package or bundle id, without an activity (`com.shop.app/.Main`).
        android: a.targets.android.as_deref().map(|t| t.split('/').next().unwrap_or(t).trim().to_owned()).filter(|t| !t.is_empty()),
        ios: a.targets.ios.as_deref().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()),
    }));
}

/// Move this process into the app's folder (made, owner-only, if it is a workspace app).
pub fn go(app: &App) {
    set_scope(Some(app));
    let dir = app.dir();
    if app.repo.is_none() {
        let _ = crate::filestore::ensure_dir(&dir, true);
    }
    let _ = std::env::set_current_dir(&dir);
}

/// Older fastpilot folders in the home folder (`.fastpilot` inside), not listed yet: searched only
/// when the person asks. A few levels deep; Library, hidden folders and dependency folders are
/// skipped.
pub fn find_older() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let listed: Vec<PathBuf> = list().into_iter().filter_map(|a| a.repo).collect();
    let mut found = Vec::new();
    let mut stack = vec![(home.clone(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if found.len() >= 50 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        if dir != home && is_project(&dir) {
            if let Ok(real) = std::fs::canonicalize(&dir)
                && !listed.contains(&real)
            {
                found.push(real);
            }
            continue;
        }
        if depth >= 4 {
            continue;
        }
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let skip = name.starts_with('.') || ["Library", "node_modules", "target", "Applications", "fastpilot"].contains(&name.as_str());
            if !skip && e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push((e.path(), depth + 1));
            }
        }
    }
    found.sort();
    found
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A name for an app from its address: "https://staging.shop.example" → "Shop staging",
/// "http://127.0.0.1:8080" → "Your website", "file:///…/demo/login.html" → "Demo",
/// "com.shop.app" → "Shop".
fn name_from(t: &Targets) -> String {
    let cap = |w: &str| w.chars().take(1).flat_map(char::to_uppercase).chain(w.chars().skip(1)).collect::<String>();
    if let Some(u) = t.web.as_deref().and_then(|w| url::Url::parse(w).ok()) {
        if u.scheme() == "file" {
            let dir = u.path_segments().and_then(|p| p.rev().nth(1).map(str::to_owned)).unwrap_or_default();
            let dir = percent_decode(&dir);
            return if dir.is_empty() { "Local page".into() } else { cap(&dir) };
        }
        return match u.host() {
            Some(url::Host::Domain(d)) if d != "localhost" => {
                let parts: Vec<&str> = d.split('.').filter(|p| *p != "www").collect();
                let words = if parts.len() > 1 { &parts[..parts.len() - 1] } else { &parts[..] };
                if words.len() > 1 { format!("{} {}", cap(words[1]), words[0]) } else { cap(words.first().copied().unwrap_or("Website")) }
            }
            // A local address or a bare IP reads like a technical value (U1): the switcher shows
            // the address under the name.
            Some(_) => "Your website".into(),
            None => "Website".into(),
        };
    }
    let id = t.android.as_deref().or(t.ios.as_deref()).unwrap_or("");
    let segs: Vec<&str> = id.split('.').filter(|p| !p.is_empty()).collect();
    let last = match segs.as_slice() {
        [.., a, b] if ["app", "android", "ios", "mobile", "client"].contains(b) => a,
        [.., b] => b,
        [] => "App",
    };
    cap(last)
}

/// The name picked for a phone app id (`com.shop.app` → "Shop").
pub fn name_from_id(id: &str) -> String {
    name_from(&Targets { android: Some(id.to_owned()), ..Default::default() })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).chars().filter(|c| !c.is_control()).take(60).collect()
}

fn check_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("give the app a name, for example \"Shop staging\"".into());
    }
    if n.chars().count() > 60 || n.chars().any(char::is_control) {
        return Err("the name is too long or has odd characters: use up to 60 letters, digits and spaces".into());
    }
    Ok(n.to_owned())
}

/// At least one platform; a website is an http(s) or file address, a phone app an app id such as
/// com.shop.app.
fn check_targets(t: Targets) -> Result<Targets, String> {
    let clean = |v: Option<String>| v.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let t = Targets { web: clean(t.web), android: clean(t.android), ios: clean(t.ios) };
    if t.web.is_none() && t.android.is_none() && t.ios.is_none() {
        return Err("add a website address, an Android app or an iPhone app".into());
    }
    let t = Targets { web: t.web.map(|w| check_web(&w)).transpose()?, ..t };
    for (what, v) in [("Android", &t.android), ("iPhone", &t.ios)] {
        if let Some(id) = v
            && !app_id(id)
        {
            return Err(format!("\"{}\" is not an {what} app id: it looks like com.shop.app", id.chars().take(80).collect::<String>()));
        }
    }
    Ok(t)
}

/// A website: an http(s) address with a host and no user name or password, or a local file
/// (`file:///…`, no host). Parsed like a browser does (WHATWG), and stored as parsed, so the host S-1
/// binds secrets to is the host the browser opens (Security, #180 L2).
fn check_web(w: &str) -> Result<String, String> {
    let shown = || w.chars().take(80).collect::<String>();
    let bad = || format!("\"{}\" is not a website address: start it with https://", shown());
    if w.len() > 2000 || w.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(bad());
    }
    let u = url::Url::parse(w).map_err(|_| bad())?;
    match u.scheme() {
        "http" | "https" => {
            if u.host_str().is_none_or(str::is_empty) {
                return Err(bad());
            }
            // Any "@" before the host, even with an empty user name, is a login part (#180 L2).
            let authority = w.split_once("://").map(|(_, r)| r.split(['/', '?', '#']).next().unwrap_or("")).unwrap_or("");
            if !u.username().is_empty() || u.password().is_some() || authority.contains('@') {
                return Err("leave the user name and password out of the address: fastpilot asks for the password when a test needs it".into());
            }
        }
        "file" if u.host_str().is_none_or(|h| h.is_empty() || h == "localhost") => {}
        _ => return Err(bad()),
    }
    Ok(u.as_str().to_owned())
}

pub(crate) fn app_id(s: &str) -> bool {
    s.len() <= 200 && s.split('.').count() >= 2 && s.split('.').all(|p| p.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
}

/// "Shop staging" → "shop-staging": lower-case letters and digits, one dash between words.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(40).collect();
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "app".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_comes_from_the_address() {
        let web = |w: &str| name_from(&check_targets(Targets { web: Some(w.into()), ..Default::default() }).expect("ok"));
        assert_eq!(web("https://staging.shop.example"), "Shop staging");
        assert_eq!(web("https://www.tokoku.co"), "Tokoku");
        assert_eq!(web("http://127.0.0.1:8080/login"), "Your website");
        assert_eq!(web("http://localhost:3000"), "Your website");
        assert_eq!(web("file:///Users/me/my%20demo/login.html"), "My demo");
        let phone = |a: &str| name_from(&Targets { android: Some(a.into()), ..Default::default() });
        assert_eq!((phone("com.shop.app"), phone("com.fastpilot.demo")), ("Shop".into(), "Demo".into()));
    }

    #[test]
    fn names_become_stable_ids() {
        assert_eq!(slug("Shop staging"), "shop-staging");
        assert_eq!(slug("  Android demo (v2)!"), "android-demo-v2");
        assert_eq!(slug("日本"), "app");
        assert_eq!(slug(&"a".repeat(90)).len(), 40);
    }

    #[test]
    fn an_app_has_a_name_and_at_least_one_real_platform() {
        assert!(check_name("  ").is_err() && check_name("a\u{7}b").is_err() && check_name(&"x".repeat(61)).is_err());
        assert!(check_targets(Targets::default()).is_err());
        assert!(check_targets(Targets { web: Some("javascript:alert(1)".into()), ..Default::default() }).is_err());
        assert!(check_targets(Targets { web: Some("shop.example".into()), ..Default::default() }).is_err());
        // Parsed like a browser (#180 L2): no host, a login, a hidden host, a remote file share.
        for bad in ["https://", "https://@evil.example", "https://user:pw@shop.example", "https://shop.example%2f@evil.example", "file://evil.example/share", "data:text/html,x", "ftp://shop.example"] {
            assert!(check_web(bad).is_err(), "{bad}");
        }
        assert_eq!(check_web("file:///Users/me/demo/login.html").as_deref(), Ok("file:///Users/me/demo/login.html"));
        assert_eq!(check_web("HTTPS://Shop.Example:8443/a b").ok(), None, "whitespace is refused");
        assert_eq!(check_web("HTTPS://Shop.Example:8443/x").as_deref(), Ok("https://shop.example:8443/x"));
        assert!(check_targets(Targets { android: Some("../../etc".into()), ..Default::default() }).is_err());
        assert!(check_targets(Targets { ios: Some("Shop".into()), ..Default::default() }).is_err());
        let t = check_targets(Targets { web: Some(" https://staging.shop.example ".into()), android: Some("com.shop.app".into()), ios: Some(String::new()) }).expect("ok");
        assert_eq!((t.web.as_deref(), t.android.as_deref(), t.ios.as_deref()), (Some("https://staging.shop.example/"), Some("com.shop.app"), None));
        assert_eq!(name_from(&t), "Shop staging");
        let a = App { id: "x".into(), name: "x".into(), targets: t, repo: None, created_ms: 0, secrets: Vec::new(), demo: false };
        assert_eq!(a.kinds(), ["web", "android"]);
    }
}
