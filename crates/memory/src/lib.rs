//! Path memory + run evidence (PRD §12). Everything lives under `.fastpilot/` (or
//! `FASTPILOT_DIR`):
//! - `runs/<utc>-<test_id>/`: `steps.jsonl`, `assertions.json`, failure screenshots and
//!   snapshots, `report.md`.
//! - `memory/<test_id>.json`: the last passing flow as a *hint* (durable selectors, never refs).
//!   Written only after a run reported as passed; a failed run only updates statistics.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fastpilot_core::FlowReport;
use serde::{Deserialize, Serialize};

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
        let root = std::env::var_os("FASTPILOT_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".fastpilot"));
        let mode = match std::env::var("FASTPILOT_MEMORY").as_deref() {
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
        let notes = if notes.is_empty() { String::new() } else { format!("\n{}", notes.join("\n")) };
        Some(format!(
            "path memory for {test_id}: passed {}x, failed {}x, last pass {}, recorded from {} ({} ms).{notes}\n\
             Hint (send as one run_flow; if a step fails, explore from there):\n```yaml\n{}```",
            m.pass_count, m.fail_count, m.last_pass, m.start, m.duration_ms, m.flow
        ))
    }

    pub fn begin_run(&self, test_id: &str) -> Result<Run> {
        check_id(test_id)?;
        let id = format!("{}-{test_id}", utc_stamp(now_secs(), true));
        let dir = self.root.join("runs").join(&id);
        fs::create_dir_all(&dir).map_err(io(&dir))?;
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
}

impl Run {
    pub fn dir(&self) -> &Path {
        &self.dir
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
                "category": r.error.as_ref().map(|e| e.category()),
                "error": r.error.as_ref().map(|e| fastpilot_core::redact(&e.to_string())),
            });
            writeln!(f, "{line}").map_err(io(&path))?;
        }
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
                fs::write(&shot, jpeg).map_err(io(&shot))?;
            }
        }
        Ok(())
    }

    pub fn record_assertion(&mut self, a: Assertion) {
        if !a.pass {
            self.failures += 1;
        }
        self.assertions.push(a);
    }

    /// Close the run: `report.md`, `assertions.json`, and the path memory update.
    pub fn finish(self, memory: &Memory, passed: bool, notes: &str) -> Result<PathBuf> {
        let previous = memory.load(&self.test_id);
        let stamp = utc_stamp(now_secs(), false);
        let assertions = self.dir.join("assertions.json");
        let json = serde_json::to_string_pretty(&self.assertions).map_err(|source| Error::Json { path: assertions.clone(), source })?;
        fs::write(&assertions, json).map_err(io(&assertions))?;

        let status = if passed { "pass" } else { "fail" };
        let mut md = format!(
            "# {} — {status}\n\nrun: {}\nfinished: {stamp} UTC\nsteps: {}, failures: {}\n\n",
            self.test_id, self.id, self.steps, self.failures
        );
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
        if let Some(f) = previous.as_ref().and_then(PathMemory::flakiness) {
            md.push_str(&format!("History: {f} (before this run)\n\n"));
        }
        md.push_str("Evidence: `steps.jsonl`, `assertions.json`, `fail-*.txt|jpg`.\n");
        let report = self.dir.join("report.md");
        fs::write(&report, fastpilot_core::redact(&md)).map_err(io(&report))?;

        if memory.mode == Mode::ReadWrite {
            let path = memory.memory_path(&self.test_id);
            let mut m = memory.load(&self.test_id).unwrap_or_else(|| PathMemory { test_id: self.test_id.clone(), ..PathMemory::default() });
            // Only a fully passing run becomes the hint; a failed run never overwrites it.
            let clean = passed && self.failures == 0;
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
            m.history.push(clean);
            let extra = m.history.len().saturating_sub(20);
            m.history.drain(..extra);
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).map_err(io(dir))?;
            }
            let json = serde_json::to_string_pretty(&m).map_err(|source| Error::Json { path: path.clone(), source })?;
            fs::write(&path, json).map_err(io(&path))?;
        }
        Ok(report)
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
        assert!(md.ends_with("report.md") && md.exists());
        assert!(mem.begin_run("../evil").is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
