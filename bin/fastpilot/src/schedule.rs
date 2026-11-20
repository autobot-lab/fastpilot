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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Every {
    Day,
    Weekday,
    Week,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct When {
    pub every: Every,
    /// `HH:MM`, local time of this machine.
    pub time: String,
    /// For `week`: 1 = Monday ... 7 = Sunday.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<u8>,
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
        return Err("URL webhook harus https:// (http hanya untuk 127.0.0.1/localhost)".into());
    }
    let host = url.split("://").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or("");
    if host.is_empty() || host.contains('@') {
        return Err("URL webhook tidak valid: tulis tanpa user:password@".into());
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
        return Err("Id jadwal hanya boleh huruf kecil, angka, dan -".into());
    }
    if s.name.trim().is_empty() || s.name.chars().count() > 80 {
        return Err("Nama jadwal wajib, 1–80 karakter".into());
    }
    let tests = Path::new(&s.tests);
    if !tests.is_absolute() || !tests.exists() {
        return Err(format!("Folder atau file test tidak ditemukan: {} (tulis path lengkap)", s.tests));
    }
    if tests.is_file() && tests.extension().is_none_or(|x| x != "md") {
        return Err("File test harus .md (atau pilih foldernya)".into());
    }
    if hour_minute(&s.when.time).is_none() {
        return Err("Jam harus HH:MM, mis. 08:00".into());
    }
    match (s.when.every, s.when.day) {
        (Every::Week, Some(1..=7)) | (Every::Day | Every::Weekday, None) => {}
        (Every::Week, _) => return Err("Pilih hari untuk jadwal mingguan".into()),
        _ => return Err("Hari hanya untuk jadwal mingguan".into()),
    }
    if let Some(repo) = &s.notify.github
        && !valid_repo(repo)
    {
        return Err("Repo GitHub ditulis pemilik/nama, mis. autobot-lab/fastpilot".into());
    }
    Ok(())
}

