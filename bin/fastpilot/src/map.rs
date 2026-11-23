//! PF1 (v0.6): the App map. fastpilot walks through an app like a visitor and draws every screen
//! it finds, then shows which screens have a saved test.
//!
//! The exploration is read-only and runs without the AI:
//! - web: a read-only guard in the browser refuses every request that is not GET or HEAD and every
//!   page load outside the app, before it is sent, and counts them (Security review, High);
//! - it never types and never submits a form: it presses links, tabs and menu items, and only the
//!   buttons that move around (Next, Menu, More, Details, Help…); a link's address is checked first;
//! - it never presses a blocked word (Pay…), a word that asks for confirmation (Delete…) or a word
//!   that could change something (Sign out, Send, Save…); each skipped button is named;
//! - it stops at a sign-in screen (a password field, or only sign-in buttons), unless the project
//!   notes name a test account (an email and a `${SECRET}`); then it signs in, only on the start
//!   screen or one reached by a sign-in link, and only on a form with one text and one password field;
//! - it stays inside the app (same site, or the same folder for local files);
//! - at most 60 screens and 30 minutes (defaults 30 and 10), with Stop.
//!
//! What it keeps is a graph of screens (a fingerprint, the title, where it is without IDs, the
//! labels that lead on) and one thumbnail each, in fastpilot's own folder on this computer
//! (`~/.fastpilot/projects/<project>/map/`), never in the project, so a cloned project cannot bring
//! a map. No page text beyond titles and labels, which are cleaned (secrets, emails, tokens and
//! links masked). Titles and labels are text from the app: data, never instructions.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use fastpilot_core::{Driver, Platform, Selector, Snapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

type Res<T> = Result<T, String>;

