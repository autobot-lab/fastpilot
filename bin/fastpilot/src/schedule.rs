//! Jadwal (C3): run a test suite with `fastpilot run` at set times through the user's own OS
//! scheduler, and tell someone when it fails.
//!
//! - Schedules live in `~/.fastpilot/schedules.json` (0600). An OS entry is written only when the
//!   user saves an enabled schedule, and removed when it is turned off or deleted: a launchd
//!   LaunchAgent on macOS, a systemd user timer on Linux, else a crontab line. The entry runs
//!   `<this fastpilot> schedule run <id>`; the id is `[a-z0-9-]`, and the tests path, name and
//!   notification settings stay in the JSON file, so nothing the user typed reaches a plist, unit
//!   file or shell line.
//! - Notifications on failure: an https webhook (Slack-compatible `{"text": ...}`), a local OS
//!   notification, and/or a GitHub issue (token from `GITHUB_TOKEN`/`GH_TOKEN` or the Keychain /
//!   secret service). The webhook URL is a secret: stored in `~/.fastpilot/secrets/` (0600), never
//!   in the JSON file, and shown back only as its host.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

type Res<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Every {
    #[default]
    Day,
    Weekday,
    Week,
    /// Every `hours` hours (1–24), inside the optional `from`–`to` window, every day.
    Hours,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct When {
    pub every: Every,
    /// `HH:MM`, local time of this machine (not used by `hours`).
    #[serde(default)]
    pub time: String,
    /// For `week`: 1 = Monday ... 7 = Sunday.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<u8>,
    /// For `hours`: run every N hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<u8>,
    /// For `hours`: the active window, `HH:MM` to `HH:MM` (both or neither; none = all day from 00:00).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}


#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notify {
    /// Post to the webhook stored for this schedule.
    #[serde(default)]
    pub webhook: bool,
    /// A notification on this computer.
    #[serde(default)]
    pub os: bool,
    /// `owner/repo` to open an issue in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub name: String,
    /// A folder of test cases or one `.md` file (absolute).
    pub tests: String,
    pub when: When,
    #[serde(default)]
    pub notify: Notify,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// D10a: the agent it belongs to; `None` = the first agent (`default`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastRun {
    pub started_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<u64>,
    /// `None` while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    /// What happened to each notification (`webhook: terkirim`, `github: gagal ...`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notified: Vec<String>,
    /// Tests that passed, of how many (from the suite's summary line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// The `fastpilot schedule run` process, while it runs (Run now refuses a second one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub schedules: Vec<Schedule>,
    #[serde(default)]
    pub last: std::collections::BTreeMap<String, LastRun>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// `~/.fastpilot` (the folder of `config.toml`).
fn home() -> PathBuf {
    fastpilot_core::config::path().parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir)
}

/// Where scheduled runs keep their HTML reports.
pub fn reports_dir() -> PathBuf {
    home().join("schedules").join("reports")
}

fn store_path() -> PathBuf {
    home().join("schedules.json")
}

pub fn secret_path(id: &str) -> PathBuf {
    home().join("secrets").join(format!("webhook-{id}"))
}

/// Write `text` owner-only (0600), replacing the file atomically; its folder becomes 0700.
fn write_private(path: &Path, text: &str) -> Res<()> {
    use std::io::Write;
    let dir = path.parent().ok_or("Folder tidak dikenal")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Tidak bisa membuat folder {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = path.with_extension("tmp");
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(&tmp)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| format!("Tidak bisa menulis {}: {e}", path.display()))
}

pub fn load() -> Store {
    std::fs::read_to_string(store_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_store(store: &Store) -> Res<()> {
    write_private(&store_path(), &serde_json::to_string_pretty(store).map_err(|e| e.to_string())?)
}

fn set_last(id: &str, last: LastRun) -> Res<()> {
    let mut store = load();
    store.last.insert(id.to_owned(), last);
    save_store(&store)
}

// ---- validation ---------------------------------------------------------------------------

/// A readable id from the name: `Login harian` -> `login-harian` (unique among `taken`).
pub fn new_id(name: &str, taken: &[String]) -> String {
    let mut base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    base.truncate(40);
    let base = base.trim_end_matches('-').to_owned();
    let base = if base.is_empty() { "jadwal".to_owned() } else { base };
    (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|id| !taken.contains(id)).unwrap_or(base)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 48 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn hour_minute(time: &str) -> Option<(u8, u8)> {
    let (h, m) = time.split_once(':')?;
    let (h, m) = (h.parse::<u8>().ok()?, m.parse::<u8>().ok()?);
    (h < 24 && m < 60 && time.len() == 5).then_some((h, m))
}

/// `owner/repo` with GitHub's allowed characters.
fn valid_repo(repo: &str) -> bool {
    let ok = |p: &str| !p.is_empty() && p.len() <= 100 && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && p != "." && p != "..";
    repo.split_once('/').is_some_and(|(o, r)| ok(o) && ok(r))
}

/// https anywhere; plain http only to this computer (a local relay, or a test server).
pub fn check_webhook(url: &str) -> Res<()> {
    let lower = url.to_ascii_lowercase();
    let local = ["http://127.0.0.1:", "http://127.0.0.1/", "http://localhost:", "http://localhost/", "http://[::1]:"].iter().any(|p| lower.starts_with(p));
    if !(lower.starts_with("https://") || local) || url.len() > 2000 || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("The webhook URL must start with https:// (http only for 127.0.0.1 or localhost)".into());
    }
    let host = url.split("://").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or("");
    if host.is_empty() || host.contains('@') {
        return Err("Write the webhook URL without user:password@".into());
    }
    Ok(())
}

/// The webhook shown back to the user: only scheme and host.
pub fn mask_webhook(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{scheme}://{host}/•••")
}

pub fn has_webhook(id: &str) -> bool {
    secret_path(id).exists()
}

/// Whether `s` can be saved (also used for a chat proposal before it is shown).
pub fn validate(s: &Schedule) -> Res<()> {
    if !valid_id(&s.id) {
        return Err("A schedule id may only use lowercase letters, digits and -".into());
    }
    if s.name.trim().is_empty() || s.name.chars().count() > 80 {
        return Err("Give the schedule a name (1–80 characters)".into());
    }
    let tests = Path::new(&s.tests);
    if !tests.is_absolute() || !tests.exists() {
        return Err(format!("Tests not found: {}. Pick a test, or write the full path to a folder or a .md file", s.tests));
    }
    if tests.is_file() && tests.extension().is_none_or(|x| x != "md") {
        return Err("A test file must be a .md file (or pick its folder)".into());
    }
    check_when(&s.when)?;
    if let Some(repo) = &s.notify.github
        && !valid_repo(repo)
    {
        return Err("Write the GitHub repo as owner/name, e.g. autobot-lab/fastpilot".into());
    }
    Ok(())
}

/// Whether the time part of a schedule makes sense; the reason in the user's words if not.
pub fn check_when(w: &When) -> Res<()> {
    if w.every == Every::Hours {
        if w.day.is_some() {
            return Err("A day is only for weekly schedules".into());
        }
        if !matches!(w.hours, Some(1..=24)) {
            return Err("Every N hours: N must be a whole number from 1 to 24".into());
        }
        return match (w.from.as_deref(), w.to.as_deref()) {
            (None, None) => Ok(()),
            (Some(a), Some(b)) => match (hour_minute(a), hour_minute(b)) {
                (Some(x), Some(y)) if x < y => Ok(()),
                (Some(_), Some(_)) => Err(format!("The active window ends before it starts ({a}–{b}). Write it like 08:00–20:00")),
                _ => Err("Write the active window as HH:MM, e.g. 08:00–20:00".into()),
            },
            _ => Err("Give both the start and the end of the active window, or leave both empty".into()),
        };
    }
    if w.hours.is_some() || w.from.is_some() || w.to.is_some() {
        return Err("Hours and an active window are only for \"every N hours\" schedules".into());
    }
    if hour_minute(&w.time).is_none() {
        return Err("Write the time as HH:MM, e.g. 08:00".into());
    }
    match (w.every, w.day) {
        (Every::Week, Some(1..=7)) | (Every::Day | Every::Weekday, None) => Ok(()),
        (Every::Week, _) => Err("Pick a day for a weekly schedule".into()),
        _ => Err("A day is only for weekly schedules".into()),
    }
}

/// The times of day a schedule runs (local time), in order.
pub fn slots(w: &When) -> Vec<(u8, u8)> {
    if w.every != Every::Hours {
        return hour_minute(&w.time).into_iter().collect();
    }
    let step = u16::from(w.hours.unwrap_or(24).clamp(1, 24)) * 60;
    let mins = |t: Option<&str>, d: (u8, u8)| hour_minute(t.unwrap_or("")).unwrap_or(d);
    let (a, b) = (mins(w.from.as_deref(), (0, 0)), mins(w.to.as_deref(), (23, 59)));
    let (start, end) = (u16::from(a.0) * 60 + u16::from(a.1), u16::from(b.0) * 60 + u16::from(b.1));
    (start..=end).step_by(usize::from(step)).map(|m| ((m / 60) as u8, (m % 60) as u8)).collect()
}

/// The days a schedule runs: `None` = every day, else 1 = Monday ... 7 = Sunday.
pub fn run_days(w: &When) -> Option<Vec<u8>> {
    match w.every {
        Every::Day | Every::Hours => None,
        Every::Weekday => Some((1..=5).collect()),
        Every::Week => Some(vec![w.day.unwrap_or(1).clamp(1, 7)]),
    }
}

/// `daily 09:00`, `weekdays 08:00`, `Monday 07:00`, `every 2 hours, 08:00–20:00`.
pub fn describe(w: &When) -> String {
    const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    match w.every {
        Every::Day => format!("daily {}", w.time),
        Every::Weekday => format!("weekdays {}", w.time),
        Every::Week => format!("{} {}", DAYS.get(usize::from(w.day.unwrap_or(1).clamp(1, 7)) - 1).unwrap_or(&"Monday"), w.time),
        Every::Hours => {
            let every = match w.hours.unwrap_or(1) {
                1 => "every hour".to_owned(),
                n => format!("every {n} hours"),
            };
            match (&w.from, &w.to) {
                (Some(a), Some(b)) => format!("{every}, {a}–{b}"),
                _ => every,
            }
        }
    }
}

// ---- OS scheduler -------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Launchd,
    Systemd,
    Cron,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Self::Launchd => "launchd",
            Self::Systemd => "systemd",
            Self::Cron => "cron",
        }
    }
}