/// `Setiap hari 09:00`, `Hari kerja 08:00`, `Senin 07:00`.
pub fn describe(w: &When) -> String {
    const DAYS: [&str; 7] = ["Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu", "Minggu"];
    match w.every {
        Every::Day => format!("Setiap hari {}", w.time),
        Every::Weekday => format!("Hari kerja {}", w.time),
        Every::Week => format!("{} {}", DAYS.get(usize::from(w.day.unwrap_or(1).clamp(1, 7)) - 1).unwrap_or(&"Senin"), w.time),
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
    let (h, m) = hour_minute(&s.when.time).unwrap_or((9, 0));
    let entry = |day: Option<u8>| {
        // launchd: 0 and 7 are Sunday, 1 Monday.
        let day = day.map(|d| format!("<key>Weekday</key><integer>{}</integer>", d % 7)).unwrap_or_default();
        format!("<dict><key>Hour</key><integer>{h}</integer><key>Minute</key><integer>{m}</integer>{day}</dict>")
    };
    let times = match s.when.every {
        Every::Day => entry(None),
        Every::Weekday => (1..=5).map(|d| entry(Some(d))).collect(),
        Every::Week => entry(s.when.day),
    };
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
    let (h, m) = hour_minute(&s.when.time).unwrap_or((9, 0));
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let days = match s.when.every {
        Every::Day => String::new(),
        Every::Weekday => "Mon..Fri ".into(),
        Every::Week => format!("{} ", DAYS[usize::from(s.when.day.unwrap_or(1).clamp(1, 7)) - 1]),
    };
    let service = format!(
        "[Unit]\nDescription=fastpilot schedule {id}\n\n[Service]\nType=oneshot\nEnvironment={}\nExecStart={} schedule run {id}\n",
        systemd_word(&format!("PATH={path}")),
        systemd_word(exe),
        id = s.id
    );
    let timer = format!(
        "[Unit]\nDescription=fastpilot schedule {id}\n\n[Timer]\nOnCalendar={days}*-*-* {h:02}:{m:02}:00\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n",
        id = s.id
    );
    (service, timer)
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn cron_line(s: &Schedule, exe: &str, path: &str) -> String {
    let (h, m) = hour_minute(&s.when.time).unwrap_or((9, 0));
    let dow = match s.when.every {
        Every::Day => "*".to_owned(),
        Every::Weekday => "1-5".to_owned(),
        Every::Week => (s.when.day.unwrap_or(1) % 7).to_string(),
    };
    format!("{m} {h} * * {dow} PATH={} {} schedule run {} # fastpilot-schedule:{}", shell_quote(path), shell_quote(exe), s.id, s.id)
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
            lines.push(cron_line(s, &exe, &path));
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
        return Err("Isi URL webhook dulu, atau matikan notifikasi webhook".into());
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

pub fn set_enabled(id: &str, on: bool) -> Res<()> {
    let mut store = load();
    let s = store.schedules.iter_mut().find(|x| x.id == id).ok_or("jadwal tidak ditemukan")?;
    s.enabled = on;
    if on { install(s)? } else { uninstall(id)? }
    save_store(&store)
}

pub fn delete(id: &str) -> Res<()> {
    let mut store = load();
    if !store.schedules.iter().any(|x| x.id == id) {
        return Err("jadwal tidak ditemukan".into());
    }
    uninstall(id)?;
    let _ = std::fs::remove_file(secret_path(id));
    store.schedules.retain(|x| x.id != id);
    store.last.remove(id);
    save_store(&store)
}

/// Start `fastpilot schedule run <id>` in the background (Jalankan sekarang).
pub fn start_now(id: &str) -> Res<()> {
    if !load().schedules.iter().any(|x| x.id == id) {
        return Err("jadwal tidak ditemukan".into());
    }
    let log = home().join("schedules").join(format!("{id}.log"));
    std::fs::create_dir_all(log.parent().unwrap_or(&home())).map_err(|e| format!("Tidak bisa menyiapkan file jadwal: {e}"))?;
    let out = std::fs::OpenOptions::new().create(true).append(true).open(&log).map_err(|e| format!("Tidak bisa menyiapkan file jadwal: {e}"))?;
    let err = out.try_clone().map_err(|e| format!("Tidak bisa menyiapkan file jadwal: {e}"))?;
    std::process::Command::new(exe()?)
        .args(["schedule", "run", id])
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|e| format!("Run tidak bisa dimulai: {e}"))?;
    Ok(())
}

// ---- running ------------------------------------------------------------------------------

/// `fastpilot schedule run <id>`: the suite with `fastpilot run`, its result kept for the page,
/// and the notifications when it fails.
pub async fn run(id: &str) -> Res<bool> {
    let store = load();
    let s = store.schedules.iter().find(|x| x.id == id).cloned().ok_or_else(|| format!("Jadwal `{id}` tidak ditemukan"))?;
    let started = now_ms();
    let reports = home().join("schedules").join("reports");
    std::fs::create_dir_all(&reports).map_err(|e| format!("Tidak bisa menyiapkan file jadwal: {e}"))?;
    let report = reports.join(format!("{id}-{started}.html"));
    set_last(id, LastRun { started_ms: started, ..Default::default() })?;
    // Test cases name pages and files relative to the project (the folder above `tests/`), and
    // the OS scheduler starts us in `/`.
    let tests = Path::new(&s.tests);
    let folder = if tests.is_file() { tests.parent() } else { Some(tests) };
    let project = folder.and_then(Path::parent).unwrap_or(Path::new("/"));
    let out = tokio::process::Command::new(exe()?)
        .args(["run", &s.tests, "--html", &report.to_string_lossy()])
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
    let mut last = LastRun { started_ms: started, finished_ms: Some(now_ms()), passed: Some(passed), summary: summary.clone(), report: report.clone(), notified: vec![] };
    if !passed {
        let msg = format!(
            "fastpilot: jadwal \"{}\" gagal — {summary}{}",
            s.name,
            report.as_deref().map(|r| format!("\nLaporan: {r}")).unwrap_or_default()
        );
        last.notified = notify(&s, &format!("Jadwal \"{}\" gagal", s.name), &msg).await;
    }
    set_last(id, last)?;
    Ok(passed)
}

/// The suite's own summary line (`3 tests: 2 passed, 1 failed, ...`) and the failed tests.
pub fn summarize(output: &str, passed: bool) -> String {
    // `3 tests: 2 passed, 1 failed, 0 quarantined failures, in …` -> `2 dari 3 test lolos, 1 gagal`.
    let line = output.lines().rev().find(|l| l.contains(" tests: ") && l.contains(" passed")).map(|l| {
        let num = |after: &str| l.split(after).next().and_then(|x| x.rsplit([' ', ':', ',']).find(|w| !w.is_empty())).and_then(|w| w.parse::<u64>().ok()).unwrap_or(0);
        let (total, ok, bad, quarantined) = (num(" tests:"), num(" passed"), num(" failed"), num(" quarantined failures"));
        let mut t = format!("{ok} dari {total} test lolos");
        if bad > 0 {
            t.push_str(&format!(", {bad} gagal"));
        }
        if quarantined > 0 {
            t.push_str(&format!(", {quarantined} gagal di karantina"));
        }
        t
    });
    let failed: Vec<&str> = output.lines().filter_map(|l| l.strip_prefix("✗ ")).filter_map(|l| l.split_whitespace().next()).collect();
    let mut text = line.unwrap_or_else(|| {
        let err = output.lines().rev().find(|l| l.contains("error") || l.contains("Error")).unwrap_or("").trim();
        if passed { "lolos".to_owned() } else if err.is_empty() { "tidak jalan".to_owned() } else { format!("tidak jalan: {}", err.chars().take(300).collect::<String>()) }
    });
    if !failed.is_empty() {
        text.push_str(&format!(" (gagal: {})", failed.join(", ")));
    }
    text
}

// ---- notifications ------------------------------------------------------------------------

/// Send `msg` on every channel the schedule has; one line per channel saying how it went.
pub async fn notify(s: &Schedule, title: &str, msg: &str) -> Vec<String> {
    let mut done = Vec::new();
    if s.notify.webhook {
        done.push(match std::fs::read_to_string(secret_path(&s.id)) {
            Ok(url) => match webhook(url.trim(), msg).await {
                Ok(()) => "Webhook: terkirim".to_owned(),
                Err(e) => format!("Webhook: gagal ({e})"),
            },
            Err(_) => "Webhook: URL belum diisi".to_owned(),
        });
    }
    if s.notify.os {
        done.push(match os_notification(title, msg) {
            Ok(()) => "Notifikasi: terkirim".to_owned(),
            Err(e) => format!("Notifikasi: gagal ({e})"),
        });
    }
    if let Some(repo) = &s.notify.github {
        done.push(match github_issue(repo, title, msg).await {
            Ok(url) => format!("Issue GitHub: {url}"),
            Err(e) => format!("Issue GitHub: gagal ({e})"),
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

pub const USAGE: &str = "usage: fastpilot schedule <list | run ID | remove ID>\n  schedules are made in `fastpilot ui` (Jadwal); the OS scheduler runs `fastpilot schedule run ID`";

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
            when: When { every, time: "08:05".into(), day },
            notify: Notify::default(),
            enabled: true,
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
        assert_eq!(describe(&When { every: Every::Week, time: "07:00".into(), day: Some(1) }), "Senin 07:00");
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
        let line = cron_line(&s, "/opt/it's/fastpilot", "/usr/bin");
        assert_eq!(line, "5 8 * * 1-5 PATH='/usr/bin' '/opt/it'\\''s/fastpilot' schedule run login-harian # fastpilot-schedule:login-harian");
    }

    #[test]
    fn summary_of_a_run() {
        let out = "3 tests, 1 workers, web\n✓ login 3.1s, 4 turns, haiku: ok\n✗ checkout 9.0s, 7 turns, haiku: no Bayar button\n\n3 tests: 2 passed, 1 failed, 0 quarantined failures, in 20.0s; tokens in 1 out 2\nhtml: /x.html\n";
        assert_eq!(summarize(out, false), "2 dari 3 test lolos, 1 gagal (gagal: checkout)");
        assert_eq!(summarize("\n1 tests: 1 passed, 0 failed, 0 quarantined failures, in 2.0s\n", true), "1 dari 1 test lolos");
        assert_eq!(summarize("\n2 tests: 1 passed, 0 failed, 1 quarantined failures, in 2.0s\n", true), "1 dari 2 test lolos, 1 gagal di karantina");
        assert_eq!(summarize("error: fastpilot run needs Claude Code installed\n", false), "tidak jalan: error: fastpilot run needs Claude Code installed");
    }
}
