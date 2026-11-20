//! History (D7, v0.4): the past runs and Chat conversations of a project, kept under `.fastpilot/`
//! (or `FASTPILOT_DIR`), and how they are deleted.
//! - `runs/<run>/`: one test run (`report.md`, `report.html`, screenshots, recordings), written by
//!   [`crate::Run::finish`]. A run that is part of a suite shows inside that suite.
//! - `history/suites/<id>.json`: one `fastpilot run` of several tests (a schedule, the CLI): its
//!   test runs and its HTML report.
//! - `history/chats/<id>.json`: one Chat conversation.
//! - `history/settings.json`: `keep_days`, auto-delete after N days. Off unless the user picks it.
//!
//! Deleting removes the run folders (reports, screenshots, recordings) and the suite's report when
//! fastpilot wrote it. Tests, schedules and path memory stay.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One `fastpilot run` of several tests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Suite {
    pub id: String,
    pub title: String,
    pub platforms: Vec<String>,
    pub started_ms: u64,
    pub finished_ms: u64,
    pub passed: u32,
    pub failed: u32,
    /// Tests that could not run (no device, timed out before starting).
    #[serde(default)]
    pub not_run: u32,
    /// Its test runs: folder names under `runs/`.
    #[serde(default)]
    pub runs: Vec<String>,
    /// The suite's HTML report, when there is one.
    #[serde(default)]
    pub report: Option<String>,
    /// fastpilot wrote that report (into History or a schedule's report folder): deleting the
    /// suite deletes it. A report the user asked for elsewhere (`--html out.html`) stays.
    #[serde(default)]
    pub report_owned: bool,
}

/// A row on the History page: a suite, or a single test run outside any suite.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    /// `suite:<id>` or `run:<folder>`.
    pub id: String,
    pub title: String,
    pub tests: u32,
    pub passed: u32,
    pub failed: u32,
    pub not_run: u32,
    pub platforms: Vec<String>,
    pub finished_ms: u64,
    pub has_report: bool,
}

/// A saved Chat conversation, as the list shows it.
#[derive(Debug, Clone, Serialize)]
pub struct ChatItem {
    pub id: String,
    pub title: String,
    pub messages: usize,
    pub updated_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    keep_days: Option<u32>,
}

/// What a delete removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Deleted {
    pub runs: usize,
    pub chats: usize,
}

pub struct History {
    root: PathBuf,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Ids are made here (`<ms>` or `<ms>-<n>`) or are run folder names (`<stamp>-<test_id>`): letters,
/// digits, `-` and `_` only, so an id can never name a path outside its folder.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 160 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn modified_ms(p: &Path) -> u64 {
    fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Owner-only file: conversations can hold anything the user typed.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // A new, unguessable temp name, created fresh: a link a cloned project put at a fixed
    // `<name>.tmp` is never followed (Security re-review, finding 4).
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}-{}.tmp", std::process::id(), now_ms()));
    {
        use std::io::Write;
        let mut o = fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&tmp)?;
        f.write_all(text.as_bytes())?;
    }
    fs::rename(&tmp, path)
}