/// `FASTPILOT_SCHEDULER` (launchd, systemd, cron), else the platform's own.
pub fn backend() -> Backend {
    match std::env::var("FASTPILOT_SCHEDULER").ok().as_deref() {
        Some("launchd") => Backend::Launchd,
        Some("systemd") => Backend::Systemd,
        Some("cron") => Backend::Cron,
        _ if cfg!(target_os = "macos") => Backend::Launchd,
        _ => {
            let systemd = std::process::Command::new("systemctl").args(["--user", "show-environment"]).output().is_ok_and(|o| o.status.success());
            if systemd { Backend::Systemd } else { Backend::Cron }
        }
    }
}

/// Tests: write the entries under `FASTPILOT_SCHEDULER_DIR` and do not load them.
fn dry_dir() -> Option<PathBuf> {
    std::env::var_os("FASTPILOT_SCHEDULER_DIR").filter(|d| !d.is_empty()).map(PathBuf::from)
}

fn label(id: &str) -> String {
    format!("dev.fastpilot.schedule.{id}")
}

fn exe() -> Res<String> {
    let p = std::env::current_exe().map_err(|e| format!("File program fastpilot tidak ditemukan: {e}"))?;
    Ok(p.to_string_lossy().into_owned())
}

/// PATH for the scheduled run: the one of whoever saved it (so `claude` is found), plus the usual.
fn run_path() -> String {
    let mine = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<&str> = mine.split(':').filter(|p| p.starts_with('/')).collect();
    for p in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        if !parts.contains(&p) {
            parts.push(p);
        }
    }
    parts.join(":")
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The launchd LaunchAgent for `s`.
pub fn plist(s: &Schedule, exe: &str, path: &str, log: &Path) -> String {
    let entry = |(h, m): (u8, u8), day: Option<u8>| {
        // launchd: 0 and 7 are Sunday, 1 Monday.
        let day = day.map(|d| format!("<key>Weekday</key><integer>{}</integer>", d % 7)).unwrap_or_default();
        format!("<dict><key>Hour</key><integer>{h}</integer><key>Minute</key><integer>{m}</integer>{day}</dict>")
    };
    let days: Vec<Option<u8>> = run_days(&s.when).map(|d| d.into_iter().map(Some).collect()).unwrap_or_else(|| vec![None]);
    let times: String = days.iter().flat_map(|d| slots(&s.when).into_iter().map(move |t| entry(t, *d))).collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{}</string>
  <key>ProgramArguments</key><array><string>{}</string><string>schedule</string><string>run</string><string>{}</string></array>
  <key>StartCalendarInterval</key><array>{times}</array>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{}</string></dict>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
"#,
        xml(&label(&s.id)),
        xml(exe),
        xml(&s.id),
        xml(path),
        xml(&log.to_string_lossy()),
        xml(&log.to_string_lossy())
    )
}

