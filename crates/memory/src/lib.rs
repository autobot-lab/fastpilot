//! Path memory + run evidence (PRD §12). Everything lives under `.fastpilot/` (or
//! `FASTPILOT_DIR`):
//! - `runs/<utc>-<test_id>/`: `steps.jsonl`, `assertions.json`, failure screenshots and
//!   snapshots, `report.md`.
//! - `memory/<test_id>.json`: the last passing flow as a *hint* (durable selectors, never refs).
//!   Written only after a run reported as passed; a failed run only updates statistics.

mod triage;

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fastpilot_core::{FlowReport, Recording, escape_html};
use serde::{Deserialize, Serialize};

pub use triage::{Breaker, Class, RetryBudget, Stats, classify, pass_hat_k, wilson};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: {source}")]
    Json { path: PathBuf, source: serde_json::Error },
    #[error("invalid test id `{0}` (use letters, digits, '-' and '_')")]
    TestId(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_owned(), source }
}

/// `FASTPILOT_MEMORY`: `rw` (default), `readonly` (CI: replay reviewed hints, never rewrite
/// them), `off` (no hints, no memory writes). Run evidence is always written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    ReadWrite,
    ReadOnly,
    Off,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PathMemory {
    pub test_id: String,
    /// Passing flow, ready to send as one `run_flow`.
    pub flow: String,
    /// Where the recorded run started (URL / app), to spot a stale hint.
    pub start: String,
    pub screens: Vec<String>,
    pub duration_ms: u64,
    pub pass_count: u32,
    pub fail_count: u32,
    pub last_pass: String,
    /// Most recent failures (UTC), newest last, capped at 10.
    pub failed_at: Vec<String>,
    /// Pass/fail of the last 20 runs, newest last (flake statistics).
    #[serde(default)]
    pub history: Vec<bool>,
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Three failed runs in a row: the hint is withheld until a run passes again.
    #[serde(default)]
    pub stale: bool,
    /// Failure classes of the most recent failed runs, newest last, capped at 10.
    #[serde(default)]
    pub classes: Vec<Class>,
}

/// A test kept out of the suite verdict: it still runs and is reported, but its failure
/// does not fail the suite. `auto` entries come from flakiness and leave by themselves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quarantine {
    pub reason: String,
    pub since: String,
    pub auto: bool,
}

/// What `Run::finish` decided.
#[derive(Debug, Clone)]
pub struct Finished {
    pub report: PathBuf,
    /// Failure class; `None` for a pass.
    pub class: Option<Class>,
    /// History statistics including this run (`None` with `FASTPILOT_MEMORY=off`).
    pub stats: Option<Stats>,
    pub quarantine: Option<Quarantine>,
}

/// Auto-quarantine: ≥ 5 runs, ≥ 2 failures and ≥ 2 flips in the last 10. Released after 10
/// clean runs in a row.
fn auto_quarantine(m: &PathMemory) -> Option<String> {
    let recent = &m.history[m.history.len().saturating_sub(10)..];
    let fails = recent.iter().filter(|p| !**p).count();
    let flips = recent.windows(2).filter(|w| w[0] != w[1]).count();
    (recent.len() >= 5 && fails >= 2 && flips >= 2)
        .then(|| format!("auto: {fails}/{} recent runs failed, {flips} flips", recent.len()))
}

impl PathMemory {
    /// `flaky: 2/10 recent runs failed`, when recent runs both passed and failed.
    pub fn flakiness(&self) -> Option<String> {
        let fails = self.history.iter().filter(|p| !**p).count();
        (fails > 0 && fails < self.history.len())
            .then(|| format!("flaky: {fails}/{} recent runs failed", self.history.len()))
    }
}

/// The `(...)` part of a `screen "T" (location)` header.
fn location(header: &str) -> &str {
    header.rsplit_once('(').map(|(_, l)| l.trim_end_matches(')')).unwrap_or(header)
}

#[derive(Debug, Clone, Serialize)]
pub struct Assertion {
    pub session: String,
    pub what: String,
    pub pass: bool,
    pub detail: String,
}

pub struct Memory {
    root: PathBuf,
    pub mode: Mode,
}