/// Roles that can lead to another screen.
const NAV_ROLES: &[&str] = &["link", "button", "tab", "menuitem"];
/// Roles that hold typed input: a screen with one has a form.
const FIELD_ROLES: &[&str] = &["textfield", "textbox", "searchbox", "combobox", "spinbutton"];
/// Words that could change something, on top of the blocked and confirm lists.
const CHANGE_WORDS: &[&str] = &[
    "logout", "log out", "sign out", "signout", "keluar", "delete", "remove", "hapus", "submit", "send", "kirim", "save", "simpan", "reset", "clear",
    "archive", "approve", "reject", "confirm", "unsubscribe", "cancel", "order", "pesan", "upload", "pay", "bayar", "buy", "beli", "checkout",
    "transfer", "publish", "post", "update", "edit", "allow", "permission", "permissions", "izin", "grant",
];
/// Labels of the button that signs in (used only with a test account from the project notes).
const SIGN_IN_WORDS: &[&str] = &["sign in", "log in", "login", "masuk", "sign-in", "signin", "sign up", "register", "daftar", "continue with", "sign in with", "log in with"];
/// The only buttons it presses: ones that move around (Security review, High). Links, tabs and
/// menu items are pressed after their own checks.
const MOVE_WORDS: &[&str] = &[
    "next", "previous", "back", "menu", "more", "details", "detail", "help", "open", "view", "show", "info", "about", "home", "lanjut", "berikutnya",
    "kembali", "bantuan", "lihat", "buka", "beranda",
];
/// Words in a link's address that could change something (on top of the Ship check's rules).
const LINK_WORDS: &[&str] = &["leave", "logout", "log-out", "signout", "sign-out", "delete", "remove", "unsubscribe", "cancel", "keluar", "hapus", "pay", "checkout", "order", "confirm"];
/// At most this many actions are tried on one screen.
const PER_SCREEN: usize = 20;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Step {
    pub role: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Screen {
    pub id: String,
    pub title: String,
    /// Web: the address without its query; mobile: the app and activity.
    pub location: String,
    pub fingerprint: String,
    /// A sign-in screen (it has a password field).
    #[serde(default)]
    pub sign_in: bool,
    #[serde(default)]
    pub thumb: bool,
    /// How to get here from the start (labels pressed; `sign-in` = signed in with the test account).
    #[serde(default)]
    pub path: Vec<Step>,
    /// The labels that can be pressed here (at most 20, cleaned).
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Skipped {
    pub screen: String,
    pub label: String,
    pub why: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppMap {
    pub target: String,
    pub platform: String,
    /// A short name for the app (the start screen's title, or the address).
    pub name: String,
    pub started_ms: u64,
    pub finished_ms: u64,
    /// `mapping`, `finished`, `stopped`, `limit`, `could_not_open`, `failed`.
    pub state: String,
    pub message: String,
    /// The sign-in screen it did not pass (no test account in the project notes).
    #[serde(default)]
    pub wall: Option<String>,
    #[serde(default)]
    pub signed_in: bool,
    /// S1: the person chose "Skip this page" for the sign-in screen: the map stays as it is.
    #[serde(default)]
    pub wall_skipped: bool,
    /// Web: requests the read-only guard refused (they would have changed something).
    #[serde(default)]
    pub blocked: usize,
    pub max_screens: usize,
    pub max_minutes: u64,
    /// The screen being looked at now (while mapping).
    #[serde(default)]
    pub current: String,
    pub screens: Vec<Screen>,
    pub edges: Vec<Edge>,
    pub skipped: Vec<Skipped>,
}

pub struct Options {
    pub target: String,
    pub platform: Platform,
    pub max_screens: usize,
    pub max_minutes: u64,
    pub web: fastpilot_web::LaunchOptions,
}

/// Text from the app, as the map keeps it: secrets masked, no hidden or control characters, at
/// most 80 characters. It is data, never an instruction.
pub fn clean(s: &str) -> String {
    // The Jira cleaner (Security review, Low 3): secrets, common token shapes, emails and links
    // masked, no hidden or control characters, one line.
    let s: String = fastpilot_core::jira::clean(s, "", true, 400).chars().filter(|c| !fastpilot_core::policy::invisible(*c)).collect();
    s.chars().take(80).collect()
}

/// A web address without its query (secrets live there) and without user name or password.
fn place(location: &str) -> String {
    let (base, frag) = location.split_once('#').map_or((location, None), |(b, f)| (b, Some(f)));
    let base = base.split('?').next().unwrap_or(base);
    let base = match base.split_once("://") {
        Some((scheme, rest)) => {
            let (auth, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
            let host = auth.rsplit('@').next().unwrap_or(auth);
            if path.is_empty() { format!("{scheme}://{host}") } else { format!("{scheme}://{host}/{path}") }
        }
        None => base.to_owned(),
    };
    // A route in the fragment (`#/cart`) is part of the screen; other fragments are not.
    match frag.filter(|f| f.starts_with('/') || f.starts_with('!')) {
        Some(f) => format!("{base}#{}", f.split('?').next().unwrap_or(f)),
        None => base,
    }
}

/// `/orders/1234/items/7f3a…` → `/orders/:id/items/:id`.
fn without_ids(path: &str) -> String {
    path.split('/')
        .map(|seg| {
            let digits = seg.chars().filter(char::is_ascii_digit).count();
            if !seg.is_empty() && (seg.chars().all(|c| c.is_ascii_digit()) || (seg.len() >= 16 && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && digits >= 4)) {
                ":id".to_owned()
            } else {
                seg.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn heading(s: &Snapshot) -> Option<String> {
    s.entries.iter().find(|e| e.node.role == "heading" && !e.node.name.trim().is_empty()).map(|e| clean(&e.node.name))
}

/// What makes a screen this screen: web, the address without IDs plus the main heading; mobile,
/// the activity plus the first text and the first three button labels.
fn fingerprint(s: &Snapshot, platform: Platform) -> String {
    let words = |t: &str| t.to_lowercase().chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect::<String>();
    match platform {
        Platform::Web => format!("{}|{}", without_ids(&place(&s.location)), words(&heading(s).unwrap_or_default())),
        _ => {
            let first = s.entries.iter().find(|e| e.node.role == "text" && !e.node.name.trim().is_empty()).map(|e| words(&clean(&e.node.name))).unwrap_or_default();
            let buttons: Vec<String> = s.entries.iter().filter(|e| e.node.role == "button").take(3).map(|e| words(&clean(&e.node.name))).collect();
            format!("{}|{first}|{}", s.location, buttons.join(","))
        }
    }
}

/// The main heading; on a phone (no headings) the first text; else the window title.
fn title_of(s: &Snapshot) -> String {
    heading(s)
        .filter(|h| !h.is_empty())
        .or_else(|| s.entries.iter().find(|e| e.node.role == "text" && !e.node.name.trim().is_empty()).map(|e| clean(&e.node.name)))
        .or_else(|| Some(clean(&s.title)).filter(|t| !t.is_empty()))
        .unwrap_or_else(|| clean(s.location.rsplit('/').next().unwrap_or("Screen")))
}

/// FNV-1a: short, stable ids for screens and file names.
fn short_hash(s: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

/// The map's file name for a target: readable letters plus a hash, never a path.
pub fn slug(target: &str) -> String {
    let base: String = target.rsplit(['/', '\\']).find(|p| !p.is_empty()).unwrap_or("app").chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let base = base.trim_matches('-').chars().take(40).collect::<String>();
    format!("{}-{}", if base.is_empty() { "app" } else { &base }, short_hash(target))
}

fn has_word(label: &str, words: &[&str]) -> Option<String> {
    let l = format!(" {} ", label.to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '-').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "));
    words.iter().find(|w| l.contains(&format!(" {w} "))).map(|w| (*w).to_owned())
}

/// Why fastpilot does not press `label` on a screen (`form`: the screen has a field), or `None`.
pub fn skip_reason(role: &str, label: &str, form: bool) -> Option<String> {
    if fastpilot_core::policy::block_match(label, &fastpilot_core::policy::blocked_words()).is_some() {
        return Some("blocked by limits".into());
    }
    if fastpilot_core::policy::risky_match(label, &fastpilot_core::policy::risky_words()).is_some() {
        return Some("asks for confirmation".into());
    }
    if let Some(w) = has_word(label, CHANGE_WORDS) {
        return Some(format!("could change something (\"{w}\")"));
    }
    if form && role != "link" {
        return Some("on a screen with a form, only links are followed".into());
    }
    if role != "link" && role != "tab" && role != "menuitem" && has_word(label, MOVE_WORDS).is_none() {
        return Some("a button that may change something (only buttons that move around are pressed)".into());
    }
    None
}

/// A sign-in screen (Security review, Low 4): a password field, or a screen whose only ways on are
/// sign-in buttons ("Sign in with Google", "Continue with email").
fn is_sign_in(s: &Snapshot) -> bool {
    if s.entries.iter().any(|e| e.node.secure) {
        return true;
    }
    let ways: Vec<&str> = s.entries.iter().filter(|e| NAV_ROLES.contains(&e.node.role.as_str()) && !e.node.name.trim().is_empty()).map(|e| e.node.name.as_str()).collect();
    let sign: Vec<&&str> = ways.iter().filter(|w| has_word(w, SIGN_IN_WORDS).is_some()).collect();
    !sign.is_empty() && ways.iter().all(|w| has_word(w, SIGN_IN_WORDS).is_some() || has_word(w, &["back", "kembali", "home", "help", "bantuan", "privacy", "terms"]).is_some())
}

/// The browser's own page for a refused request ("… is blocked", ERR_BLOCKED_BY_CLIENT).
fn block_page(s: &Snapshot) -> bool {
    s.location.starts_with("chrome-error://")
        || s.title.contains("is blocked")
        || s.entries.iter().any(|e| e.node.name.contains("ERR_BLOCKED_BY_CLIENT") || (e.node.role == "heading" && e.node.name.contains("is blocked")))
}

/// The start as it is shown: secrets masked, no user name or password, no query.
fn shown_target(target: &str) -> String {
    let t = if target.contains("://") { place(target) } else { target.to_owned() };
    fastpilot_core::change::printable(&fastpilot_core::redact(&t)).chars().take(200).collect()
}

/// Inside the app: `inside` is `scheme://host/` or the folder `file:///…/demo/`.
fn in_app(url: &str, inside: &str) -> bool {
    url.starts_with(inside) || url == inside.trim_end_matches('/')
}

/// A link's `href` made absolute against the page it is on.
fn join(page: &str, href: &str) -> String {
    let href = href.trim();
    if href.contains("://") || href.starts_with("javascript:") || href.starts_with("mailto:") || href.starts_with("tel:") || href.starts_with("data:") {
        return href.to_owned();
    }
    let page = page.split('#').next().unwrap_or(page);
    if let Some(frag) = href.strip_prefix('#') {
        return format!("{page}#{frag}");
    }
    let (scheme, rest) = page.split_once("://").unwrap_or(("file", page));
    if href.starts_with("//") {
        return format!("{scheme}:{href}");
    }
    let host_end = if scheme == "file" { 0 } else { rest.find('/').unwrap_or(rest.len()) };
    let (host, path) = rest.split_at(host_end);
    if href.starts_with('/') {
        return format!("{scheme}://{host}{href}");
    }
    let path = path.split('?').next().unwrap_or(path);
    let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
    format!("{scheme}://{host}{dir}/{href}")
}

/// Why a link is not opened, from its address alone (before anything is sent).
fn link_refusal(url: &str, inside: &str) -> Option<String> {
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file://")) {
        return Some("not a page address (a script, mail or phone link)".into());
    }
    if !in_app(&place(url), inside) {
        return Some("leads outside the app".into());
    }
    // A local file cannot change a server: only the words after the app's folder count there.
    if !url.starts_with("file://")
        && let Some(why) = fastpilot_core::ship::reopen_refusal(url)
    {
        return Some(format!("its address could change something ({why})"));
    }
    let full = place(url).to_lowercase();
    let path = full.strip_prefix(&inside.to_lowercase()).unwrap_or(&full).to_owned();
    let words: Vec<&str> = path.split(|c: char| !c.is_ascii_alphanumeric() && c != '-').collect();
    LINK_WORDS.iter().find(|w| words.contains(w)).map(|w| format!("its address could change something (\"{w}\")"))
}

/// The test account from the project notes: an email and a `${SECRET}` whose value this process
/// has (a name listed in FASTPILOT_VARS). Nothing else in the notes is used.
pub fn test_account() -> Option<(String, String)> {
    test_account_of().map(|(u, p, _)| (u, p))
}

/// S1 (v0.7): the current app's own test account first, as saved in the Keychain (E2: Settings ›
/// Secrets, or asked once on the Map page): `TEST_USER` and `TEST_PASSWORD`. Else the project
/// notes, as before. The flag says it is the app's: then S-1 holds when signing in (only on the
/// app's own site, or inside the app itself on a phone).
pub fn test_account_of() -> Option<(String, String, bool)> {
    if let (Some(u), Some(p)) = (fastpilot_core::secrets::lookup("TEST_USER"), fastpilot_core::secrets::lookup("TEST_PASSWORD")) {
        return Some((u, p, true));
    }
    notes_account().map(|(u, p)| (u, p, false))
}

fn notes_account() -> Option<(String, String)> {
    let allowed: Vec<String> = fastpilot_core::config::var("FASTPILOT_VARS").unwrap_or_default().split(',').map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).collect();
    for n in fastpilot_memory::notes::Notes::from_env().list() {
        if n.problem.is_some() {
            continue;
        }
        let email = n.text.split(|c: char| c.is_whitespace() || ",;()<>\"'".contains(c)).find(|w| w.contains('@') && w.split('@').nth(1).is_some_and(|d| d.contains('.'))).map(|w| w.trim_end_matches('.').to_owned());
        let secret = n.text.split("${").nth(1).and_then(|r| r.split('}').next()).filter(|name| allowed.iter().any(|a| a == name)).and_then(|name| std::env::var(name).ok()).filter(|v| !v.is_empty());
        if let (Some(e), Some(s)) = (email, secret) {
            return Some((e, s));
        }
    }
    None
}

/// Where the map files live: fastpilot's own folder for this project
/// (`~/.fastpilot/projects/<sha256 of the project folder>/map`), never the project (Security
/// review, Low 2): a cloned project cannot bring a map.
pub fn dir() -> PathBuf {
    let project = std::env::current_dir().unwrap_or_default();
    let ship = fastpilot_core::ship::speed_dir(&project);
    ship.parent().map(|p| p.join("map")).unwrap_or_else(|| ship.join("map"))
}

/// Writes `bytes` to `dir/name` without following a link: a new private file, then a rename.
fn write_safe(dir: &Path, name: &str, bytes: &[u8]) -> Res<()> {
    use std::io::Write;
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("bad file name".into());
    }
    for d in [dir.parent().and_then(Path::parent), dir.parent(), Some(dir)].into_iter().flatten() {
        if std::fs::symlink_metadata(d).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("{} is a link; the map is not written through links", d.display()));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(".{name}.{}.tmp", short_hash(&format!("{:?}{}", Instant::now(), std::process::id()))));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&tmp).map_err(|e| format!("cannot write the map: {e}"))?;
    f.write_all(bytes).map_err(|e| format!("cannot write the map: {e}"))?;
    drop(f);
    std::fs::rename(&tmp, dir.join(name)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write the map: {e}")
    })
}

pub fn save(m: &AppMap) -> Res<()> {
    let text = serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?;
    write_safe(&dir(), &format!("{}.json", slug(&m.target)), &text)
}

/// The most recently mapped app (v1 shows one map at a time; one file per target).
pub fn latest() -> Option<AppMap> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json") && std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file() && m.len() < 4 * 1024 * 1024))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    found.sort();
    let (_, p) = found.pop()?;
    let m: AppMap = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
    Some(m)
}

/// The thumbnail of screen `id` of the map of `target`, if there is one.
pub fn thumb(target: &str, id: &str) -> Option<Vec<u8>> {
    if id.len() != 8 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let p = dir().join(slug(target)).join(format!("{id}.jpg"));
    if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
        return None;
    }
    std::fs::read(p).ok()
}

fn now_ms() -> u64 {
    fastpilot_core::change::now_ms()
}

// ---- exploring ----------------------------------------------------------------------------

struct Explorer<'a> {
    d: &'a dyn Driver,
    /// Web: the page itself, for the read-only guard and link addresses.
    web: Option<&'a fastpilot_web::WebPage>,
    platform: Platform,
    start: String,
    /// Web: what "inside the app" starts with (the site, or the folder of a local file).
    inside: String,
    map: Arc<Mutex<AppMap>>,
    stop: Arc<AtomicBool>,
    deadline: Instant,
    account: Option<(String, String)>,
    /// The account is the app's own (E2): S-1 is checked before it is typed.
    app_account: bool,
}