/// systemd: a word of `ExecStart=` / `Environment=`, quoted, with `%` and `$` escaped.
fn systemd_word(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$"))
}

pub fn systemd_units(s: &Schedule, exe: &str, path: &str) -> (String, String) {
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let days = match s.when.every {
        Every::Day | Every::Hours => String::new(),
        Every::Weekday => "Mon..Fri ".into(),
        Every::Week => format!("{} ", DAYS[usize::from(s.when.day.unwrap_or(1).clamp(1, 7)) - 1]),
    };
    let calendar: String = slots(&s.when).iter().map(|(h, m)| format!("OnCalendar={days}*-*-* {h:02}:{m:02}:00\n")).collect();
    let service = format!(
        "[Unit]\nDescription=fastpilot schedule {id}\n\n[Service]\nType=oneshot\nEnvironment={}\nExecStart={} schedule run {id}\n",
        systemd_word(&format!("PATH={path}")),
        systemd_word(exe),
        id = s.id
    );
    let timer = format!(
        "[Unit]\nDescription=fastpilot schedule {id}\n\n[Timer]\n{calendar}Persistent=true\n\n[Install]\nWantedBy=timers.target\n",
        id = s.id
    );
    (service, timer)
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// One crontab line per minute-of-the-hour (every N hours from 08:30 is `30 8,10,12 ...`).
pub fn cron_lines(s: &Schedule, exe: &str, path: &str) -> Vec<String> {
    let dow = match s.when.every {
        Every::Day | Every::Hours => "*".to_owned(),
        Every::Weekday => "1-5".to_owned(),
        Every::Week => (s.when.day.unwrap_or(1) % 7).to_string(),
    };
    let mut by_minute: std::collections::BTreeMap<u8, Vec<String>> = Default::default();
    for (h, m) in slots(&s.when) {
        by_minute.entry(m).or_default().push(h.to_string());
    }
    by_minute
        .iter()
        .map(|(m, hours)| format!("{m} {} * * {dow} PATH={} {} schedule run {} # fastpilot-schedule:{}", hours.join(","), shell_quote(path), shell_quote(exe), s.id, s.id))
        .collect()
}

fn run_cmd(cmd: &str, args: &[&str], stdin: Option<&str>) -> Res<String> {
    use std::io::Write;
    let mut c = std::process::Command::new(cmd);
    c.args(args).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    if stdin.is_some() {
        c.stdin(std::process::Stdio::piped());
    }
    let mut child = c.spawn().map_err(|e| format!("Perintah `{cmd}` tidak bisa dijalankan: {e}"))?;
    if let (Some(text), Some(mut i)) = (stdin, child.stdin.take()) {
        i.write_all(text.as_bytes()).map_err(|e| format!("Perintah `{cmd}` gagal: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("Perintah `{cmd}` gagal: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() { Ok(text) } else { Err(format!("Perintah `{cmd} {}` gagal: {}", args.join(" "), text.trim())) }
}

fn uid() -> String {
    run_cmd("id", &["-u"], None).map(|s| s.trim().to_owned()).unwrap_or_default()
}

fn launch_agents() -> PathBuf {
    dry_dir().unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Library/LaunchAgents"))
}

fn systemd_dir() -> PathBuf {
    dry_dir().unwrap_or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config"))
            .join("systemd/user")
    })
}

/// Lines of the user's crontab other than this schedule's.
fn crontab_without(id: &str) -> Res<Vec<String>> {
    let current = if let Some(d) = dry_dir() {
        std::fs::read_to_string(d.join("crontab")).unwrap_or_default()
    } else {
        // No crontab yet is not an error.
        run_cmd("crontab", &["-l"], None).unwrap_or_default()
    };
    let marker = format!("# fastpilot-schedule:{id}");
    Ok(current.lines().filter(|l| !l.ends_with(&marker)).map(str::to_owned).collect())
}

fn write_crontab(lines: &[String]) -> Res<()> {
    let text = if lines.is_empty() { String::new() } else { format!("{}\n", lines.join("\n")) };
    match dry_dir() {
        Some(d) => write_private(&d.join("crontab"), &text),
        None => run_cmd("crontab", &["-"], Some(&text)).map(|_| ()),
    }
}

/// Create (or replace) the OS entry for `s`.
pub fn install(s: &Schedule) -> Res<()> {
    let (exe, path) = (exe()?, run_path());
    match backend() {
        Backend::Launchd => {
            let file = launch_agents().join(format!("{}.plist", label(&s.id)));
            let log = home().join("schedules").join(format!("{}.log", s.id));
            std::fs::create_dir_all(log.parent().unwrap_or(&home())).map_err(|e| format!("Tidak bisa menyiapkan file jadwal: {e}"))?;
            if dry_dir().is_none() {
                let _ = run_cmd("launchctl", &["bootout", &format!("gui/{}/{}", uid(), label(&s.id))], None);
            }
            std::fs::create_dir_all(launch_agents()).map_err(|e| format!("Tidak bisa menyiapkan file jadwal: {e}"))?;
            std::fs::write(&file, plist(s, &exe, &path, &log)).map_err(|e| format!("Tidak bisa menulis {}: {e}", file.display()))?;
            if dry_dir().is_none() {
                run_cmd("launchctl", &["bootstrap", &format!("gui/{}", uid()), &file.to_string_lossy()], None)?;
            }
        }
        Backend::Systemd => {
            let dir = systemd_dir();
            std::fs::create_dir_all(&dir).map_err(|e| format!("Tidak bisa menyiapkan file jadwal: {e}"))?;
            let (service, timer) = systemd_units(s, &exe, &path);
            let unit = format!("fastpilot-schedule-{}", s.id);
            std::fs::write(dir.join(format!("{unit}.service")), service).map_err(|e| format!("Tidak bisa menyiapkan file jadwal: {e}"))?;
            std::fs::write(dir.join(format!("{unit}.timer")), timer).map_err(|e| format!("Tidak bisa menyiapkan file jadwal: {e}"))?;
            if dry_dir().is_none() {
                run_cmd("systemctl", &["--user", "daemon-reload"], None)?;
                run_cmd("systemctl", &["--user", "enable", "--now", &format!("{unit}.timer")], None)?;
            }
        }
        Backend::Cron => {
            let mut lines = crontab_without(&s.id)?;
            lines.extend(cron_lines(s, &exe, &path));
            write_crontab(&lines)?;
        }
    }
    Ok(())
}

/// Remove the OS entry of `id` (nothing to remove is fine).
pub fn uninstall(id: &str) -> Res<()> {
    match backend() {
        Backend::Launchd => {
            if dry_dir().is_none() {
                let _ = run_cmd("launchctl", &["bootout", &format!("gui/{}/{}", uid(), label(id))], None);
            }
            let _ = std::fs::remove_file(launch_agents().join(format!("{}.plist", label(id))));
        }
        Backend::Systemd => {
            let unit = format!("fastpilot-schedule-{id}");
            if dry_dir().is_none() {
                let _ = run_cmd("systemctl", &["--user", "disable", "--now", &format!("{unit}.timer")], None);
            }
            let dir = systemd_dir();
            let _ = std::fs::remove_file(dir.join(format!("{unit}.service")));
            let _ = std::fs::remove_file(dir.join(format!("{unit}.timer")));
            if dry_dir().is_none() {
                let _ = run_cmd("systemctl", &["--user", "daemon-reload"], None);
            }
        }
        Backend::Cron => write_crontab(&crontab_without(id)?)?,
    }
    Ok(())
}

// ---- changes (from the page) --------------------------------------------------------------

/// Save `s` (new or changed): the JSON file, the webhook secret when given (`Some("")` removes
/// it), and the OS entry. Returns the saved schedule (with its id).
pub fn save(mut s: Schedule, webhook: Option<&str>) -> Res<Schedule> {
    let mut store = load();
    if s.id.is_empty() {
        let taken: Vec<String> = store.schedules.iter().map(|x| x.id.clone()).collect();
        s.id = new_id(&s.name, &taken);
    }
    s.name = s.name.trim().to_owned();
    s.tests = s.tests.trim().to_owned();
    validate(&s)?;
    match webhook.map(str::trim) {
        Some("") => {
            let _ = std::fs::remove_file(secret_path(&s.id));
        }
        Some(url) => {
            check_webhook(url)?;
            write_private(&secret_path(&s.id), url)?;
        }
        None => {}
    }
    if s.notify.webhook && !has_webhook(&s.id) {
        return Err("Enter the webhook URL first, or turn off the webhook notification".into());
    }
    if s.enabled {
        install(&s)?;
    } else {
        uninstall(&s.id)?;
    }
    store.schedules.retain(|x| x.id != s.id);
    store.schedules.push(s.clone());
    save_store(&store)?;
    Ok(s)
}

/// D10a: an agent's schedules go to another (when it is deleted). No OS entry changes.
pub fn move_agent(from: &str, to: &str) -> Res<()> {
    let mut store = load();
    let mut moved = false;
    for s in store.schedules.iter_mut().filter(|s| s.agent.as_deref() == Some(from)) {
        s.agent = (to != crate::agents::DEFAULT).then(|| to.to_owned());
        moved = true;
    }
    if moved { save_store(&store) } else { Ok(()) }
}

pub fn set_enabled(id: &str, on: bool) -> Res<()> {
    let mut store = load();
    let s = store.schedules.iter_mut().find(|x| x.id == id).ok_or("Schedule not found")?;
    s.enabled = on;
    if on { install(s)? } else { uninstall(id)? }
    save_store(&store)
}

pub fn delete(id: &str) -> Res<()> {
    let mut store = load();
    if !store.schedules.iter().any(|x| x.id == id) {
        return Err("Schedule not found".into());
    }
    uninstall(id)?;
    let _ = std::fs::remove_file(secret_path(id));
    store.schedules.retain(|x| x.id != id);
    store.last.remove(id);
    save_store(&store)
}

/// Whether a run of this schedule is going on now: started, not finished, and its process alive
/// (a run that started under a minute ago may not have written its process id yet).
pub fn is_running(last: Option<&LastRun>) -> bool {
    let Some(l) = last else { return false };
    if l.passed.is_some() || l.finished_ms.is_some() {
        return false;
    }
    match l.pid {
        Some(pid) => alive(pid),
        None => now_ms().saturating_sub(l.started_ms) < 60_000,
    }
}

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Start `fastpilot schedule run <id>` in the background (Run now). Refused while it runs.
pub fn start_now(id: &str) -> Res<()> {
    let store = load();
    let s = store.schedules.iter().find(|x| x.id == id).ok_or("Schedule not found")?;
    if is_running(store.last.get(id)) {
        return Err(format!("\"{}\" is already running. Wait until it finishes, then try again.", s.name));
    }
    // Marked as started now, so a second click right away is refused too.
    let before = store.last.get(id).cloned();
    set_last(id, LastRun { started_ms: now_ms(), ..Default::default() })?;
    let undo = |e: String| {
        let mut store = load();
        match &before {
            Some(l) => store.last.insert(id.to_owned(), l.clone()),
            None => store.last.remove(id),
        };
        let _ = save_store(&store);
        e
    };
    let log = home().join("schedules").join(format!("{id}.log"));
    std::fs::create_dir_all(log.parent().unwrap_or(&home())).map_err(|e| undo(format!("Could not prepare the schedule files: {e}")))?;
    let out = std::fs::OpenOptions::new().create(true).append(true).open(&log).map_err(|e| undo(format!("Could not prepare the schedule files: {e}")))?;
    let err = out.try_clone().map_err(|e| undo(format!("Could not prepare the schedule files: {e}")))?;
    std::process::Command::new(exe().map_err(undo)?)
        .args(["schedule", "run", id])
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|e| undo(format!("The run could not start: {e}")))?;
    Ok(())
}

// ---- running ------------------------------------------------------------------------------

/// `fastpilot schedule run <id>`: the suite with `fastpilot run`, its result kept for the page,
/// and the notifications when it fails.
pub async fn run(id: &str) -> Res<bool> {
    let store = load();
    let s = store.schedules.iter().find(|x| x.id == id).cloned().ok_or_else(|| format!("Jadwal `{id}` tidak ditemukan"))?;
    // Another run of it is still going (an OS start during a long Run now): this one is skipped.
    if let Some(l) = store.last.get(id)
        && l.pid.is_some_and(|p| p != std::process::id())
        && is_running(Some(l))
    {
        println!("\"{}\" is already running; this start is skipped.", s.name);
        return Ok(true);
    }
    let started = now_ms();
    let reports = reports_dir();
    std::fs::create_dir_all(&reports).map_err(|e| format!("Tidak bisa menyiapkan file jadwal: {e}"))?;
    let report = reports.join(format!("{id}-{started}.html"));
    set_last(id, LastRun { started_ms: started, pid: Some(std::process::id()), ..Default::default() })?;
    // Test cases name pages and files relative to the project (the folder above `tests/`), and
    // the OS scheduler starts us in `/`.
    let tests = Path::new(&s.tests);
    let folder = if tests.is_file() { tests.parent() } else { Some(tests) };
    let project = folder.and_then(Path::parent).unwrap_or(Path::new("/"));
    let mut cmd = tokio::process::Command::new(exe()?);
    // D10b: an agent's schedule runs with that agent's provider and model, and its extra words.
    for (k, v) in agent_env(s.agent.as_deref()) {
        cmd.env(k, v);
    }
    let out = cmd
        .args(["run", &s.tests, "--html", &report.to_string_lossy()])
        // The schedule's name is the run's title in History (D7).
        .env("FASTPILOT_RUN_TITLE", &s.name)
        .current_dir(project)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("fastpilot run tidak bisa dimulai: {e}"))?;
    let text = fastpilot_core::redact(&format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
    print!("{text}");
    let passed = out.status.success();
    let summary = summarize(&text, passed);
    let report = report.exists().then(|| report.to_string_lossy().into_owned());
    let (ok, total) = counts(&text).unzip();
    let mut last = LastRun { started_ms: started, finished_ms: Some(now_ms()), passed: Some(passed), summary: summary.clone(), report: report.clone(), notified: vec![], ok, total, pid: None };
    if !passed {
        let msg = format!(
            "fastpilot: schedule \"{}\" failed — {summary}{}",
            s.name,
            report.as_deref().map(|r| format!("\nReport: {r}")).unwrap_or_default()
        );
        last.notified = notify(&s, &format!("Schedule \"{}\" failed", s.name), &msg).await;
    }
    set_last(id, last)?;
    Ok(passed)
}

/// `(passed, total)` from the suite's summary line `3 tests: 2 passed, ...`.
pub fn counts(output: &str) -> Option<(u64, u64)> {
    let l = output.lines().rev().find(|l| l.contains(" tests: ") && l.contains(" passed"))?;
    let num = |after: &str| l.split(after).next().and_then(|x| x.rsplit([' ', ':', ',']).find(|w| !w.is_empty())).and_then(|w| w.parse::<u64>().ok());
    Some((num(" passed")?, num(" tests:")?))
}

/// The environment of a scheduled run for `agent`: its provider and model (D10b), its extra
/// blocked words (stricter only), and its id for Live.
pub fn agent_env(agent: Option<&str>) -> Vec<(String, String)> {
    let Some(a) = agent.and_then(crate::agents::get) else { return vec![] };
    let mut env = vec![("FASTPILOT_AGENT".to_owned(), a.id.clone())];
    match (a.provider.as_deref(), a.model.as_deref()) {
        (Some(p), Some(m)) if p != "claude" => {
            env.push(("FASTPILOT_PROVIDER".into(), p.into()));
            env.push(("FASTPILOT_PROVIDER_MODEL".into(), m.into()));
            if let Some(base) = fastpilot_core::config::preset_base(p) {
                env.push(("FASTPILOT_PROVIDER_BASE_URL".into(), base.into()));
            }
        }
        (_, Some(m)) => {
            env.push(("FASTPILOT_PROVIDER".into(), "claude".into()));
            env.push(("FASTPILOT_MODEL".into(), m.into()));
        }
        _ => {}
    }
    if let Some(b) = crate::agents::block_actions(&a) {
        env.push(("FASTPILOT_BLOCK_ACTIONS".into(), b));
    }
    env
}

/// The suite's own summary line (`3 tests: 2 passed, 1 failed, ...`) and why each failed test
/// failed (QA B7: the schedules table showed only "0 of 1 passed"; the reason was in the report).
pub fn summarize(output: &str, passed: bool) -> String {
    // `3 tests: 2 passed, 1 failed, 0 quarantined failures, in …` -> `2 of 3 tests passed, 1 failed`.
    let line = output.lines().rev().find(|l| l.contains(" tests: ") && l.contains(" passed")).map(|l| {
        let num = |after: &str| l.split(after).next().and_then(|x| x.rsplit([' ', ':', ',']).find(|w| !w.is_empty())).and_then(|w| w.parse::<u64>().ok()).unwrap_or(0);
        let (total, ok, bad, quarantined) = (num(" tests:"), num(" passed"), num(" failed"), num(" quarantined failures"));
        let mut t = format!("{ok} of {total} {} passed", if total == 1 { "test" } else { "tests" });
        if bad > 0 {
            t.push_str(&format!(", {bad} failed"));
        }
        if quarantined > 0 {
            t.push_str(&format!(", {quarantined} set aside failed"));
        }
        t
    });
    // `✗ checkout 9.0s, 7 turns, haiku [product]: no Pay button` -> `checkout: no Pay button`.
    let failed: Vec<String> = output
        .lines()
        .filter_map(|l| l.strip_prefix("✗ "))
        .map(|l| {
            let id = l.split_whitespace().next().unwrap_or("");
            match l.split_once(": ").map(|(_, why)| why.trim()).filter(|w| !w.is_empty()) {
                Some(why) => format!("{id}: {}", clip(why, 160)),
                None => id.to_owned(),
            }
        })
        .collect();
    let mut text = line.unwrap_or_else(|| {
        let err = output.lines().rev().find(|l| l.contains("error") || l.contains("Error")).unwrap_or("").trim();
        if passed { "passed".to_owned() } else if err.is_empty() { "did not run".to_owned() } else { format!("did not run: {}", clip(err, 300)) }
    });
    if !failed.is_empty() {
        let more = failed.len().saturating_sub(3);
        text.push_str(&format!(" — {}{}", failed.iter().take(3).cloned().collect::<Vec<_>>().join("; "), if more > 0 { format!("; and {more} more") } else { String::new() }));
    }
    text
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_owned() }
}