impl Memory {
    pub fn from_env() -> Self {
        let root = fastpilot_core::config::var_os("FASTPILOT_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".fastpilot"));
        let mode = match fastpilot_core::config::var("FASTPILOT_MEMORY").as_deref() {
            Ok("readonly") => Mode::ReadOnly,
            Ok("off") => Mode::Off,
            _ => Mode::ReadWrite,
        };
        Self { root, mode }
    }

    pub fn new(root: impl Into<PathBuf>, mode: Mode) -> Self {
        Self { root: root.into(), mode }
    }

    fn memory_path(&self, test_id: &str) -> PathBuf {
        self.root.join("memory").join(format!("{test_id}.json"))
    }

    fn quarantine_path(&self) -> PathBuf {
        self.root.join("quarantine.json")
    }

    /// Quarantined tests (`.fastpilot/quarantine.json`), visible in every report.
    pub fn quarantined(&self) -> BTreeMap<String, Quarantine> {
        fs::read_to_string(self.quarantine_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn save_quarantine(&self, q: &BTreeMap<String, Quarantine>) -> Result<()> {
        let path = self.quarantine_path();
        fs::create_dir_all(&self.root).map_err(io(&self.root))?;
        let json = serde_json::to_string_pretty(q).map_err(|source| Error::Json { path: path.clone(), source })?;
        fs::write(&path, json).map_err(io(&path))
    }

    /// Quarantine (`Some(reason)`) or release (`None`) a test by hand.
    pub fn set_quarantine(&self, test_id: &str, reason: Option<&str>) -> Result<()> {
        check_id(test_id)?;
        let mut q = self.quarantined();
        match reason {
            Some(r) => {
                q.insert(test_id.to_owned(), Quarantine { reason: r.to_owned(), since: utc_stamp(now_secs(), false), auto: false });
            }
            None => {
                q.remove(test_id);
            }
        }
        self.save_quarantine(&q)
    }

    pub fn load(&self, test_id: &str) -> Option<PathMemory> {
        if self.mode == Mode::Off || check_id(test_id).is_err() {
            return None;
        }
        let text = fs::read_to_string(self.memory_path(test_id)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Text shown to the AI when it starts a session for `test_id`. `current` is the header of
    /// the screen the session starts on, to flag a hint recorded somewhere else.
    pub fn hint(&self, test_id: &str, current: Option<&str>) -> Option<String> {
        let m = self.load(test_id)?;
        if m.flow.is_empty() {
            return None;
        }
        if m.stale {
            return Some(format!(
                "path memory for {test_id} is stale ({} failed runs in a row); explore, then report pass to refresh it",
                m.consecutive_failures
            ));
        }
        let mut notes = Vec::new();
        if let Some(cur) = current.filter(|c| !m.start.is_empty() && location(c) != location(&m.start)) {
            notes.push(format!("recorded from {}, now on {} — the hint may not apply", location(&m.start), location(cur)));
        }
        notes.extend(m.flakiness());
        if let Some(q) = self.quarantined().get(test_id) {
            notes.push(format!("QUARANTINED since {}: {}", q.since, q.reason));
        }
        let notes = if notes.is_empty() { String::new() } else { format!("\n{}", notes.join("\n")) };
        Some(format!(
            "path memory for {test_id}: passed {}x, failed {}x, last pass {}, recorded from {} ({} ms).{notes}\n\
             Hint (send as one run_flow; if a step fails, explore from there):\n```yaml\n{}```",
            m.pass_count, m.fail_count, m.last_pass, m.start, m.duration_ms, m.flow
        ))
    }

    pub fn begin_run(&self, test_id: &str) -> Result<Run> {
        check_id(test_id)?;
        let stamp = utc_stamp(now_secs(), true);
        let runs = self.root.join("runs");
        fs::create_dir_all(&runs).map_err(io(&runs))?;
        // Unique even for runs started in the same second (create_dir fails if it exists).
        let (id, dir) = (1..)
            .map(|n| if n == 1 { format!("{stamp}-{test_id}") } else { format!("{stamp}-{test_id}-{n}") })
            .map(|id| (runs.join(&id), id))
            .find_map(|(dir, id)| match fs::create_dir(&dir) {
                Ok(()) => Some(Ok((id, dir))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(io(&dir)(e))),
            })
            .unwrap_or_else(|| Err(Error::TestId(test_id.to_owned())))?;
        Ok(Run { id, test_id: test_id.to_owned(), dir, ..Run::default() })
    }
}

fn check_id(id: &str) -> Result<()> {
    let ok = !id.is_empty() && id.len() <= 100 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok { Ok(()) } else { Err(Error::TestId(id.to_owned())) }
}

/// One test run: accumulates every `run_flow` and `assert` until `report` closes it.
#[derive(Default)]
pub struct Run {
    pub id: String,
    pub test_id: String,
    dir: PathBuf,
    flows: Vec<String>,
    screens: Vec<String>,
    start: Option<String>,
    duration_ms: u64,
    assertions: Vec<Assertion>,
    steps: usize,
    failures: usize,
    /// `(reason, category)` of the first failure, for classification.
    first_failure: Option<(String, String)>,
    passed_on_retry: bool,
    /// Per-step rows (as in steps.jsonl) and each flow's full response, for report.html.
    rows: Vec<serde_json::Value>,
    outputs: Vec<(bool, String)>,
    shots: Vec<(String, Vec<u8>)>,
    recordings: Vec<(String, Recording)>,
}

impl Run {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// This run is a retry of a failed attempt (suite retry budget): if it passes, the test
    /// is classified flaky.
    pub fn passed_on_retry(&mut self) {
        self.passed_on_retry = true;
    }

    fn note_failure(&mut self, reason: &str, category: &str) {
        self.first_failure.get_or_insert_with(|| (reason.to_owned(), category.to_owned()));
    }

    /// Record a `run_flow`: its durable YAML (refs already replaced), per-step results, and on
    /// failure the rendered session states plus an optional screenshot.
    pub fn record_flow(&mut self, durable_yaml: String, report: &FlowReport, screenshot: Option<Vec<u8>>) -> Result<()> {
        let path = self.dir.join("steps.jsonl");
        let mut f = OpenOptions::new().create(true).append(true).open(&path).map_err(io(&path))?;
        for r in &report.records {
            let line = serde_json::json!({
                "flow": self.flows.len() + 1,
                "step": r.index,
                "session": r.session,
                "label": fastpilot_core::redact(&r.label),
                "ms": r.elapsed.as_millis() as u64,
                "ok": r.error.is_none(),
                "reason": r.error.as_ref().map(|e| e.reason()),
                "category": r.category(),
                "error": r.error.as_ref().map(|e| fastpilot_core::redact(&e.to_string())),
            });
            writeln!(f, "{line}").map_err(io(&path))?;
            self.rows.push(line);
        }
        self.outputs.push((report.passed(), fastpilot_core::redact(&report.to_string())));
        self.steps += report.records.len();
        for s in &report.states {
            if let Some(header) = s.lines().next() {
                if self.start.is_none() {
                    self.start = Some(header.to_owned());
                }
                if !self.screens.iter().any(|h| h == header) {
                    self.screens.push(header.to_owned());
                }
            }
        }
        if let Some((e, category)) = report.records.iter().find_map(|r| r.error.as_ref().zip(r.category())) {
            self.note_failure(e.reason(), category);
        }
        if report.passed() {
            self.flows.push(durable_yaml);
            self.duration_ms += report.elapsed.as_millis() as u64;
        } else {
            self.failures += 1;
            let n = self.failures;
            let snap = self.dir.join(format!("fail-{n}-snapshot.txt"));
            fs::write(&snap, fastpilot_core::redact(&report.to_string())).map_err(io(&snap))?;
            if let Some(jpeg) = screenshot {
                let shot = self.dir.join(format!("fail-{n}.jpg"));
                fs::write(&shot, &jpeg).map_err(io(&shot))?;
                self.shots.push((format!("failure {n}"), jpeg));
            }
        }
        Ok(())
    }

    /// A screenshot for the report (e.g. each session's final screen).
    pub fn add_screenshot(&mut self, label: &str, jpeg: Vec<u8>) {
        self.shots.push((label.to_owned(), jpeg));
    }

    /// A session's screen recording: saved as `recording-<session>.mp4|html` and embedded.
    pub fn add_recording(&mut self, session: &str, rec: Recording) -> Result<()> {
        let safe: String = session.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        let (ext, bytes) = rec.file(&format!("{} {session}", self.test_id));
        let path = self.dir.join(format!("recording-{safe}.{ext}"));
        fs::write(&path, bytes).map_err(io(&path))?;
        self.recordings.push((session.to_owned(), rec));
        Ok(())
    }

    pub fn record_assertion(&mut self, a: Assertion) {
        if !a.pass {
            self.failures += 1;
            self.note_failure("assertion_failed", "product");
        }
        self.assertions.push(a);
    }

    /// Close the run: `report.md`, `assertions.json`, the path memory update, the failure
    /// class and the quarantine decision.
    pub fn finish(self, memory: &Memory, passed: bool, notes: &str) -> Result<Finished> {
        let previous = memory.load(&self.test_id);
        // Only a retry that passed makes the test flaky; a retry that failed again is classified
        // like any failure.
        let retried_pass = passed && self.passed_on_retry;
        let class = (!passed || retried_pass).then(|| {
            let category = self.first_failure.as_ref().map(|(_, c)| c.as_str());
            classify(category, retried_pass, previous.as_ref())
        });
        // Statistics and quarantine include this run (only when memory is written).
        let mut updated = previous.clone().unwrap_or_else(|| PathMemory { test_id: self.test_id.clone(), ..PathMemory::default() });
        let clean = passed && self.failures == 0;
        updated.history.push(clean);
        let extra = updated.history.len().saturating_sub(20);
        updated.history.drain(..extra);
        let mut quarantine = memory.quarantined();
        if memory.mode == Mode::ReadWrite {
            let recent_clean = updated.history.len() >= 10 && updated.history.iter().rev().take(10).all(|p| *p);
            match (quarantine.get(&self.test_id), auto_quarantine(&updated)) {
                (None, Some(reason)) => {
                    quarantine.insert(self.test_id.clone(), Quarantine { reason, since: utc_stamp(now_secs(), false), auto: true });
                    memory.save_quarantine(&quarantine)?;
                }
                (Some(q), _) if q.auto && recent_clean => {
                    quarantine.remove(&self.test_id);
                    memory.save_quarantine(&quarantine)?;
                }
                _ => {}
            }
        }
        let quarantine = quarantine.get(&self.test_id).cloned();
        let stats = (memory.mode != Mode::Off).then(|| updated.stats());
        let stamp = utc_stamp(now_secs(), false);
        let assertions = self.dir.join("assertions.json");
        let json = serde_json::to_string_pretty(&self.assertions).map_err(|source| Error::Json { path: assertions.clone(), source })?;
        fs::write(&assertions, json).map_err(io(&assertions))?;

        let status = if passed { "pass" } else { "fail" };
        let mut md = format!("# {} — {status}\n\n", self.test_id);
        if let Some(q) = &quarantine {
            md.push_str(&format!("> **QUARANTINED** since {}: {} (its result does not fail the suite)\n\n", q.since, q.reason));
        }
        md.push_str(&format!("run: {}\nfinished: {stamp} UTC\nsteps: {}, failures: {}\n", self.id, self.steps, self.failures));
        if let Some(c) = class {
            let why = self.first_failure.as_ref().map(|(r, _)| format!(" (first failure: {r})")).unwrap_or_default();
            md.push_str(&format!("failure class: {}{why}\n", c.as_str()));
        }
        if let Some(st) = &stats {
            md.push_str(&format!("history: {st}\n"));
        }
        md.push('\n');
        if !notes.is_empty() {
            md.push_str(&format!("## Notes\n\n{}\n\n", fastpilot_core::redact(notes)));
        }
        if !self.screens.is_empty() {
            md.push_str("## Screens\n\n");
            for s in &self.screens {
                md.push_str(&format!("- {s}\n"));
            }
            md.push('\n');
        }
        if !self.assertions.is_empty() {
            md.push_str("## Assertions\n\n");
            for a in &self.assertions {
                md.push_str(&format!("- {} [{}] {} {}\n", if a.pass { "✓" } else { "✗" }, a.session, a.what, a.detail));
            }
            md.push('\n');
        }
        md.push_str("Evidence: `report.html` (self-contained), `steps.jsonl`, `assertions.json`, `fail-*.txt|jpg`, `recording-*`.\n");
        let report = self.dir.join("report.md");
        fs::write(&report, fastpilot_core::redact(&md)).map_err(io(&report))?;
        let html = self.html(passed, notes, class, stats.as_ref(), quarantine.as_ref(), &stamp);
        let html_path = self.dir.join("report.html");
        fs::write(&html_path, fastpilot_core::redact(&html)).map_err(io(&html_path))?;

        if memory.mode == Mode::ReadWrite {
            let path = memory.memory_path(&self.test_id);
            let mut m = updated;
            // Only a fully passing run becomes the hint; a failed run never overwrites it.
            if clean && !self.flows.is_empty() {
                m.flow = self.flows.concat();
                m.start = self.start.clone().unwrap_or_default();
                m.screens = self.screens.clone();
                m.duration_ms = self.duration_ms;
                m.pass_count += 1;
                m.last_pass = stamp;
                m.consecutive_failures = 0;
                m.stale = false;
            } else if !clean {
                m.fail_count += 1;
                m.failed_at.push(stamp);
                let extra = m.failed_at.len().saturating_sub(10);
                m.failed_at.drain(..extra);
                m.consecutive_failures += 1;
                m.stale = m.consecutive_failures >= 3;
            }
            if let Some(c) = class {
                m.classes.push(c);
                let extra = m.classes.len().saturating_sub(10);
                m.classes.drain(..extra);
            }
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).map_err(io(dir))?;
            }
            let json = serde_json::to_string_pretty(&m).map_err(|source| Error::Json { path: path.clone(), source })?;
            fs::write(&path, json).map_err(io(&path))?;
        }
        Ok(Finished { report, class, stats, quarantine })
    }
}

impl Run {
    /// One self-contained page: verdict, class, history, steps with timings, every flow's
    /// response (diffs), assertions, screenshots and recordings (all inlined).
    fn html(&self, passed: bool, notes: &str, class: Option<Class>, stats: Option<&Stats>, q: Option<&Quarantine>, stamp: &str) -> String {
        use base64::Engine;
        let e = escape_html;
        let mut b = String::new();
        let status = if passed { "pass" } else { "fail" };
        b.push_str(&format!("<h1>{} <span class=\"badge {status}\">{status}</span></h1>", e(&self.test_id)));
        if let Some(q) = q {
            b.push_str(&format!("<p class=\"warn\">QUARANTINED since {}: {}</p>", e(&q.since), e(&q.reason)));
        }
        b.push_str(&format!("<p class=\"meta\">run {} · finished {stamp} UTC · {} steps · {} failures", e(&self.id), self.steps, self.failures));
        if let Some(c) = class {
            let why = self.first_failure.as_ref().map(|(r, _)| format!(" ({r})")).unwrap_or_default();
            b.push_str(&format!(" · failure class <b>{}</b>{}", c.as_str(), e(&why)));
        }
        b.push_str("</p>");
        if let Some(st) = stats {
            b.push_str(&format!("<p class=\"meta\">history: {}</p>", e(&st.to_string())));
        }
        if !notes.is_empty() {
            b.push_str(&format!("<p>{}</p>", e(notes)));
        }
        if !self.rows.is_empty() {
            b.push_str("<h2>Steps</h2><table><tr><th>flow</th><th>#</th><th>session</th><th>step</th><th>ms</th><th>result</th></tr>");
            for r in &self.rows {
                let ok = r["ok"].as_bool() == Some(true);
                let result = if ok {
                    "✓".to_owned()
                } else {
                    format!("✗ {}: {}", r["reason"].as_str().unwrap_or(""), r["error"].as_str().unwrap_or(""))
                };
                b.push_str(&format!(
                    "<tr class=\"{}\"><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    if ok { "ok" } else { "bad" },
                    r["flow"],
                    r["step"],
                    e(r["session"].as_str().unwrap_or("")),
                    e(r["label"].as_str().unwrap_or("")),
                    r["ms"],
                    e(&result)
                ));
            }
            b.push_str("</table>");
        }
        for (i, (ok, text)) in self.outputs.iter().enumerate() {
            b.push_str(&format!(
                "<details{}><summary>run_flow {} — {}</summary><pre>{}</pre></details>",
                if *ok { "" } else { " open" },
                i + 1,
                if *ok { "ok" } else { "failed" },
                e(text)
            ));
        }
        if !self.assertions.is_empty() {
            b.push_str("<h2>Assertions</h2><ul>");
            for a in &self.assertions {
                b.push_str(&format!("<li>{} [{}] {} {}</li>", if a.pass { "✓" } else { "✗" }, e(&a.session), e(&a.what), e(&a.detail)));
            }
            b.push_str("</ul>");
        }
        if !self.shots.is_empty() {
            b.push_str("<h2>Screenshots</h2><div class=\"shots\">");
            for (label, jpeg) in &self.shots {
                let data = base64::engine::general_purpose::STANDARD.encode(jpeg);
                b.push_str(&format!("<figure><img alt=\"{0}\" src=\"data:image/jpeg;base64,{data}\"><figcaption>{0}</figcaption></figure>", e(label)));
            }
            b.push_str("</div>");
        }
        for (session, rec) in &self.recordings {
            b.push_str(&format!("<h2>Recording [{}]</h2>{}", e(session), rec.embed(session)));
        }
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{} — {status}</title><style>
:root {{ --fg:#1d1d1f; --bg:#fff; --muted:#6e6e73; --line:#d0d7de; --pass:#1a7f37; --fail:#cf222e; }}
@media (prefers-color-scheme: dark) {{ :root {{ --fg:#e6edf3; --bg:#0d1117; --muted:#8b949e; --line:#30363d; }} }}
body {{ font: 14px/1.45 system-ui, sans-serif; color: var(--fg); background: var(--bg); max-width: 1100px; margin: 16px auto; padding: 0 16px; }}
.badge {{ font-size: 14px; text-transform: uppercase; }} .pass {{ color: var(--pass); }} .fail, .bad td:last-child {{ color: var(--fail); }}
.meta {{ color: var(--muted); }} .warn {{ color: #9a6700; font-weight: 600; }}
table {{ border-collapse: collapse; width: 100%; }} td, th {{ border-bottom: 1px solid var(--line); padding: 3px 6px; text-align: left; vertical-align: top; }}
pre {{ white-space: pre-wrap; word-break: break-word; }} .shots {{ display: flex; flex-wrap: wrap; gap: 12px; }}
figure {{ margin: 0; }} img {{ max-width: 320px; max-height: 640px; border: 1px solid var(--line); }}
</style></head><body>{b}</body></html>
",
            e(&self.test_id)
        )
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// UTC timestamp without a date crate: `20261103-142501` (compact) or `2026-11-03 14:25:01`.
fn utc_stamp(secs: u64, compact: bool) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    if compact {
        format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{s:02}")
    } else {
        format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc() {
        assert_eq!(utc_stamp(0, false), "1970-01-01 00:00:00");
        assert_eq!(utc_stamp(1_793_752_501, true), "20261104-003501");
    }

    #[test]
    fn hint_written_only_after_a_passing_run() {
        let dir = std::env::temp_dir().join(format!("fastpilot-mem-test-{}", std::process::id()));
        let mem = Memory::new(&dir, Mode::ReadWrite);
        let report = FlowReport { total: 0, elapsed: Default::default(), records: vec![], candidates: vec![], states: vec!["[w] screen \"Login\" (u)".into()] };

        let mut run = mem.begin_run("login-ok").expect("run");
        run.record_flow("- tapOn: Masuk\n".into(), &report, None).expect("record");
        run.record_assertion(Assertion { session: "w".into(), what: "visible Budi".into(), pass: false, detail: String::new() });
        run.finish(&mem, false, "").expect("finish");
        assert!(mem.hint("login-ok", None).is_none(), "failed run must not create a hint");

        let mut run = mem.begin_run("login-ok").expect("run");
        run.record_flow("- tapOn: Masuk\n".into(), &report, None).expect("record");
        let md = run.finish(&mem, true, "ok").expect("finish");
        let hint = mem.hint("login-ok", Some("[w] screen \"Login\" (u)")).expect("hint");
        assert!(hint.contains("passed 1x, failed 1x") && hint.contains("- tapOn: Masuk"), "{hint}");
        assert!(hint.contains("flaky: 1/2") && !hint.contains("may not apply"), "{hint}");
        let moved = mem.hint("login-ok", Some("screen \"X\" (other)")).expect("hint");
        assert!(moved.contains("now on other"), "{moved}");
        for _ in 0..3 {
            mem.begin_run("login-ok").expect("run").finish(&mem, false, "").expect("finish");
        }
        assert!(mem.hint("login-ok", None).expect("stale note").contains("stale"));
        assert!(md.report.ends_with("report.md") && md.report.exists());
        assert_eq!(md.class, None);
        // 1 pass + 4 failures: classes recorded, auto-quarantined (2 flips? no: f,p,f,f,f = 2 flips).
        let m = mem.load("login-ok").expect("memory");
        assert_eq!(m.classes, vec![Class::Product; 4]);
        let q = mem.quarantined();
        assert!(q.get("login-ok").is_some_and(|q| q.auto), "{q:?}");
        assert!(mem.hint("login-ok", None).is_some());
        let text = fs::read_to_string(&md.report).expect("report");
        assert!(text.contains("history: pass 1/2"), "{text}");
        let mut retry = mem.begin_run("login-ok").expect("run");
        retry.passed_on_retry();
        assert_eq!(retry.finish(&mem, true, "").expect("finish").class, Some(Class::Flaky));
        let mut retry = mem.begin_run("login-ok").expect("run");
        retry.passed_on_retry();
        assert_eq!(retry.finish(&mem, false, "").expect("finish").class, Some(Class::Product));
        mem.set_quarantine("login-ok", None).expect("release");
        assert!(mem.quarantined().is_empty());
        assert!(mem.begin_run("../evil").is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