impl History {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The project's `.fastpilot` (or `FASTPILOT_DIR`), as [`crate::Memory::from_env`].
    pub fn from_env() -> Self {
        Self::new(fastpilot_core::config::var_os("FASTPILOT_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".fastpilot")))
    }

    fn dir(&self) -> PathBuf {
        self.root.join("history")
    }

    fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    /// Where fastpilot writes suite reports: this History's `suites/` and the schedules' report
    /// folder (`~/.fastpilot/schedules/reports`).
    fn report_dirs(&self) -> Vec<PathBuf> {
        let home = fastpilot_core::config::path().parent().map(Path::to_path_buf).unwrap_or_default();
        [self.dir().join("suites"), home.join("schedules").join("reports")].into_iter().filter_map(|d| fs::canonicalize(d).ok()).collect()
    }

    /// A suite's report path, only when it is a real `.html` file inside a folder fastpilot writes
    /// reports to. The path comes from a file in the project, which a cloned project can forge
    /// (Security re-review, finding 3): anything else is never served or deleted.
    fn own_report(&self, report: &str) -> Option<PathBuf> {
        let p = Path::new(report);
        let meta = fs::symlink_metadata(p).ok()?;
        if !meta.is_file() || p.extension().is_none_or(|x| x != "html") {
            return None;
        }
        let real = fs::canonicalize(p).ok()?;
        self.report_dirs().iter().any(|d| real.parent() == Some(d.as_path())).then_some(real)
    }

    fn suite_path(&self, id: &str) -> PathBuf {
        self.dir().join("suites").join(format!("{id}.json"))
    }

    fn chat_path(&self, id: &str) -> PathBuf {
        self.dir().join("chats").join(format!("{id}.json"))
    }

    /// A new id, unique in `folder` (`<ms>`, else `<ms>-2`, …).
    fn new_id(&self, folder: &str) -> String {
        let base = now_ms().to_string();
        (1..)
            .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
            .find(|id| !self.dir().join(folder).join(format!("{id}.json")).exists())
            .unwrap_or(base)
    }

    /// Where a suite's own HTML report goes, when the run was not asked for one elsewhere (its
    /// folder is created).
    pub fn suite_report_path(&self, id: &str) -> PathBuf {
        let dir = self.dir().join("suites");
        let _ = fs::create_dir_all(&dir);
        dir.join(format!("{id}.html"))
    }

    pub fn new_suite_id(&self) -> String {
        self.new_id("suites")
    }

    /// Keep a suite (after `fastpilot run`); old items go first when auto-delete is on.
    pub fn record_suite(&self, s: &Suite) -> std::io::Result<()> {
        if !safe_id(&s.id) {
            return Err(std::io::Error::other(format!("bad history id {}", s.id)));
        }
        let text = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
        write_private(&self.suite_path(&s.id), &text)?;
        self.prune(now_ms());
        Ok(())
    }

    fn suites(&self) -> Vec<Suite> {
        let Ok(rd) = fs::read_dir(self.dir().join("suites")) else { return vec![] };
        rd.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<Suite>(&t).ok())
            .filter(|s| safe_id(&s.id))
            .collect()
    }

    /// `# <test_id> — pass|fail` from a run's report.md; `None` while it has no result.
    fn run_result(dir: &Path) -> Option<bool> {
        let md = fs::read_to_string(dir.join("report.md")).ok()?;
        let first = md.lines().next()?;
        if first.ends_with("— pass") {
            Some(true)
        } else if first.ends_with("— fail") {
            Some(false)
        } else {
            None
        }
    }

    /// Run folders: (name, finished or last changed).
    fn run_folders(&self) -> Vec<(String, PathBuf)> {
        let Ok(rd) = fs::read_dir(self.runs_dir()) else { return vec![] };
        rd.flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().to_str().map(|n| (n.to_owned(), e.path())))
            .filter(|(n, _)| safe_id(n))
            .collect()
    }

    /// Every run, newest first: suites, and the test runs that are in no suite.
    pub fn runs(&self) -> Vec<Item> {
        let suites = self.suites();
        let in_suite: std::collections::HashSet<&str> = suites.iter().flat_map(|s| s.runs.iter().map(String::as_str)).collect();
        let mut items: Vec<Item> = suites
            .iter()
            .map(|s| Item {
                id: format!("suite:{}", s.id),
                title: s.title.clone(),
                tests: s.passed + s.failed + s.not_run,
                passed: s.passed,
                failed: s.failed,
                not_run: s.not_run,
                platforms: s.platforms.clone(),
                finished_ms: s.finished_ms,
                has_report: s.report.as_deref().and_then(|r| self.own_report(r)).is_some(),
            })
            .collect();
        for (name, dir) in self.run_folders() {
            if in_suite.contains(name.as_str()) {
                continue;
            }
            let result = Self::run_result(&dir);
            let test = name.splitn(3, '-').nth(2).unwrap_or(&name).to_owned();
            items.push(Item {
                id: format!("run:{name}"),
                title: test,
                tests: 1,
                passed: u32::from(result == Some(true)),
                failed: u32::from(result == Some(false)),
                not_run: u32::from(result.is_none()),
                platforms: vec![],
                finished_ms: modified_ms(&dir.join("report.md")).max(modified_ms(&dir)),
                has_report: dir.join("report.html").is_file(),
            });
        }
        items.sort_by(|a, b| b.finished_ms.cmp(&a.finished_ms).then(b.id.cmp(&a.id)));
        items
    }