// ---- notifications ------------------------------------------------------------------------

/// Send `msg` on every channel the schedule has; one line per channel saying how it went.
pub async fn notify(s: &Schedule, title: &str, msg: &str) -> Vec<String> {
    let mut done = Vec::new();
    if s.notify.webhook {
        done.push(match std::fs::read_to_string(secret_path(&s.id)) {
            Ok(url) => match webhook(url.trim(), msg).await {
                Ok(()) => "Webhook: sent".to_owned(),
                Err(e) => format!("Webhook: failed ({e})"),
            },
            Err(_) => "Webhook: no URL yet".to_owned(),
        });
    }
    if s.notify.os {
        done.push(match os_notification(title, msg) {
            Ok(()) => "Notification: sent".to_owned(),
            Err(e) => format!("Notification: failed ({e})"),
        });
    }
    if let Some(repo) = &s.notify.github {
        done.push(match github_issue(repo, title, msg).await {
            Ok(url) => format!("GitHub issue: {url}"),
            Err(e) => format!("GitHub issue: failed ({e})"),
        });
    }
    done
}

fn http() -> Res<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        // A redirect could point the request (and the issue token) somewhere else.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("fastpilot/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// Slack-compatible incoming webhook: `{"text": msg}`. Errors never include the URL.
pub async fn webhook(url: &str, msg: &str) -> Res<()> {
    check_webhook(url)?;
    let r = http()?.post(url).json(&json!({ "text": msg })).send().await.map_err(|e| if e.is_timeout() { "tidak menjawab dalam 15 detik".to_owned() } else { "tidak bisa dihubungi".to_owned() })?;
    if r.status().is_success() { Ok(()) } else { Err(format!("server menjawab {}", r.status().as_u16())) }
}