impl Explorer<'_> {
    fn with<R>(&self, f: impl FnOnce(&mut AppMap) -> R) -> R {
        f(&mut self.map.lock().unwrap_or_else(|p| p.into_inner()))
    }

    fn over(&self) -> Option<&'static str> {
        if self.stop.load(Ordering::Relaxed) {
            Some("stopped")
        } else if Instant::now() >= self.deadline {
            Some("limit")
        } else {
            None
        }
    }

    async fn settle(&self) {
        let _ = self.d.wait_idle(Duration::from_secs(3)).await;
    }

    async fn snap(&self) -> Res<Snapshot> {
        self.d.snapshot().await.map_err(|e| e.to_string())
    }

    /// Web: the same site or folder. Phone: the app's own package (a system dialog, such as a
    /// permission request, is outside: never explored, closed with Back).
    fn inside_app(&self, s: &Snapshot) -> bool {
        match self.platform {
            Platform::Web => in_app(&place(&s.location), &self.inside),
            _ => s.location.split([',', ' ']).any(|w| w.trim() == self.inside) || s.location.contains(&format!("{}/", self.inside)),
        }
    }

    /// Signs in with the test account from the project notes: the email in the first field, the
    /// secret in the password field, then the sign-in button. The only typing the map ever does.
    async fn sign_in(&self, s: &Snapshot) -> Res<()> {
        let (email, secret) = self.account.clone().ok_or("no test account")?;
        // Only a plain sign-in form (Security review, Low 5): one text field and one password.
        let texts = s.entries.iter().filter(|e| FIELD_ROLES.contains(&e.node.role.as_str()) && !e.node.secure).count();
        let secrets = s.entries.iter().filter(|e| e.node.secure).count();
        if texts != 1 || secrets != 1 {
            return Err("not a plain sign-in form".into());
        }
        let field = s.entries.iter().find(|e| FIELD_ROLES.contains(&e.node.role.as_str()) && !e.node.secure).ok_or("no email field")?;
        let pass = s.entries.iter().find(|e| e.node.secure).ok_or("no password field")?;
        let button = s.entries.iter().find(|e| NAV_ROLES.contains(&e.node.role.as_str()) && has_word(&e.node.name, SIGN_IN_WORDS).is_some()).ok_or("no sign-in button")?;
        // A plain driver snapshot has no refs: the element's id, else its role and name.
        let sel = |n: &fastpilot_core::Node| match &n.id {
            Some(id) if !id.is_empty() => Selector::Id(id.clone()),
            _ => Selector::Role { role: n.role.clone(), name: Some(n.name.clone()) },
        };
        // S-1 (E2, D07-L3): the app's own password goes only into a form on its own site, or into
        // the app itself on a phone; a sign-in on another site (an identity provider) is a wall.
        if self.app_account {
            let at = self.d.url_of(&sel(&pass.node)).await;
            fastpilot_core::secrets::allowed_at(at.as_deref(), self.platform)?;
            if let Some(own) = fastpilot_core::secrets::own_app(self.platform) {
                fastpilot_core::secrets::allowed_in_front(&own, self.d.foreground_is(&own).await)?;
            }
        }
        self.d.type_text(&sel(&field.node), &email).await.map_err(|e| e.to_string())?;
        self.d.type_text(&sel(&pass.node), &secret).await.map_err(|e| e.to_string())?;
        self.d.tap(&sel(&button.node)).await.map_err(|e| e.to_string())?;
        // The app may take a moment to answer: wait until the password field is gone and the
        // screen stops changing.
        let mut last = String::new();
        for _ in 0..15 {
            self.settle().await;
            if let Ok(now) = self.snap().await
                && !now.entries.iter().any(|e| e.node.secure)
            {
                let fp = fingerprint(&now, self.platform);
                if fp == last && now.entries.iter().any(|e| NAV_ROLES.contains(&e.node.role.as_str())) {
                    break;
                }
                last = fp;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        Ok(())
    }

    /// Back to screen `s`: web, its address first; else (or when that shows another screen) the
    /// start again and the same presses.
    async fn reach(&self, s: &Screen) -> Res<Snapshot> {
        if self.platform == Platform::Web && !s.location.is_empty() && !s.location.contains(":id") {
            self.d.open(&s.location).await.map_err(|e| e.to_string())?;
            self.settle().await;
            let now = self.snap().await?;
            if fingerprint(&now, self.platform) == s.fingerprint {
                return Ok(now);
            }
        }
        self.d.open(&self.start).await.map_err(|e| e.to_string())?;
        self.settle().await;
        for step in &s.path {
            if step.role == "sign-in" {
                // Still signed in from before: nothing to do.
                let now = self.snap().await?;
                if now.entries.iter().any(|e| e.node.secure) {
                    self.sign_in(&now).await?;
                }
                continue;
            }
            self.d.tap(&Selector::Role { role: step.role.clone(), name: Some(step.name.clone()) }).await.map_err(|e| e.to_string())?;
            self.settle().await;
        }
        let now = self.snap().await?;
        if fingerprint(&now, self.platform) == s.fingerprint { Ok(now) } else { Err("the screen looks different now".into()) }
    }

    /// Adds the screen `s` reached by `path` (or finds it). `Some(index)`, and whether it is new.
    async fn add(&self, s: &Snapshot, path: Vec<Step>) -> (usize, bool) {
        let fp = fingerprint(s, self.platform);
        if let Some(i) = self.with(|m| m.screens.iter().position(|x| x.fingerprint == fp)) {
            return (i, false);
        }
        let id = short_hash(&fp);
        let labels: Vec<String> = s.entries.iter().filter(|e| NAV_ROLES.contains(&e.node.role.as_str()) && !e.node.name.trim().is_empty()).map(|e| clean(&e.node.name)).take(PER_SCREEN).collect();
        let screen = Screen {
            id: id.clone(),
            title: title_of(s),
            location: if self.platform == Platform::Web { without_ids(&place(&s.location)) } else { clean(&s.location) },
            fingerprint: fp,
            sign_in: is_sign_in(s),
            thumb: false,
            path,
            labels,
        };
        let target = self.with(|m| m.target.clone());
        let thumb = match self.d.screenshot().await {
            Ok(bytes) => write_safe(&dir().join(slug(&target)), &format!("{id}.jpg"), &bytes).is_ok(),
            Err(_) => false,
        };
        let i = self.with(|m| {
            m.screens.push(Screen { thumb, ..screen });
            m.screens.len() - 1
        });
        let snapshot = self.with(|m| m.clone());
        let _ = save(&snapshot);
        (i, true)
    }

    async fn run(&self) -> Res<()> {
        self.d.open(&self.start).await.map_err(|e| format!("could_not_open: {e}"))?;
        self.settle().await;
        let mut first = self.snap().await.map_err(|e| format!("could_not_open: {e}"))?;
        // A system dialog left open over the app: close it (Back), never press anything in it.
        if self.platform != Platform::Web && !self.inside_app(&first) {
            let _ = self.d.back().await;
            self.settle().await;
            self.d.open(&self.start).await.map_err(|e| format!("could_not_open: {e}"))?;
            self.settle().await;
            first = self.snap().await.map_err(|e| format!("could_not_open: {e}"))?;
            if !self.inside_app(&first) {
                return Err("could_not_open: the app did not come to the front (a system screen is in the way)".into());
            }
        }
        let name = title_of(&first);
        self.with(|m| m.name = name);
        let (_, _) = self.add(&first, Vec::new()).await;
        let max = self.with(|m| m.max_screens);
        let mut queue: VecDeque<usize> = VecDeque::from([0]);
        while let Some(i) = queue.pop_front() {
            if let Some(why) = self.over() {
                return Err(why.into());
            }
            let screen = self.with(|m| m.screens[i].clone());
            self.with(|m| m.current = screen.title.clone());
            let Ok(here) = self.reach(&screen).await else { continue };
            let form = here.entries.iter().any(|e| FIELD_ROLES.contains(&e.node.role.as_str()) || e.node.secure);
            // A sign-in screen: with a test account, sign in once and go on from there; without
            // one, this is as far as the map goes on this path.
            if screen.sign_in {
                let signed = self.with(|m| m.signed_in);
                // Only on the start screen or one reached by a sign-in link (Security review, Low 5).
                let by_sign_in_link = screen.path.last().is_none_or(|st| has_word(&st.name, SIGN_IN_WORDS).is_some());
                if self.account.is_some() && !signed && by_sign_in_link && self.sign_in(&here).await.is_ok() {
                    let after = self.snap().await?;
                    if fingerprint(&after, self.platform) != screen.fingerprint && self.inside_app(&after) {
                        let mut path = screen.path.clone();
                        path.push(Step { role: "sign-in".into(), name: "Sign in with the test account".into() });
                        let (j, new) = self.add(&after, path).await;
                        self.with(|m| {
                            m.signed_in = true;
                            m.edges.push(Edge { from: screen.id.clone(), to: m.screens[j].id.clone(), label: "Sign in (test account)".into() });
                        });
                        if new {
                            queue.push_back(j);
                        }
                    }
                }
                if !self.with(|m| m.signed_in) {
                    self.with(|m| m.wall.get_or_insert(screen.title.clone()).clone());
                }
                // A sign-in screen is a wall (Security review, Low 4): nothing else on it is pressed.
                continue;
            }
            let mut tried: Vec<(String, String)> = Vec::new();
            let actions: Vec<(String, String, Option<i64>)> = here
                .entries
                .iter()
                .filter(|e| NAV_ROLES.contains(&e.node.role.as_str()) && !e.node.disabled && !e.node.name.trim().is_empty())
                .map(|e| (e.node.role.clone(), e.node.name.trim().to_owned(), e.node.handle))
                .collect();
            let mut at_here = true;
            for (role, label, handle) in actions {
                if tried.contains(&(role.clone(), label.clone())) || tried.len() >= PER_SCREEN {
                    continue;
                }
                tried.push((role.clone(), label.clone()));
                if screen.sign_in && has_word(&label, SIGN_IN_WORDS).is_some() {
                    continue; // signing in is done above, with the test account only
                }
                let mut why = skip_reason(&role, &label, form);
                // A web link's address is checked before it is opened.
                if why.is_none()
                    && role == "link"
                    && let (Some(web), Some(h)) = (self.web, handle)
                    && let Some(href) = web.link_target(h).await
                {
                    why = link_refusal(&join(&here.location, &href), &self.inside);
                }
                if let Some(why) = why {
                    self.with(|m| m.skipped.push(Skipped { screen: screen.title.clone(), label: clean(&label), why }));
                    continue;
                }
                if let Some(why) = self.over() {
                    return Err(why.into());
                }
                if !at_here && self.reach(&screen).await.is_err() {
                    break;
                }
                at_here = false;
                let blocked_before = self.web.map_or(0, |w| w.blocked());
                if self.d.tap(&Selector::Role { role: role.clone(), name: Some(label.clone()) }).await.is_err() {
                    continue;
                }
                self.settle().await;
                let Ok(after) = self.snap().await else { continue };
                // The guard refused what this press sent, and the browser shows its block page: a
                // press that would have changed something, not a screen.
                if self.web.is_some_and(|w| w.blocked() > blocked_before) && block_page(&after) {
                    self.with(|m| m.skipped.push(Skipped { screen: screen.title.clone(), label: clean(&label), why: "it would have sent something (the request was blocked)".into() }));
                    continue;
                }
                if !self.inside_app(&after) {
                    self.with(|m| m.skipped.push(Skipped { screen: screen.title.clone(), label: clean(&label), why: "leads outside the app".into() }));
                    if self.platform != Platform::Web {
                        let _ = self.d.back().await;
                        self.settle().await;
                    }
                    continue;
                }
                let fp = fingerprint(&after, self.platform);
                if fp == screen.fingerprint {
                    at_here = true;
                    continue;
                }
                let known = self.with(|m| m.screens.iter().position(|x| x.fingerprint == fp));
                if known.is_none() && self.with(|m| m.screens.len()) >= max {
                    return Err("limit".into());
                }
                let mut path = screen.path.clone();
                path.push(Step { role: role.clone(), name: label.clone() });
                let (j, new) = self.add(&after, path).await;
                self.with(|m| {
                    let to = m.screens[j].id.clone();
                    if !m.edges.iter().any(|e| e.from == screen.id && e.to == to) {
                        m.edges.push(Edge { from: screen.id.clone(), to, label: clean(&label) });
                    }
                });
                if new {
                    queue.push_back(j);
                }
            }
        }
        Ok(())
    }
}

/// Maps the app: opens the target on its platform, explores, saves the map as it goes, and
/// returns it. `progress` is shared with the dashboard while it runs; `stop` ends it early.
pub async fn explore(opts: Options, progress: Arc<Mutex<AppMap>>, stop: Arc<AtomicBool>) -> AppMap {
    let started = now_ms();
    {
        let mut m = progress.lock().unwrap_or_else(|p| p.into_inner());
        *m = AppMap {
            target: opts.target.clone(),
            platform: opts.platform.to_string(),
            name: opts.target.clone(),
            started_ms: started,
            state: "mapping".into(),
            max_screens: opts.max_screens.clamp(1, 60),
            max_minutes: opts.max_minutes.clamp(1, 30),
            ..Default::default()
        };
    }
    let deadline = Instant::now() + Duration::from_secs(opts.max_minutes.clamp(1, 30) * 60);
    let outcome = drive(&opts, progress.clone(), stop, deadline).await;
    let mut m = progress.lock().unwrap_or_else(|p| p.into_inner()).clone();
    m.finished_ms = now_ms();
    m.current.clear();
    let n = m.screens.len();
    // Only what was counted is claimed (Security review, High).
    let tested_note = if m.platform == "web" {
        if m.blocked == 0 {
            "Nothing was typed in or submitted".to_owned()
        } else {
            format!("{} request{} that would have changed something {} blocked, and nothing was typed in", m.blocked, if m.blocked == 1 { "" } else { "s" }, if m.blocked == 1 { "was" } else { "were" })
        }
    } else {
        "Nothing was typed in, and only links and buttons that move around were pressed".to_owned()
    };
    m.state = match &outcome {
        Ok(()) => "finished".into(),
        Err(e) if e == "stopped" => "stopped".into(),
        Err(e) if e == "limit" => "limit".into(),
        Err(e) if e.starts_with("could_not_open") => "could_not_open".into(),
        Err(_) => "failed".into(),
    };
    // QA re-test Low 5: the count is the named list, all of it.
    let skipped = m.skipped.len();
    m.message = match m.state.as_str() {
        "finished" => format!(
            "Mapped {n} screen{} in {}. {skipped} button{} and link{} not pressed (each is named, with why). {}.",
            if n == 1 { "" } else { "s" },
            minutes(m.finished_ms.saturating_sub(started)),
            if skipped == 1 { "" } else { "s" },
            if skipped == 1 { " was" } else { "s were" },
            if m.signed_in { format!("It signed in with the test account. {tested_note}") } else { tested_note }
        ),
        "stopped" => format!("Stopped after {n} screen{}. The map shows what was found. Screens it did not reach are not on it. {tested_note}.", if n == 1 { "" } else { "s" }),
        "limit" => format!("Stopped at {n} screens or {} minutes, the limit. Your app may have more. Raise the limit or start from a different screen.", m.max_minutes),
        "could_not_open" => format!("Could not open {}. Check the address and that the app is running. Nothing was mapped.", shown_target(&opts.target)),
        _ => format!("Mapping stopped after {n} screen{}: {}. The screens found are kept.", if n == 1 { "" } else { "s" }, clean(outcome.as_ref().err().map_or("", |e| e.as_str()))),
    };
    if let Some(w) = &m.wall {
        // S1/M2: the page asks there and then (sign in with the test account, or skip); no Project notes step.
        m.message.push_str(&format!(" \"{w}\" needs a sign-in, so the pages behind it are not on the map yet."));
    }
    *progress.lock().unwrap_or_else(|p| p.into_inner()) = m.clone();
    if m.state != "could_not_open" {
        let _ = save(&m);
    }
    m
}

fn minutes(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 { format!("{s} s") } else { format!("{} min {} s", s / 60, s % 60) }
}

async fn drive(opts: &Options, map: Arc<Mutex<AppMap>>, stop: Arc<AtomicBool>, deadline: Instant) -> Res<()> {
    let (account, app_account) = match test_account_of() {
        Some((u, p, app)) => (Some((u, p)), app),
        None => (None, false),
    };
    match opts.platform {
        Platform::Web => {
            let browser = fastpilot_web::Browser::launch(opts.web.clone()).await.map_err(|e| format!("could_not_open: {e}"))?;
            let page = browser.new_page().await.map_err(|e| format!("could_not_open: {e}"))?;
            let start = web_target(&opts.target);
            let inside = inside_of(&start);
            // The read-only guard is on before the first page loads.
            page.read_only(&inside).await.map_err(|e| format!("could_not_open: {e}"))?;
            let ex = Explorer { d: &page, web: Some(&page), platform: Platform::Web, start, inside, map: map.clone(), stop, deadline, account, app_account };
            let r = ex.run().await;
            map.lock().unwrap_or_else(|p| p.into_inner()).blocked = page.blocked();
            r
        }
        p => {
            let pool = crate::pool::Pool::discover(&[p]).await;
            let leased = tokio::time::timeout(Duration::from_secs(120), pool.lease(&[p], &[])).await.map_err(|_| "could_not_open: the device is busy with other tests".to_owned())?;
            let devices = leased.map_err(|e| format!("could_not_open: {e}"))?;
            let id = devices.first().map(|d| d.id.clone()).ok_or("could_not_open: no device")?;
            let driver: Box<dyn Driver> = match p {
                Platform::Android => Box::new(fastpilot_android::AndroidDriver::start(Some(&id)).await.map_err(|e| format!("could_not_open: {e}"))?),
                _ => Box::new(fastpilot_ios::IosDriver::start(Some(&id)).await.map_err(|e| format!("could_not_open: {e}"))?),
            };
            let package = opts.target.split('/').next().unwrap_or(&opts.target).trim().to_owned();
            let ex = Explorer { d: driver.as_ref(), web: None, platform: p, start: opts.target.clone(), inside: package, map, stop, deadline, account, app_account };
            let r = ex.run().await;
            pool.release(devices);
            r
        }
    }
}

/// A local file becomes a `file://` address; anything else is used as given.
fn web_target(target: &str) -> String {
    if target.contains("://") {
        return target.to_owned();
    }
    match Path::new(target).canonicalize() {
        Ok(p) => format!("file://{}", p.display()),
        Err(_) => format!("https://{target}"),
    }
}

/// What "inside the app" starts with: `scheme://host/` for a site (so `shop.test.evil.com` is not
/// inside), the folder for a local file.
fn inside_of(start: &str) -> String {
    let p = place(start);
    if let Some(rest) = p.strip_prefix("file://") {
        return format!("file://{}/", rest.rsplit_once('/').map_or(rest, |(d, _)| d));
    }
    match p.split_once("://") {
        Some((scheme, rest)) => format!("{scheme}://{}/", rest.split('/').next().unwrap_or(rest)),
        None => p,
    }
}

// ---- coverage ------------------------------------------------------------------------------

/// The saved tests that reach each screen, from the tests' own start (`target:`) and from the
/// screens their last runs went through (path memory). A failing test still counts as "has a
/// test", shown as failing.
pub fn coverage(map: &AppMap, project: &Path) -> Value {
    let memory = fastpilot_memory::Memory::from_env();
    let remembered = memory.remembered();
    let tests: Vec<(String, String, String)> = crate::ui::test_files(&project.join("tests"))
        .into_iter()
        .filter_map(|(path, title)| {
            let text = std::fs::read_to_string(&path).ok()?;
            let field = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_owned()));
            let id = field("id:").unwrap_or_else(|| Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
            Some((id, title, field("target:").unwrap_or_default()))
        })
        .collect();
    let target_of = |t: &str| -> String {
        if t.is_empty() {
            return String::new();
        }
        if map.platform == "web" {
            let full = if t.contains("://") { t.to_owned() } else { project.join(t).canonicalize().map(|p| format!("file://{}", p.display())).unwrap_or_default() };
            without_ids(&place(&full))
        } else {
            t.to_owned()
        }
    };
    let status_of = |id: &str| match memory.load(id) {
        Some(m) if m.history.last() == Some(&false) => "failing",
        Some(_) => "passing",
        None => "not_run",
    };
    let shared = |loc: &str| map.screens.iter().filter(|s| s.location == loc).count() > 1;
    let mut seen_start: Vec<String> = Vec::new();
    let screens: Vec<Value> = map
        .screens
        .iter()
        .map(|s| {
            let mut list: Vec<(String, String, &str)> = Vec::new();
            // A test starts here: the first screen at its target.
            for (id, title, target) in &tests {
                let t = target_of(target);
                let starts = !t.is_empty() && (if map.platform == "web" { t == s.location } else { s.location.contains(t.as_str()) });
                if starts && !seen_start.contains(id) {
                    seen_start.push(id.clone());
                    list.push((id.clone(), title.clone(), status_of(id)));
                }
            }
            // A test's last run went through it.
            for m in &remembered {
                let hit = m.screens.iter().any(|h| {
                    // `[s1] screen "Title" (address)`: the address the same way the map keeps it.
                    let addr = h.rsplit_once('(').and_then(|(_, r)| r.strip_suffix(')')).unwrap_or("");
                    let at = if map.platform == "web" { without_ids(&place(addr)) == s.location } else { h.contains(&s.location) };
                    at && (!shared(&s.location) || h.contains(&format!("\"{}\"", s.title)))
                });
                if hit && !list.iter().any(|(id, ..)| *id == m.test_id) {
                    let title = tests.iter().find(|(id, ..)| *id == m.test_id).map_or(m.test_id.clone(), |(_, t, _)| t.clone());
                    list.push((m.test_id.clone(), title, status_of(&m.test_id)));
                }
            }
            let status = if list.iter().any(|(_, _, st)| *st == "failing") {
                "failing"
            } else if list.is_empty() {
                "none"
            } else {
                "tested"
            };
            json!({ "id": s.id, "status": status, "tests": list.iter().map(|(id, title, st)| json!({ "id": id, "title": clean(title), "status": st })).collect::<Vec<_>>() })
        })
        .collect();
    let tested = screens.iter().filter(|s| s["status"] != "none").count();
    let failing = screens.iter().filter(|s| s["status"] == "failing").count();
    json!({ "screens": screens, "tested": tested, "failing": failing, "total": map.screens.len() })
}