    /// The HTML report of a row (`suite:<id>` or `run:<folder>`), only inside this project's
    /// History or a report fastpilot wrote for the suite.
    pub fn report(&self, id: &str) -> Option<PathBuf> {
        if let Some(name) = id.strip_prefix("run:").filter(|n| safe_id(n)) {
            let p = self.runs_dir().join(name).join("report.html");
            return p.is_file().then_some(p);
        }
        let sid = id.strip_prefix("suite:").filter(|n| safe_id(n))?;
        let s: Suite = serde_json::from_str(&fs::read_to_string(self.suite_path(sid)).ok()?).ok()?;
        self.own_report(&s.report?)
    }

    /// Keep a Chat conversation (after each answer). `messages` is what the Chat page shows.
    pub fn save_chat(&self, id: &str, messages: &Value) -> std::io::Result<()> {
        if !safe_id(id) {
            return Err(std::io::Error::other(format!("bad history id {id}")));
        }
        let title = messages
            .as_array()
            .and_then(|m| m.iter().find(|m| m["role"] == "user"))
            .and_then(|m| m["text"].as_str())
            .map(|t| {
                let line = t.lines().next().unwrap_or("").trim();
                if line.chars().count() > 80 { format!("{}…", line.chars().take(80).collect::<String>()) } else { line.to_owned() }
            })
            .unwrap_or_default();
        let doc = serde_json::json!({ "id": id, "title": title, "updated_ms": now_ms(), "messages": messages });
        write_private(&self.chat_path(id), &doc.to_string())?;
        self.prune(now_ms());
        Ok(())
    }

    /// A new conversation id.
    pub fn new_chat_id(&self) -> String {
        self.new_id("chats")
    }