/// A notification on this computer; the text goes as an argument, never into a script.
pub fn os_notification(title: &str, msg: &str) -> Res<()> {
    if std::env::var_os("FASTPILOT_NOTIFY_DRY").is_some() {
        return Ok(());
    }
    let body: String = msg.chars().take(240).collect();
    if cfg!(target_os = "macos") {
        run_cmd(
            "osascript",
            &["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run", title, &body],
            None,
        )
        .map(|_| ())
    } else {
        run_cmd("notify-send", &["--app-name=fastpilot", title, &body], None).map(|_| ())
    }
}

/// `GITHUB_TOKEN` / `GH_TOKEN`, else the Keychain (macOS, service `fastpilot-github`) or the
/// secret service (Linux, `secret-tool lookup service fastpilot-github`).
fn github_token() -> Option<String> {
    for k in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(t) = std::env::var(k)
            && !t.trim().is_empty()
        {
            return Some(t.trim().to_owned());
        }
    }
    let found = if cfg!(target_os = "macos") {
        run_cmd("security", &["find-generic-password", "-s", "fastpilot-github", "-w"], None)
    } else {
        run_cmd("secret-tool", &["lookup", "service", "fastpilot-github"], None)
    };
    found.ok().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

/// Where the GitHub API is (`FASTPILOT_GITHUB_API` for a test server).
fn github_api() -> String {
    std::env::var("FASTPILOT_GITHUB_API").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| "https://api.github.com".into())
}