// ---- dashboard and terminal ----------------------------------------------------------------

struct Running {
    map: Arc<Mutex<AppMap>>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

static RUNNING: LazyLock<Mutex<Option<Running>>> = LazyLock::new(Default::default);

/// A map is being made now.
pub fn is_mapping() -> bool {
    RUNNING.lock().unwrap_or_else(|p| p.into_inner()).as_ref().is_some_and(|r| !r.done.load(Ordering::Relaxed))
}

/// What the Map page shows: the map being made, else the latest one, with its coverage.
pub fn state() -> Value {
    let running = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
    let live = running.as_ref().filter(|r| !r.done.load(Ordering::Relaxed)).map(|r| r.map.lock().unwrap_or_else(|p| p.into_inner()).clone());
    drop(running);
    let mapping = live.is_some();
    let map = live.or_else(latest);
    let project = std::env::current_dir().unwrap_or_default();
    let cov = map.as_ref().map(|m| coverage(m, &project));
    let has_account = test_account().is_some();
    // S1: which part of the app's test account is saved, so the page asks only for what is missing.
    let has_user = fastpilot_core::secrets::lookup("TEST_USER").is_some();
    json!({ "mapping": mapping, "map": map, "coverage": cov, "test_account": has_account, "test_user": has_user, "plan": plan() })
}

/// S1 (v0.7): what one click maps: the current app (E6), on the kind of device that fits it (its
/// website first, else its Android app, else its iPhone app; L062-10: a website is never mapped on
/// the Android emulator by default), from its own start, 30 screens, 10 minutes. `null` without an app.
pub fn plan() -> Value {
    let Some(app) = crate::apps::current() else { return Value::Null };
    let t = &app.targets;
    let kinds: Vec<(&str, &str)> = [("web", &t.web), ("android", &t.android), ("ios", &t.ios)]
        .into_iter()
        .filter_map(|(k, v)| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(|v| (k, v)))
        .collect();
    let Some((platform, target)) = kinds.first().copied() else { return Value::Null };
    json!({
        "app": { "id": app.id, "name": app.name },
        "kinds": kinds.iter().map(|(k, v)| json!({ "platform": k, "target": v })).collect::<Vec<_>>(),
        "platform": platform,
        "target": target,
        "screens": 30,
        "minutes": 10,
    })
}

/// S1: "Skip this page": keep the latest map as it is and stop asking about its sign-in screen.
pub fn skip_wall() -> Res<()> {
    let mut m = latest().ok_or("There is no map yet.")?;
    if m.wall.is_none() {
        return Err("This map has no sign-in page to skip.".into());
    }
    m.wall_skipped = true;
    save(&m)
}

/// Starts mapping in the background (one at a time).
pub fn start(opts: Options) -> Res<()> {
    let mut running = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
    if running.as_ref().is_some_and(|r| !r.done.load(Ordering::Relaxed)) {
        return Err("An app is being mapped already. Stop it first, or wait until it is done.".into());
    }
    let map = Arc::new(Mutex::new(AppMap::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    *running = Some(Running { map: map.clone(), stop: stop.clone(), done: done.clone() });
    drop(running);
    tokio::spawn(async move {
        explore(opts, map, stop).await;
        done.store(true, Ordering::Relaxed);
    });
    Ok(())
}

pub fn stop() -> bool {
    let running = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
    match running.as_ref().filter(|r| !r.done.load(Ordering::Relaxed)) {
        Some(r) => {
            r.stop.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// The start of a map request: a web address or file, or an Android/iOS app.
pub fn options(target: &str, platform: &str, screens: Option<u64>, minutes: Option<u64>, web: fastpilot_web::LaunchOptions) -> Res<Options> {
    let target = target.trim();
    if target.is_empty() || target.len() > 500 || target.chars().any(char::is_control) {
        return Err("Enter where it should start: a web address, or the app on the phone.".into());
    }
    let platform = match platform {
        "" | "web" => Platform::Web,
        "android" => Platform::Android,
        "ios" => Platform::Ios,
        _ => return Err("Choose web, android or ios.".into()),
    };
    if platform == Platform::Web && target.contains("://") && !(target.starts_with("http://") || target.starts_with("https://") || target.starts_with("file://")) {
        return Err("A web address starts with http:// or https://.".into());
    }
    Ok(Options { target: target.to_owned(), platform, max_screens: screens.unwrap_or(30).clamp(1, 60) as usize, max_minutes: minutes.unwrap_or(10).clamp(1, 30), web })
}

/// `fastpilot map <start> [--android | --ios] [--screens N] [--minutes M]`.
pub async fn cli(args: &[String], web: fastpilot_web::LaunchOptions) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let usage = "usage: fastpilot map <web address, file or app> [--android | --ios] [--screens N (30)] [--minutes M (10)] [--headful]";
    let Some(target) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("{usage}");
        std::process::exit(2);
    };
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u64>().ok());
    let platform = if args.iter().any(|a| a == "--android") {
        "android"
    } else if args.iter().any(|a| a == "--ios") {
        "ios"
    } else {
        "web"
    };
    let opts = options(target, platform, value("--screens"), value("--minutes"), web)?;
    let progress = Arc::new(Mutex::new(AppMap::default()));
    let stop = Arc::new(AtomicBool::new(false));
    println!("Mapping {} (read-only; up to {} screens, {} minutes)…", shown_target(target), opts.max_screens, opts.max_minutes);
    let m = explore(opts, progress, stop).await;
    for s in &m.screens {
        println!("  {} {}{}", s.id, s.title, if s.sign_in { "  (sign-in)" } else { "" });
    }
    for k in &m.skipped {
        println!("  skipped \"{}\" on {}: {}", k.label, k.screen, k.why);
    }
    println!("{}", m.message);
    if m.state == "could_not_open" || m.state == "failed" {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_and_fingerprints_have_no_secrets_or_ids() {
        assert_eq!(place("https://u:p@shop.test/orders/12?token=abc#x"), "https://shop.test/orders/12");
        assert_eq!(place("https://shop.test/app#/cart?coupon=1"), "https://shop.test/app#/cart");
        assert_eq!(without_ids("/orders/1234/items/7f3a9c2e8b1d4a6f99"), "/orders/:id/items/:id");
        assert_eq!(inside_of("file:///x/demo/index.html"), "file:///x/demo/");
        assert_eq!(inside_of("https://shop.test/a/b"), "https://shop.test/");
        assert!(in_app("https://shop.test", "https://shop.test/") && !in_app("https://shop.test.evil.com/x", "https://shop.test/"));
        assert_eq!(join("https://shop.test/a/b.html?x=1", "c.html"), "https://shop.test/a/c.html");
        assert_eq!(join("https://shop.test/a/b.html", "/leave"), "https://shop.test/leave");
        assert_eq!(join("file:///x/demo/index.html", "help.html"), "file:///x/demo/help.html");
        assert_eq!(join("https://shop.test/a", "#/cart"), "https://shop.test/a#/cart");
        let inside = "https://shop.test/";
        assert!(link_refusal("https://shop.test/products", inside).is_none());
        assert!(link_refusal("file:///private/var/folders/_s/tym1vbd97l5dy15zbgz258h40000gn/T/demo/help.html", "file:///private/var/folders/_s/tym1vbd97l5dy15zbgz258h40000gn/T/demo/").is_none(), "a temp folder name is no token");
        assert!(link_refusal("file:///x/demo/delete.html", "file:///x/demo/").is_some());
        for bad in ["https://shop.test/leave", "https://shop.test/account/delete", "https://evil.test/", "javascript:alert(1)", "https://shop.test/a?token=abc", "mailto:a@b.c"] {
            assert!(link_refusal(bad, inside).is_some(), "{bad}");
        }
        assert_eq!(clean("Hello\u{200b} \u{1b}[2J world"), "Hello world");
        assert_eq!(clean("Order for bob@corp.example ghp_abcdefghijklmnopqrstu1234 see https://evil.example/x"), "Order for [email removed] *** see [link removed]");
        assert!(slug("https://shop.test/").starts_with("shop-test-") && !slug("../../etc/passwd").contains('/'));
    }

    #[test]
    fn it_never_presses_what_could_change_something() {
        for (label, form) in [("Delete", false), ("Bayar", false), ("Sign out", false), ("Keluar", false), ("Send message", false), ("Masuk", true), ("Search", true)] {
            assert!(skip_reason("button", label, form).is_some(), "{label}");
        }
        assert!(skip_reason("link", "Orders", true).is_none(), "a link on a form screen is followed");
        // Security review, High: only buttons that move around are pressed.
        for label in ["Next", "Menu", "More details", "Help"] {
            assert!(skip_reason("button", label, false).is_none(), "{label}");
        }
        for label in ["Products", "Add", "Continue", "Yes", "OK", "Apply", "Subscribe", "Follow", "Like", "Done"] {
            assert!(skip_reason("button", label, false).is_some(), "{label}");
        }
        assert!(skip_reason("button", "Izin kamera", false).is_some() && skip_reason("button", "Allow notifications", false).is_some(), "asking for a permission changes the device");
    }
}