    pub fn chats(&self) -> Vec<ChatItem> {
        let Ok(rd) = fs::read_dir(self.dir().join("chats")) else { return vec![] };
        let mut out: Vec<ChatItem> = rd
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_str::<Value>(&fs::read_to_string(e.path()).ok()?).ok())
            .filter_map(|v| {
                let id = v["id"].as_str().filter(|i| safe_id(i))?.to_owned();
                Some(ChatItem {
                    title: v["title"].as_str().filter(|t| !t.is_empty()).unwrap_or("(no message)").to_owned(),
                    messages: v["messages"].as_array().map_or(0, Vec::len),
                    updated_ms: v["updated_ms"].as_u64().unwrap_or(0),
                    id,
                })
            })
            .collect();
        out.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then(b.id.cmp(&a.id)));
        out
    }

    /// One conversation: `{id, title, updated_ms, messages}`.
    pub fn chat(&self, id: &str) -> Option<Value> {
        if !safe_id(id) {
            return None;
        }
        serde_json::from_str(&fs::read_to_string(self.chat_path(id)).ok()?).ok()
    }

    fn remove_run_folder(&self, name: &str) -> bool {
        if !safe_id(name) {
            return false;
        }
        let dir = self.runs_dir().join(name);
        // A real folder of this project's runs, never a link to somewhere else.
        let real = fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir());
        real && fs::remove_dir_all(&dir).is_ok()
    }

    fn remove_suite(&self, id: &str) -> bool {
        if !safe_id(id) {
            return false;
        }
        let path = self.suite_path(id);
        let Some(s) = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Suite>(&t).ok()) else {
            return fs::remove_file(&path).is_ok();
        };
        for r in &s.runs {
            self.remove_run_folder(r);
        }
        // Only a report fastpilot wrote, inside its own report folders; never a link.
        if let Some(p) = s.report.as_deref().filter(|_| s.report_owned).and_then(|r| self.own_report(r)) {
            let _ = fs::remove_file(p);
        }
        let _ = fs::remove_file(self.suite_report_path(id));
        fs::remove_file(&path).is_ok()
    }

    /// Delete the given rows (`suite:<id>`, `run:<folder>`) and conversations.
    pub fn delete(&self, runs: &[String], chats: &[String]) -> Deleted {
        let mut d = Deleted::default();
        for id in runs {
            let ok = if let Some(s) = id.strip_prefix("suite:") {
                self.remove_suite(s)
            } else if let Some(r) = id.strip_prefix("run:") {
                self.remove_run_folder(r)
            } else {
                false
            };
            d.runs += usize::from(ok);
        }
        for id in chats {
            d.chats += usize::from(safe_id(id) && fs::remove_file(self.chat_path(id)).is_ok());
        }
        d
    }

    /// Delete every run and conversation of this project.
    pub fn delete_all(&self) -> Deleted {
        let runs: Vec<String> = self.runs().into_iter().map(|i| i.id).collect();
        let chats: Vec<String> = self.chats().into_iter().map(|c| c.id).collect();
        self.delete(&runs, &chats)
    }

    pub fn keep_days(&self) -> Option<u32> {
        fs::read_to_string(self.dir().join("settings.json")).ok().and_then(|t| serde_json::from_str::<Settings>(&t).ok()).and_then(|s| s.keep_days).filter(|d| *d > 0)
    }

    /// Auto-delete after `days` (None: keep everything). Applied at once.
    pub fn set_keep_days(&self, days: Option<u32>) -> std::io::Result<Deleted> {
        let days = days.filter(|d| *d > 0);
        if days.is_some_and(|d| d > 3650) {
            return Err(std::io::Error::other("auto-delete takes 1 to 3650 days"));
        }
        let text = serde_json::to_string(&Settings { keep_days: days }).map_err(std::io::Error::other)?;
        write_private(&self.dir().join("settings.json"), &text)?;
        Ok(self.prune(now_ms()))
    }

    /// Auto-delete: rows and conversations older than `keep_days` at `now`.
    pub fn prune(&self, now: u64) -> Deleted {
        let Some(days) = self.keep_days() else { return Deleted::default() };
        let cutoff = now.saturating_sub(u64::from(days) * 86_400_000);
        let runs: Vec<String> = self.runs().into_iter().filter(|i| i.finished_ms > 0 && i.finished_ms < cutoff).map(|i| i.id).collect();
        let chats: Vec<String> = self.chats().into_iter().filter(|c| c.updated_ms > 0 && c.updated_ms < cutoff).map(|c| c.id).collect();
        self.delete(&runs, &chats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fp-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).expect("tmp");
        d
    }

    fn run(root: &Path, name: &str, status: Option<&str>) {
        let d = root.join("runs").join(name);
        fs::create_dir_all(&d).expect("run");
        if let Some(s) = status {
            fs::write(d.join("report.md"), format!("# x — {s}\n")).expect("md");
            fs::write(d.join("report.html"), "<p>r</p>").expect("html");
        }
        fs::write(d.join("fail-1.jpg"), b"jpg").expect("shot");
    }

    #[test]
    fn suites_hold_their_runs_and_deleting_removes_the_evidence() {
        let root = tmp("suite");
        let h = History::new(&root);
        run(&root, "20261120-100000-login", Some("pass"));
        run(&root, "20261120-100001-cart", Some("fail"));
        run(&root, "20261120-110000-solo", Some("fail"));
        run(&root, "20261120-120000-open", None);
        let id = h.new_suite_id();
        let report = h.suite_report_path(&id);
        fs::write(&report, "<p>suite</p>").expect("report");
        let s = Suite {
            id: id.clone(),
            title: "Checkout every 2 hours".into(),
            platforms: vec!["web".into()],
            started_ms: 1,
            finished_ms: now_ms(),
            passed: 1,
            failed: 1,
            runs: vec!["20261120-100000-login".into(), "20261120-100001-cart".into()],
            report: Some(report.to_string_lossy().into_owned()),
            report_owned: true,
            ..Suite::default()
        };
        h.record_suite(&s).expect("record");
        let items = h.runs();
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(items.len(), 3, "the suite and the two runs outside it: {ids:?}");
        let suite = items.iter().find(|i| i.id == format!("suite:{id}")).expect("suite row");
        assert_eq!((suite.tests, suite.passed, suite.failed, suite.has_report), (2, 1, 1, true));
        let solo = items.iter().find(|i| i.id == "run:20261120-110000-solo").expect("solo");
        assert_eq!((solo.title.as_str(), solo.failed), ("solo", 1));
        let open = items.iter().find(|i| i.id == "run:20261120-120000-open").expect("no result yet");
        assert_eq!(open.not_run, 1);
        assert_eq!(h.report(&format!("suite:{id}")), Some(fs::canonicalize(&report).expect("real")));
        assert!(h.report("run:20261120-110000-solo").is_some());
        // Paths are never taken from the request.
        for bad in ["run:../x", "suite:../../etc", "run:", "suite:a/b", "other"] {
            assert!(h.report(bad).is_none(), "{bad}");
        }

        let d = h.delete(&[format!("suite:{id}"), "run:../runs".into(), "run:nope".into()], &[]);
        assert_eq!(d, Deleted { runs: 1, chats: 0 });
        assert!(!root.join("runs/20261120-100000-login").exists() && !root.join("runs/20261120-100001-cart").exists(), "the suite's runs");
        assert!(!report.exists(), "its report");
        assert!(root.join("runs/20261120-110000-solo").exists(), "other runs stay");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_report_the_user_asked_for_elsewhere_is_not_deleted() {
        let root = tmp("owned");
        let h = History::new(&root);
        let mine = root.join("mine.html");
        fs::write(&mine, "<p>mine</p>").expect("mine");
        let s = Suite { id: "1".into(), title: "t".into(), finished_ms: now_ms(), report: Some(mine.to_string_lossy().into_owned()), report_owned: false, ..Suite::default() };
        h.record_suite(&s).expect("record");
        assert_eq!(h.delete(&["suite:1".into()], &[]).runs, 1);
        assert!(mine.exists());
        let _ = fs::remove_dir_all(root);
    }

    /// Security re-review, finding 3: a cloned project's forged suite file names a report outside
    /// History. It is never served, and neither deleting nor auto-delete removes it.
    #[test]
    fn a_forged_suite_file_cannot_serve_or_delete_other_files() {
        let root = tmp("forged");
        let victim = root.join("outside").join("x.html");
        fs::create_dir_all(victim.parent().expect("dir")).expect("dir");
        fs::write(&victim, "<p>mine</p>").expect("victim");
        let h = History::new(root.join("project"));
        // A link inside History's own report folder that points outside counts as outside too.
        let link = h.suite_report_path("2");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&victim, &link).expect("link");
        for (id, report) in [("1", victim.clone()), ("2", link.clone()), ("3", h.dir().join("suites").join("..").join("..").join("..").join("outside").join("x.html"))] {
            let s = Suite { id: id.into(), title: "forged".into(), finished_ms: 1, report: Some(report.to_string_lossy().into_owned()), report_owned: true, ..Suite::default() };
            h.record_suite(&s).expect("record");
            assert_eq!(h.report(&format!("suite:{id}")), None, "{id} is not served");
        }
        assert!(h.runs().iter().all(|i| !i.has_report));
        fs::write(h.dir().join("settings.json"), r#"{"keep_days":1}"#).expect("settings");
        assert_eq!(h.prune(now_ms()).runs, 3, "the rows go");
        assert!(victim.exists(), "the file outside History stays");
        let _ = fs::remove_dir_all(root);
    }

    /// Security re-review, finding 4: a link a cloned project put at `history/settings.tmp` is never
    /// written through.
    #[cfg(unix)]
    #[test]
    fn a_planted_temp_link_is_never_written_through() {
        let root = tmp("tmplink");
        let victim = root.join("victim.txt");
        fs::write(&victim, "keep me").expect("victim");
        let h = History::new(root.join("project"));
        fs::create_dir_all(h.dir()).expect("dir");
        std::os::unix::fs::symlink(&victim, h.dir().join("settings.tmp")).expect("link");
        h.set_keep_days(Some(7)).expect("set");
        assert_eq!(fs::read_to_string(&victim).expect("victim"), "keep me");
        assert_eq!(h.keep_days(), Some(7));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn conversations_are_saved_privately_listed_and_deleted() {
        let root = tmp("chat");
        let h = History::new(&root);
        let id = h.new_chat_id();
        let msgs = serde_json::json!([{ "role": "user", "text": "test the login page\nplease" }, { "role": "bot", "text": "done" }]);
        h.save_chat(&id, &msgs).expect("save");
        let list = h.chats();
        assert_eq!((list.len(), list[0].title.as_str(), list[0].messages), (1, "test the login page", 2));
        assert_eq!(h.chat(&id).expect("chat")["messages"][1]["text"], "done");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(root.join(format!("history/chats/{id}.json"))).expect("file").permissions().mode();
            assert_eq!(mode & 0o077, 0, "owner only: {mode:o}");
        }
        assert!(h.save_chat("../x", &msgs).is_err() && h.chat("../x").is_none());
        assert_eq!(h.delete(&[], std::slice::from_ref(&id)), Deleted { runs: 0, chats: 1 });
        assert!(h.chats().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_delete_removes_only_what_is_older_and_delete_all_empties_it() {
        let root = tmp("prune");
        let h = History::new(&root);
        run(&root, "20261001-000000-old", Some("pass"));
        run(&root, "20261120-000000-new", Some("pass"));
        let old = Suite { id: "100".into(), title: "old".into(), finished_ms: 1_000, ..Suite::default() };
        h.record_suite(&old).expect("old");
        h.save_chat("200", &serde_json::json!([{ "role": "user", "text": "hi" }])).expect("chat");
        assert_eq!(h.keep_days(), None, "off by default");
        assert_eq!(h.prune(now_ms()), Deleted::default(), "nothing goes while it is off");
        // 30 days from "now" far in the future: everything recorded so far is older.
        let later = now_ms() + 31 * 86_400_000;
        fs::write(root.join("history/settings.json"), r#"{"keep_days":30}"#).expect("settings");
        let gone = h.prune(later);
        assert_eq!(gone, Deleted { runs: 3, chats: 1 });
        // Set at today's date: the old suite (finished at 1 s after 1970) would go, recent ones stay.
        h.record_suite(&Suite { id: "101".into(), title: "new".into(), finished_ms: now_ms(), ..Suite::default() }).expect("new");
        assert_eq!(h.set_keep_days(Some(30)).expect("set"), Deleted::default());
        assert_eq!(h.runs().len(), 1);
        assert!(h.set_keep_days(Some(99_999)).is_err());
        assert_eq!(h.set_keep_days(None).expect("off"), Deleted::default());
        assert_eq!(h.keep_days(), None);
        assert_eq!(h.delete_all(), Deleted { runs: 1, chats: 0 });
        assert!(h.runs().is_empty() && h.chats().is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