pub async fn github_issue(repo: &str, title: &str, body: &str) -> Res<String> {
    if !valid_repo(repo) {
        return Err("Repo GitHub tidak valid".into());
    }
    let token = github_token().ok_or("Token GitHub tidak ada (GITHUB_TOKEN / GH_TOKEN, atau Keychain `fastpilot-github`)")?;
    let r = http()?
        .post(format!("{}/repos/{repo}/issues", github_api()))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .json(&json!({ "title": title, "body": format!("{body}\n\n_Dibuat oleh jadwal fastpilot._") }))
        .send()
        .await
        .map_err(|_| "GitHub tidak bisa dihubungi".to_owned())?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        Ok(v["html_url"].as_str().unwrap_or("dibuat").to_owned())
    } else {
        Err(format!("GitHub menjawab {}: {}", status.as_u16(), v["message"].as_str().unwrap_or("")))
    }
}

// ---- CLI ----------------------------------------------------------------------------------

pub const USAGE: &str = "pemakaian: fastpilot schedule <list | run ID | remove ID>\n  Jadwal dibuat di `fastpilot ui` (menu Jadwal); penjadwal OS menjalankan `fastpilot schedule run ID`";

pub async fn cli(args: &[String]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("list") | None, _) => {
            let store = load();
            if store.schedules.is_empty() {
                println!("belum ada jadwal (buat di `fastpilot ui`, menu Jadwal)");
            }
            for s in &store.schedules {
                let last = store.last.get(&s.id).map(|l| l.summary.clone()).unwrap_or_else(|| "belum jalan".into());
                println!("{} {:<20} {:<18} {} [{}]", if s.enabled { "●" } else { "○" }, s.id, describe(&s.when), s.tests, last);
            }
            Ok(())
        }
        (Some("run"), Some(id)) => {
            if !run(id).await? {
                std::process::exit(1);
            }
            Ok(())
        }
        (Some("remove"), Some(id)) => Ok(delete(id)?),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sched(every: Every, day: Option<u8>) -> Schedule {
        Schedule {
            id: "login-harian".into(),
            name: "Login <harian> & \"x\"".into(),
            tests: "/tmp".into(),
            when: When { every, time: "08:05".into(), day, ..Default::default() },
            notify: Notify::default(),
            enabled: true,
            agent: None,
        }
    }

    #[test]
    fn ids_and_times() {
        assert_eq!(new_id("Login harian!", &[]), "login-harian");
        assert_eq!(new_id("Login harian", &["login-harian".into()]), "login-harian-2");
        assert_eq!(new_id("$(rm -rf ~)", &[]), "rm-rf");
        assert_eq!(new_id("日本", &[]), "jadwal");
        assert_eq!(hour_minute("08:05"), Some((8, 5)));
        assert!(hour_minute("24:00").is_none() && hour_minute("8:5").is_none());
        assert!(valid_repo("autobot-lab/fastpilot") && !valid_repo("a/../b") && !valid_repo("a") && !valid_repo("../x"));
        assert_eq!(describe(&When { every: Every::Week, time: "07:00".into(), day: Some(1), ..Default::default() }), "Monday 07:00");
    }

    #[test]
    fn webhook_rules_and_mask() {
        assert!(check_webhook("https://hooks.slack.com/services/T/B/x").is_ok());
        assert!(check_webhook("http://127.0.0.1:9/hook").is_ok());
        assert!(check_webhook("http://hooks.slack.com/x").is_err(), "plain http to another host");
        assert!(check_webhook("https://user:pw@example.com/x").is_err());
        assert!(check_webhook("file:///etc/passwd").is_err());
        assert_eq!(mask_webhook("https://hooks.slack.com/services/T/B/secret"), "https://hooks.slack.com/•••");
    }

    #[test]
    fn os_entries_carry_only_the_id() {
        let s = sched(Every::Weekday, None);
        let p = plist(&s, "/opt/fast pilot/bin/fastpilot", "/usr/bin:/bin", Path::new("/tmp/x.log"));
        assert!(p.contains("<string>dev.fastpilot.schedule.login-harian</string>"));
        assert!(p.contains("<string>/opt/fast pilot/bin/fastpilot</string><string>schedule</string><string>run</string><string>login-harian</string>"));
        assert_eq!(p.matches("<key>Weekday</key>").count(), 5);
        assert!(p.contains("<key>Hour</key><integer>8</integer><key>Minute</key><integer>5</integer>"));
        assert!(!p.contains("harian>") && !p.contains("/tmp</string>"), "name and tests path stay out: {p}");
        let sunday = plist(&sched(Every::Week, Some(7)), "/x", "/bin", Path::new("/tmp/x.log"));
        assert!(sunday.contains("<key>Weekday</key><integer>0</integer>"));
        let (service, timer) = systemd_units(&s, "/opt/fast pilot/$HOME%/fastpilot", "/usr/bin");
        assert!(service.contains("ExecStart=\"/opt/fast pilot/$$HOME%%/fastpilot\" schedule run login-harian"), "{service}");
        assert!(timer.contains("OnCalendar=Mon..Fri *-*-* 08:05:00"), "{timer}");
        let line = cron_lines(&s, "/opt/it's/fastpilot", "/usr/bin").join("\n");
        assert_eq!(line, "5 8 * * 1-5 PATH='/usr/bin' '/opt/it'\\''s/fastpilot' schedule run login-harian # fastpilot-schedule:login-harian");
    }

    fn hourly(n: u8, from: Option<&str>, to: Option<&str>) -> When {
        When { every: Every::Hours, hours: Some(n), from: from.map(Into::into), to: to.map(Into::into), ..Default::default() }
    }

    /// D5: every N hours inside an active window becomes a fixed list of times for the OS scheduler.
    #[test]
    fn every_n_hours_becomes_fixed_times() {
        let w = hourly(2, Some("08:00"), Some("20:00"));
        assert_eq!(check_when(&w), Ok(()));
        assert_eq!(slots(&w), vec![(8, 0), (10, 0), (12, 0), (14, 0), (16, 0), (18, 0), (20, 0)]);
        assert_eq!(describe(&w), "every 2 hours, 08:00–20:00");
        assert_eq!(slots(&hourly(4, None, None)), vec![(0, 0), (4, 0), (8, 0), (12, 0), (16, 0), (20, 0)]);
        assert_eq!(describe(&hourly(1, None, None)), "every hour");
        assert_eq!(slots(&hourly(1, None, None)).len(), 24);
        assert_eq!(slots(&hourly(24, None, None)), vec![(0, 0)]);
        assert_eq!(slots(&hourly(3, Some("08:30"), Some("17:00"))), vec![(8, 30), (11, 30), (14, 30)]);
        // Refused, with a reason for people.
        for (bad, why) in [
            (hourly(0, None, None), "1 to 24"),
            (hourly(25, None, None), "1 to 24"),
            (When { every: Every::Hours, ..Default::default() }, "1 to 24"),
            (hourly(2, Some("20:00"), Some("08:00")), "ends before it starts"),
            (hourly(2, Some("08:00"), Some("08:00")), "ends before it starts"),
            (hourly(2, Some("08:00"), None), "both the start and the end"),
            (hourly(2, Some("8am"), Some("20:00")), "HH:MM"),
            (When { every: Every::Day, time: "08:00".into(), hours: Some(2), ..Default::default() }, "only for"),
        ] {
            let e = check_when(&bad).expect_err("refused");
            assert!(e.contains(why), "{bad:?}: {e}");
        }

        let s = Schedule { when: w, ..sched(Every::Day, None) };
        let p = plist(&s, "/x/fastpilot", "/bin", Path::new("/tmp/x.log"));
        assert_eq!(p.matches("<key>Hour</key>").count(), 7, "{p}");
        assert!(p.contains("<key>Hour</key><integer>20</integer><key>Minute</key><integer>0</integer>") && !p.contains("<key>Weekday</key>"), "{p}");
        let (_, timer) = systemd_units(&s, "/x/fastpilot", "/bin");
        assert_eq!(timer.matches("OnCalendar=").count(), 7, "{timer}");
        assert!(timer.contains("OnCalendar=*-*-* 08:00:00\n") && timer.contains("OnCalendar=*-*-* 20:00:00\n"), "{timer}");
        assert_eq!(cron_lines(&s, "/x/fastpilot", "/bin"), vec!["0 8,10,12,14,16,18,20 * * * PATH='/bin' '/x/fastpilot' schedule run login-harian # fastpilot-schedule:login-harian"]);
        // Fixed-time schedules keep their single entry.
        assert_eq!(cron_lines(&sched(Every::Weekday, None), "/x", "/bin").len(), 1);
    }

    /// Schedules saved by v0.3.x (no `hours`, `from`, `to`) still read and run the same way.
    #[test]
    fn schedules_from_before_still_read() {
        let old = r#"{"schedules":[{"id":"a","name":"A","tests":"/tmp","when":{"every":"weekday","time":"08:30"},"notify":{"os":true},"enabled":true},{"id":"b","name":"B","tests":"/tmp","when":{"every":"week","time":"07:00","day":7}}],"last":{"a":{"started_ms":1,"finished_ms":2,"passed":true,"summary":"1 dari 1 test lolos"}}}"#;
        let store: Store = serde_json::from_str(old).expect("old file");
        assert_eq!(store.schedules[0].when, When { every: Every::Weekday, time: "08:30".into(), ..Default::default() });
        assert_eq!(slots(&store.schedules[1].when), vec![(7, 0)]);
        assert!(validate(&store.schedules[0]).is_ok());
        assert!(!is_running(store.last.get("a")));
        let back = serde_json::to_string(&store.schedules[0]).expect("json");
        assert!(!back.contains("hours") && !back.contains("from"), "{back}");
        assert_eq!(counts("\n3 tests: 2 passed, 1 failed, 0 quarantined failures, in 2.0s\n"), Some((2, 3)));
    }

    #[test]
    fn summary_of_a_run() {
        let out = "3 tests, 1 workers, web\n✓ login 3.1s, 4 turns, haiku: ok\n✗ checkout 9.0s, 7 turns, haiku: no Bayar button\n\n3 tests: 2 passed, 1 failed, 0 quarantined failures, in 20.0s; tokens in 1 out 2\nhtml: /x.html\n";
        assert_eq!(summarize(out, false), "2 of 3 tests passed, 1 failed — checkout: no Bayar button");
        assert_eq!(summarize("\n1 tests: 1 passed, 0 failed, 0 quarantined failures, in 2.0s\n", true), "1 of 1 test passed");
        assert_eq!(summarize("\n2 tests: 1 passed, 0 failed, 1 quarantined failures, in 2.0s\n", true), "1 of 2 tests passed, 1 set aside failed");
        assert_eq!(summarize("error: fastpilot run needs Claude Code installed\n", false), "did not run: error: fastpilot run needs Claude Code installed");
        // QA B7: the reason is in the table, not only in the report.
        let qa = "✗ demo-login-shows-budi 0.4s, 2 turns, claude-opus-5-5 [product]: variable `EMAIL` is not allowed; list it in FASTPILOT_VARS\n\n1 tests: 0 passed, 1 failed, 0 quarantined failures, in 3.0s\n";
        assert_eq!(summarize(qa, false), "0 of 1 test passed, 1 failed — demo-login-shows-budi: variable `EMAIL` is not allowed; list it in FASTPILOT_VARS");
        let many: String = (1..=5).map(|i| format!("✗ t{i} 1.0s, 1 turns, m: broke {i}\n")).collect::<String>() + "5 tests: 0 passed, 5 failed, 0 quarantined failures, in 1s\n";
        assert!(summarize(&many, false).ends_with("t3: broke 3; and 2 more"));
    }

    /// D10b: an agent's scheduled run uses its provider and model, its extra words, its id.
    #[test]
    fn an_agents_schedule_runs_with_its_model() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fastpilot-sched-agent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe { std::env::set_var("FASTPILOT_CONFIG", dir.join("config.toml")) };
        let a = crate::agents::create(crate::agents::Agent {
            name: "Groq tester".into(),
            job: "x".into(),
            devices: vec!["web".into()],
            provider: Some("groq".into()),
            model: Some("some-model".into()),
            blocked: vec!["Confirm order".into()],
            ..Default::default()
        })
        .expect("agent");
        let env = agent_env(Some(&a.id));
        let get = |k: &str| env.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str());
        assert_eq!(get("FASTPILOT_AGENT"), Some("groq-tester"));
        assert_eq!((get("FASTPILOT_PROVIDER"), get("FASTPILOT_PROVIDER_MODEL")), (Some("groq"), Some("some-model")));
        assert_eq!(get("FASTPILOT_PROVIDER_BASE_URL"), Some("https://api.groq.com/openai/v1"));
        assert_eq!(get("FASTPILOT_BLOCK_ACTIONS"), Some("default,Confirm order"));
        assert!(agent_env(None).is_empty() && agent_env(Some("nobody")).is_empty());
        unsafe { std::env::remove_var("FASTPILOT_CONFIG") };
        let _ = std::fs::remove_dir_all(dir);
    }
}
