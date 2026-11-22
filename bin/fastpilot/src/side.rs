//! PF5 "Side by side": one test run on several platforms (`platform: web, android, ios`), compared
//! step by step. Rows are the test's numbered steps, columns the platforms, each cell the picture
//! after the step and Passed / Failed / Not run in words.
//!
//! Only two things count as a difference (FastProduct, 22 Nov 2026): a step that fails on some
//! platforms and passes on the others, and a check that found other text on some platforms
//! (letter case and spacing aside). The same text in another place is not compared (no layout
//! checks). A platform that could not run is a column that says why, never left out.
//!
//! Everything shown from a run (step words, found text, notes) is page or test text: data. The
//! HTML here escapes all of it and has no script.

use std::fmt::Write as _;
use std::path::PathBuf;

use fastpilot_core::Platform;

use crate::run::{CaseResult, StepResult};

/// What one platform did at one step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    Passed { found: Option<String> },
    Failed { found: Option<String>, why: String },
    /// The platform did not run (`next`: what to do), or an earlier step failed.
    NotRun { why: String, next: Option<String> },
    /// The platform ran, but reported nothing for this step on its own.
    NoResult { why: String },
}

impl Cell {
    fn ran(&self) -> Option<bool> {
        match self {
            Self::Passed { .. } => Some(true),
            Self::Failed { .. } => Some(false),
            _ => None,
        }
    }

    fn found(&self) -> Option<&str> {
        match self {
            Self::Passed { found } | Self::Failed { found, .. } => found.as_deref(),
            _ => None,
        }
    }
}

/// One platform's column.
#[derive(Debug, Clone)]
pub struct Column {
    pub platform: Platform,
    /// "Web · Chrome", "Android emulator", "iPhone simulator", "Android phone (physical)", ...
    pub name: String,
    pub ran: bool,
    pub passed: bool,
    pub cells: Vec<Cell>,
    pub shots: Vec<Option<PathBuf>>,
    /// It did not run because something broke before the test could start (the device session, the
    /// helper, a secret): "Could not run", not "Not run" (no device was there).
    pub could_not_run: bool,
}

/// What one platform's run gave, before it is laid out as a column.
#[derive(Debug, Clone)]
pub enum Outcome {
    NotRun { why: String, next: Option<String> },
    Ran { passed: bool, notes: String, steps: Vec<StepResult> },
}

/// A row: one numbered step, and the columns that differ there with what the person should read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub n: usize,
    pub label: String,
    /// Indexes of the columns that differ at this step.
    pub differs: Vec<usize>,
    /// The found text most platforms show, when the found text differs.
    pub expected: Option<String>,
    /// "Step 2 · iPhone simulator shows "Log in", the others show "Sign in"."
    pub message: Option<String>,
    /// Every platform that ran it failed it (no difference: they behave the same).
    pub failed_everywhere: bool,
}

#[derive(Debug, Clone)]
pub struct Comparison {
    pub id: String,
    pub title: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
}

/// "web" / "Android" / "iPhone": a platform in a short label ("Differs on iPhone").
fn short(p: Platform) -> &'static str {
    match p {
        Platform::Web => "web",
        Platform::Android => "Android",
        Platform::Ios => "iPhone",
    }
}

/// "a", "a and b", "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Found texts compare without letter case and extra spaces (an Android button may show its
/// label in capitals).
fn norm(t: &str) -> String {
    t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The column of one platform: a cell per step.
pub fn column(platform: Platform, name: &str, steps: usize, outcome: &Outcome) -> Column {
    let (ran, passed, cells, shots) = match outcome {
        Outcome::NotRun { why, next } => (false, false, vec![Cell::NotRun { why: why.clone(), next: next.clone() }; steps], vec![None; steps]),
        Outcome::Ran { passed, notes, steps: reported } => {
            let mut cells = Vec::with_capacity(steps);
            let mut shots = Vec::with_capacity(steps);
            let mut stopped = false;
            for n in 1..=steps {
                let got = reported.iter().find(|r| r.step == n);
                shots.push(got.and_then(|r| r.shot.clone()));
                let cell = match got {
                    Some(r) if r.passed => Cell::Passed { found: r.found.clone() },
                    Some(r) => {
                        stopped = true;
                        let why = if r.notes.is_empty() { notes.clone() } else { r.notes.clone() };
                        Cell::Failed { found: r.found.clone(), why }
                    }
                    None if stopped => Cell::NotRun { why: "an earlier step failed".into(), next: None },
                    None if reported.is_empty() => Cell::NoResult { why: "this run did not report its steps one by one".into() },
                    // It stopped here: the steps are reported in order, and the test failed after the
                    // last one it reported.
                    None if !passed => {
                        stopped = true;
                        Cell::Failed { found: None, why: notes.clone() }
                    }
                    None => Cell::NoResult { why: "the test passed; this step was not reported on its own".into() },
                };
                cells.push(cell);
            }
            (true, *passed, cells, shots)
        }
    };
    Column { platform, name: name.to_owned(), ran, passed, cells, shots, could_not_run: false }
}

/// Compare the columns row by row: which platforms differ at each step, and why, in words.
pub fn compare(id: &str, title: &str, steps: &[String], columns: Vec<Column>) -> Comparison {
    let mut rows = Vec::with_capacity(steps.len());
    for (i, label) in steps.iter().enumerate() {
        let n = i + 1;
        let ran: Vec<(usize, bool)> = columns.iter().enumerate().filter_map(|(c, col)| col.cells.get(i).and_then(Cell::ran).map(|ok| (c, ok))).collect();
        let pass: Vec<usize> = ran.iter().filter(|x| x.1).map(|x| x.0).collect();
        let fail: Vec<usize> = ran.iter().filter(|x| !x.1).map(|x| x.0).collect();
        let name = |c: usize| columns[c].name.clone();
        let mut differs: Vec<usize> = Vec::new();
        let mut messages: Vec<String> = Vec::new();
        let failed_everywhere = ran.len() >= 2 && pass.is_empty();
        if !pass.is_empty() && !fail.is_empty() {
            // The odd one out differs; with as many failing as passing, the failing ones.
            let odd = if pass.len() < fail.len() { &pass } else { &fail };
            differs.extend(odd);
            let why = fail.iter().find_map(|c| match &columns[*c].cells[i] {
                Cell::Failed { why, .. } if !why.is_empty() => Some(why.clone()),
                _ => None,
            });
            messages.push(format!(
                "fails on {}, passes on {}{}",
                and_list(&fail.iter().map(|c| name(*c)).collect::<Vec<_>>()),
                and_list(&pass.iter().map(|c| name(*c)).collect::<Vec<_>>()),
                why.map(|w| format!(": {w}")).unwrap_or_default()
            ));
        }
        // Found text: the platforms that found something at this step.
        let found: Vec<(usize, &str)> = columns.iter().enumerate().filter_map(|(c, col)| col.cells.get(i).and_then(Cell::found).map(|f| (c, f))).collect();
        let mut expected = None;
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (c, f) in &found {
            match groups.iter_mut().find(|(k, _)| *k == norm(f)) {
                Some(g) => g.1.push(*c),
                None => groups.push((norm(f), vec![*c])),
            }
        }
        if groups.len() > 1 {
            groups.sort_by_key(|g| std::cmp::Reverse(g.1.len()));
            let text_of = |c: usize| found.iter().find(|(x, _)| *x == c).map(|(_, f)| (*f).to_owned()).unwrap_or_default();
            if groups[0].1.len() > groups[1].1.len() {
                let major = text_of(groups[0].1[0]);
                let odd: Vec<usize> = groups[1..].iter().flat_map(|g| g.1.clone()).collect();
                let each: Vec<String> = odd.iter().map(|c| format!("{} shows \"{}\"", name(*c), text_of(*c))).collect();
                let others = if groups[0].1.len() == 1 { name(groups[0].1[0]) } else { "the others".to_owned() };
                messages.push(format!("{}, {others} {} \"{major}\"", and_list(&each), if groups[0].1.len() == 1 { "shows" } else { "show" }));
                expected = Some(major);
                differs.extend(odd);
            } else {
                let each: Vec<String> = found.iter().map(|(c, f)| format!("{} \"{f}\"", name(*c))).collect();
                messages.push(format!("the platforms show different text: {}", and_list(&each)));
                differs.extend(found.iter().map(|(c, _)| *c));
            }
        }
        differs.sort_unstable();
        differs.dedup();
        let message = (!messages.is_empty()).then(|| format!("Step {n} · {}.", messages.join("; ")));
        rows.push(Row { n, label: label.clone(), differs, expected, message, failed_everywhere });
    }
    Comparison { id: id.to_owned(), title: title.to_owned(), columns, rows }
}

/// The side-by-side tests among `results` (one per test that ran once per platform), in order.
pub fn of_results(results: &[CaseResult]) -> Vec<Comparison> {
    let mut groups: Vec<&str> = Vec::new();
    for r in results {
        if let Some(s) = &r.case.side
            && !groups.contains(&s.group.as_str())
        {
            groups.push(&s.group);
        }
    }
    groups
        .into_iter()
        .filter_map(|g| {
            let mine: Vec<&CaseResult> = results.iter().filter(|r| r.case.side.as_ref().is_some_and(|s| s.group == g)).collect();
            let side = mine.first()?.case.side.clone()?;
            let columns = side
                .platforms
                .iter()
                .map(|p| {
                    let r = mine.iter().find(|r| r.case.platforms().first() == Some(p));
                    let outcome = match r {
                        Some(r) if !r.not_run() => Outcome::Ran { passed: r.passed, notes: r.notes.clone(), steps: r.steps.clone() },
                        Some(r) => {
                            let next = r.notes.split_once(". ").map(|(_, n)| n.trim().to_owned()).filter(|n| !n.is_empty());
                            Outcome::NotRun { why: crate::run::why_not_run(&r.notes), next }
                        }
                        None => Outcome::NotRun { why: crate::run::why_not_run(crate::run::no_device(*p)), next: Some(crate::run::set_up(*p).to_owned()) },
                    };
                    let name = crate::run::device_name(*p, r.and_then(|r| r.devices.first()).map(String::as_str));
                    let mut col = column(*p, name, side.steps.len(), &outcome);
                    col.could_not_run = r.is_some_and(|r| r.not_run() && r.notes.trim_start().starts_with("Could not run"));
                    col
                })
                .collect();
            Some(compare(g, &mine[0].case.title, &side.steps, columns))
        })
        .collect()
}

impl Comparison {
    fn differing(&self) -> Vec<usize> {
        let mut d: Vec<usize> = self.rows.iter().flat_map(|r| r.differs.clone()).collect();
        d.sort_unstable();
        d.dedup();
        d
    }

    /// "2 of 3 platforms match" and "1 platform differs · 5 steps", or "2 of 3 platforms ran".
    pub fn summary(&self) -> (String, String) {
        let total = self.columns.len();
        let ran = self.columns.iter().filter(|c| c.ran).count();
        let differ = self.differing().len();
        let steps = format!("{} {}", self.rows.len(), if self.rows.len() == 1 { "step" } else { "steps" });
        let passed = self.columns.iter().filter(|c| c.passed).count();
        let head = if ran < total {
            format!("{ran} of {total} platforms ran")
        } else if differ > 0 {
            format!("{} of {total} platforms match", total - differ)
        } else if passed == total {
            format!("All {total} platforms match")
        } else if passed == 0 {
            format!("Failed on all {total} platforms")
        } else {
            // Failures without a step of their own to compare: no "match" is claimed.
            format!("{passed} of {total} platforms passed")
        };
        // A platform that failed a step the others passed "fails"; one that found other text "differs".
        let fails = self.differing().into_iter().filter(|c| self.failed_at(*c).is_some()).count();
        let mut parts = Vec::new();
        match fails {
            0 => {}
            1 => parts.push("1 platform fails".to_owned()),
            f => parts.push(format!("{f} platforms fail")),
        }
        match differ - fails {
            0 => {}
            1 => parts.push("1 platform differs".to_owned()),
            d => parts.push(format!("{d} platforms differ")),
        }
        parts.push(steps);
        let sub = parts.join(" · ");
        (head, sub)
    }

    /// The step number where column `c` failed, if it did.
    fn failed_at(&self, c: usize) -> Option<usize> {
        self.columns[c].cells.iter().position(|x| matches!(x, Cell::Failed { .. })).map(|i| i + 1)
    }

    /// "Not run" (no device there) or "Could not run" (something broke before the test started).
    fn not_run_word(col: &Column) -> &'static str {
        if col.could_not_run { "Could not run" } else { "Not run" }
    }

    /// The short state of one column for the summary: "All 5 steps", "Failed at step 3",
    /// "Differs at step 2", ... A failed step is a failure, never only a difference (QA, L061-5).
    pub fn state(&self, c: usize) -> (&'static str, String) {
        let col = &self.columns[c];
        if !col.ran {
            return ("none", Self::not_run_word(col).into());
        }
        if let Some(n) = self.failed_at(c) {
            return ("bad", format!("Failed at step {n}"));
        }
        if let Some(r) = self.rows.iter().find(|r| r.differs.contains(&c)) {
            return ("diff", format!("Differs at step {}", r.n));
        }
        if !col.passed {
            return ("bad", "Failed".into());
        }
        if col.cells.iter().all(|x| matches!(x, Cell::Passed { .. })) {
            ("ok", format!("All {} steps", self.rows.len()))
        } else {
            ("ok", "Passed".into())
        }
    }

    /// Plain-words lines for the terminal after the run.
    pub fn terminal(&self) -> String {
        let (head, sub) = self.summary();
        let mut out = format!("Side by side · {} · {head} ({sub})", self.title);
        for (c, col) in self.columns.iter().enumerate() {
            let (_, state) = self.state(c);
            let why = match col.cells.first() {
                Some(Cell::NotRun { why, .. }) if !col.ran => format!(": {why}"),
                _ => String::new(),
            };
            let _ = write!(out, "\n  {}: {state}{why}", col.name);
        }
        for r in self.rows.iter().filter_map(|r| r.message.as_ref()) {
            let _ = write!(out, "\n  {r}");
        }
        out
    }

    /// The "Side by side" section of the suite report: a real table with a caption, a column
    /// header per platform and a row header per step; every cell in words, every picture with alt
    /// text. All text is escaped; no script.
    pub fn html(&self) -> String {
        let e = esc;
        let names: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let (head, sub) = self.summary();
        let mut b = String::new();
        let _ = write!(b, "<section class=\"sbs\" id=\"side-{}\" aria-label=\"{} · side by side\"><h2>{} <span class=\"meta\">· side by side</span></h2>", e(&self.id), e(&self.title), e(&self.title));
        let _ = write!(b, "<div class=\"sbs-sum\"><p><b>{}</b><br><span class=\"meta\">{}</span></p><ul>", e(&head), e(&sub));
        for (c, col) in self.columns.iter().enumerate() {
            let (cls, state) = self.state(c);
            let mark = match cls {
                "ok" => "✓ ",
                "bad" => "✗ ",
                _ => "",
            };
            let _ = write!(b, "<li><b>{}</b> <span class=\"chip {cls}\">{mark}{}</span></li>", e(&col.name), e(&state));
        }
        b.push_str("</ul></div>");
        // The states of the design: fails everywhere, all match, not every platform listed.
        let ran: Vec<&Column> = self.columns.iter().filter(|c| c.ran).collect();
        for r in self.rows.iter().filter(|r| r.failed_everywhere) {
            let _ = write!(
                b,
                "<p class=\"note bad\" role=\"alert\"><b>Step {} failed on all {} platforms.</b> This is probably a problem in your app, not in one platform. No differences are highlighted because they all behave the same.</p>",
                r.n,
                ran.len()
            );
        }
        if self.differing().is_empty() && ran.len() == self.columns.len() && ran.iter().all(|c| c.passed) {
            let _ = write!(
                b,
                "<p class=\"note ok\" role=\"status\"><b>All {} platforms match.</b> {} steps passed on {}.</p>",
                self.columns.len(),
                self.rows.len(),
                e(&and_list(&names))
            );
        }
        let listed: Vec<Platform> = self.columns.iter().map(|c| c.platform).collect();
        let missing: Vec<&str> = [Platform::Web, Platform::Android, Platform::Ios].into_iter().filter(|p| !listed.contains(p)).map(short).collect();
        if !missing.is_empty() {
            let have: Vec<String> = listed.iter().map(|p| short(*p).to_owned()).collect();
            let _ = write!(
                b,
                "<p class=\"note info\"><b>This test only lists {}.</b> To compare on {} too, add it to the test's <code>platform:</code> line or ask in Chat: \"run the {} test on all three\".</p>",
                e(&and_list(&have)),
                e(&missing.join(" and ")),
                e(&self.title.to_lowercase())
            );
        }
        // Security review, Low 4: at most 40 MB of pictures in one report.
        let mut left = MAX_REPORT_PICTURES;
        let _ = write!(b, "<table class=\"sbs-t\"><caption>Each step of the test on {}</caption><thead><tr><th scope=\"col\">Step</th>", e(&and_list(&names)));
        for col in &self.columns {
            let _ = write!(b, "<th scope=\"col\">{}</th>", e(&col.name));
        }
        b.push_str("</tr></thead><tbody>");
        for (i, row) in self.rows.iter().enumerate() {
            let failed_here = |c: &usize| matches!(self.columns[*c].cells[i], Cell::Failed { .. });
            let on = |pick: &dyn Fn(&usize) -> bool| -> Vec<String> { row.differs.iter().filter(|c| pick(c)).map(|c| short(self.columns[*c].platform).to_owned()).collect() };
            let (fails, other) = (on(&failed_here), on(&|c| !failed_here(c)));
            let mut label = String::new();
            if !fails.is_empty() {
                let _ = write!(label, "<span class=\"lbl\">Fails on {}</span>", e(&and_list(&fails)));
            }
            if !other.is_empty() {
                let _ = write!(label, "<span class=\"lbl\">Differs on {}</span>", e(&and_list(&other)));
            }
            let _ = write!(b, "<tr{}><th scope=\"row\">{} · {}{label}</th>", if row.differs.is_empty() { "" } else { " class=\"differs\"" }, row.n, e(&row.label));
            for (c, col) in self.columns.iter().enumerate() {
                let cell = &col.cells[i];
                let differs = row.differs.contains(&c);
                let (cls, words) = match cell {
                    Cell::Passed { found } => {
                        let f = match (found, &row.expected) {
                            (Some(f), Some(x)) if differs && norm(f) != norm(x) => format!(" · Found \"{f}\" instead of \"{x}\""),
                            (Some(f), _) => format!(" · Found \"{f}\""),
                            _ => String::new(),
                        };
                        (if differs { "diff" } else { "ok" }, format!("<span aria-hidden=\"true\">✓</span> <b>Passed</b>{}", e(&f)))
                    }
                    Cell::Failed { found, why } => {
                        let f = found.as_ref().map(|f| format!(" · found \"{f}\"")).unwrap_or_default();
                        let why = if why.is_empty() { String::new() } else { format!(" · {why}") };
                        (if differs { "bad diff" } else { "bad" }, format!("<span aria-hidden=\"true\">✗</span> <b>Failed</b>{}", e(&format!("{why}{f}"))))
                    }
                    Cell::NotRun { why, next } => {
                        let next = next.as_ref().map(|n| format!("<br><span class=\"next\">{}</span>", e(n))).unwrap_or_default();
                        let word = if col.ran { "Not run" } else { Self::not_run_word(col) };
                        ("none", format!("<b>{word}</b><br>{}{next}", e(why)))
                    }
                    Cell::NoResult { why } => ("none", format!("<b>No step result</b><br>{}", e(why))),
                };
                let state = match cell {
                    Cell::Passed { .. } => "passed",
                    Cell::Failed { .. } => "failed",
                    Cell::NotRun { .. } => "not run",
                    Cell::NoResult { .. } => "no step result",
                };
                let alt = format!("{}, step {}: {state}{}", col.name, row.n, cell.found().map(|f| format!(", \"{f}\" found")).unwrap_or_default());
                let img = match col.shots[i].as_ref().map(|p| picture(p, &mut left)) {
                    Some(Picture::Data(data)) => format!("<img alt=\"{}\" src=\"data:image/jpeg;base64,{data}\">", e(&alt)),
                    Some(Picture::Cut) => "<p class=\"meta\">No picture here: the report is at its size limit.</p>".to_owned(),
                    Some(Picture::None) | None => String::new(),
                };
                let _ = write!(b, "<td class=\"cell {cls}\">{img}<p>{words}</p></td>");
            }
            b.push_str("</tr>");
        }
        b.push_str("</tbody></table>");
        let messages: Vec<&String> = self.rows.iter().filter_map(|r| r.message.as_ref()).collect();
        b.push_str("<div class=\"where\">");
        if !messages.is_empty() {
            b.push_str("<h3>Where they differ</h3><ul>");
            for m in messages {
                let _ = write!(b, "<li>{}</li>", e(m));
            }
            b.push_str("</ul>");
        }
        b.push_str(
            "<p class=\"meta\">Differences are marked when a step fails on one platform and passes on the others, or when the text a \
             check found is not the same on every platform (letter case and spacing aside). Step results are reported by the AI that \
             ran the test; the picture next to each one is what the screen showed. Pictures show the app as it looked (names and \
             numbers on the screen are not masked) and are scaled to the same width.</p></div></section>",
        );
        b
    }
}

/// Pictures in one report, in bytes before encoding (Security review, Low 4).
const MAX_REPORT_PICTURES: u64 = 40 * 1024 * 1024;

enum Picture {
    Data(String),
    /// Left out: the report reached its size limit.
    Cut,
    None,
}

/// The picture of a cell, inlined (the report is one file); at most 4 MB each, and at most `left`
/// bytes in the whole report (counted down).
fn picture(p: &std::path::Path, left: &mut u64) -> Picture {
    use base64::Engine;
    let Ok(meta) = std::fs::metadata(p) else { return Picture::None };
    if meta.len() > 4 * 1024 * 1024 {
        return Picture::None;
    }
    if meta.len() > *left {
        return Picture::Cut;
    }
    match std::fs::read(p) {
        Ok(bytes) => {
            *left -= meta.len();
            Picture::Data(base64::engine::general_purpose::STANDARD.encode(bytes))
        }
        Err(_) => Picture::None,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// The report's styles for the side-by-side section.
pub const CSS: &str = "
.sbs { border: 1px solid var(--line); border-radius: 8px; margin: 14px 0; padding: 10px 14px; }
.sbs-sum { display: flex; flex-wrap: wrap; gap: 8px 32px; align-items: flex-start; } .sbs-sum ul { display: flex; flex-wrap: wrap; gap: 8px 28px; list-style: none; padding: 0; margin: 0; }
.sbs-sum li { display: flex; flex-direction: column; gap: 2px; }
.chip { display: inline-block; font-size: 12px; padding: 1px 8px; border-radius: 6px; } .chip.ok { background: #e3f3e6; color: #135c27; } .chip.bad { background: #fde8e8; color: #8e2323; }
.chip.diff { background: #f8ead6; color: #6f4207; } .chip.none { background: #eceef1; color: #3d434b; }
.note { border-radius: 6px; padding: 8px 12px; } .note.bad { background: #fde8e8; color: #8e2323; } .note.ok { background: #e3f3e6; color: #135c27; } .note.info { background: #e6edfb; color: #1d3f8f; }
table.sbs-t { border-collapse: separate; border-spacing: 8px; width: 100%; table-layout: fixed; } .sbs-t caption { text-align: left; color: var(--muted); }
.sbs-t th, .sbs-t td { text-align: left; vertical-align: top; border: 0; } .sbs-t th[scope=row] { width: 22%; font-weight: 600; }
.sbs-t tr.differs th[scope=row] { background: #fdf6ea; } .lbl { display: block; width: max-content; margin-top: 4px; font-size: 12px; font-weight: 500; background: #f8ead6; color: #6f4207; padding: 1px 8px; border-radius: 6px; }
.cell { border: 1px solid var(--line) !important; border-radius: 8px; padding: 6px; } .cell p { margin: 4px 2px; }
.cell img { display: block; width: 100%; height: 220px; object-fit: contain; max-width: none; background: #f4f5f7; border: 0; }
.cell.ok b { color: var(--pass); } .cell.diff { border: 2px solid #b26a00 !important; } .cell.diff b { color: #6f4207; }
.cell.bad { border: 2px solid #8e2323 !important; background: #fde8e8; } .cell.bad b { color: #8e2323; } .cell.none { border: 1px dashed #8a9099 !important; color: #3d434b; }
.next { font-size: 12px; } .where h3 { margin: 8px 0 4px; font-size: 14px; }
";

#[cfg(test)]
mod tests {
    use super::*;

    fn step(n: usize, passed: bool, found: Option<&str>) -> StepResult {
        StepResult { step: n, passed, found: found.map(str::to_owned), notes: String::new(), shot: None }
    }

    fn ran(passed: bool, steps: Vec<StepResult>) -> Outcome {
        Outcome::Ran { passed, notes: if passed { "ok".into() } else { "field not found".into() }, steps }
    }

    fn steps() -> Vec<String> {
        ["Open the app", "Check the link for a forgotten password", "Sign in", "Check the name Budi"].map(str::to_owned).to_vec()
    }

    fn three(web: Outcome, android: Outcome, ios: Outcome) -> Comparison {
        let cols = vec![
            column(Platform::Web, "Web · Chrome", 4, &web),
            column(Platform::Android, "Android emulator", 4, &android),
            column(Platform::Ios, "iPhone simulator", 4, &ios),
        ];
        compare("login", "Login", &steps(), cols)
    }

    fn all_pass(link: &str) -> Outcome {
        ran(true, vec![step(1, true, None), step(2, true, Some(link)), step(3, true, Some("Masuk")), step(4, true, Some("Selamat datang, Budi"))])
    }

    /// PF5 "Done when": the demo login on web, Android and iPhone gives three columns, and the
    /// planted iOS-only label ("Lupa kata sandi?" where the others say "Lupa password?") is the one
    /// difference: on step 2, on iPhone, named in the row header and in words.
    #[test]
    fn three_columns_and_the_planted_ios_label_is_highlighted() {
        let c = three(all_pass("Lupa password?"), all_pass("LUPA PASSWORD?"), all_pass("Lupa kata sandi?"));
        assert_eq!(c.columns.len(), 3);
        assert_eq!(c.rows.iter().map(|r| r.differs.clone()).collect::<Vec<_>>(), vec![vec![], vec![2], vec![], vec![]], "letter case alone is no difference");
        assert_eq!(c.rows[1].message.as_deref(), Some("Step 2 · iPhone simulator shows \"Lupa kata sandi?\", the others show \"Lupa password?\"."));
        assert_eq!(c.summary(), ("2 of 3 platforms match".to_owned(), "1 platform differs · 4 steps".to_owned()));
        assert_eq!(c.state(0), ("ok", "All 4 steps".into()));
        assert_eq!(c.state(2), ("diff", "Differs at step 2".into()));
        let html = c.html();
        assert!(html.contains("<caption>Each step of the test on Web · Chrome, Android emulator and iPhone simulator</caption>"), "{html}");
        assert_eq!(html.matches("<th scope=\"col\">").count(), 4, "Step + one per platform");
        assert_eq!(html.matches("<th scope=\"row\">").count(), 4, "one per step");
        assert!(html.contains("<tr class=\"differs\"><th scope=\"row\">2 · Check the link for a forgotten password<span class=\"lbl\">Differs on iPhone</span></th>"), "{html}");
        assert!(html.contains("<td class=\"cell diff\"><p><span aria-hidden=\"true\">✓</span> <b>Passed</b> · Found &quot;Lupa kata sandi?&quot; instead of &quot;Lupa password?&quot;</p></td>"), "{html}");
        assert!(html.contains("<h3>Where they differ</h3>"));
        let t = c.terminal();
        assert!(t.contains("2 of 3 platforms match") && t.contains("iPhone simulator: Differs at step 2") && t.contains("shows \"Lupa kata sandi?\""), "{t}");
        // The same text everywhere: nothing is marked, and the report says they all match.
        let same = three(all_pass("Lupa password?"), all_pass("Lupa password?"), all_pass("Lupa password?"));
        assert!(same.rows.iter().all(|r| r.differs.is_empty() && r.message.is_none()));
        assert_eq!(same.summary().0, "All 3 platforms match");
        assert!(same.html().contains("<b>All 3 platforms match.</b> 4 steps passed on Web · Chrome, Android emulator and iPhone simulator."));
    }

    /// A platform that could not run is a column that says why and what to do; never left out.
    #[test]
    fn a_missing_platform_says_not_run() {
        let missing = Outcome::NotRun { why: "No iPhone simulator".into(), next: Some("Boot an iPhone simulator, then run again.".into()) };
        let c = three(all_pass("Lupa password?"), all_pass("Lupa password?"), missing);
        assert!(c.columns[2].cells.iter().all(|x| matches!(x, Cell::NotRun { why, .. } if why == "No iPhone simulator")));
        assert!(c.rows.iter().all(|r| r.differs.is_empty()), "not running is not a difference");
        assert_eq!(c.summary().0, "2 of 3 platforms ran");
        assert_eq!(c.state(2), ("none", "Not run".into()));
        let html = c.html();
        assert_eq!(html.matches("<td class=\"cell none\"><p><b>Not run</b><br>No iPhone simulator<br><span class=\"next\">Boot an iPhone simulator, then run again.</span></p></td>").count(), 4, "{html}");
        assert!(c.terminal().contains("iPhone simulator: Not run: No iPhone simulator"), "{}", c.terminal());
    }

    /// Fails on one platform only: that cell is red with "Failed", the later steps there are "Not
    /// run: an earlier step failed", and the row differs on that platform. Fails everywhere: no
    /// difference, and the report says it is probably the app.
    #[test]
    fn fails_on_one_and_fails_everywhere() {
        let broken = ran(false, vec![step(1, true, None), step(2, true, Some("Lupa password?")), StepResult { notes: "field not found".into(), ..step(3, false, None) }]);
        let c = three(all_pass("Lupa password?"), all_pass("Lupa password?"), broken);
        assert_eq!(c.rows[2].differs, vec![2]);
        assert_eq!(c.rows[2].message.as_deref(), Some("Step 3 · fails on iPhone simulator, passes on Web · Chrome and Android emulator: field not found."));
        assert_eq!(c.columns[2].cells[3], Cell::NotRun { why: "an earlier step failed".into(), next: None });
        assert!(c.html().contains("<td class=\"cell bad diff\"><p><span aria-hidden=\"true\">✗</span> <b>Failed</b> · field not found</p></td>"));
        let fail = |found| ran(false, vec![step(1, true, None), step(2, true, Some(found)), step(3, true, None), step(4, false, None)]);
        let c = three(fail("Lupa password?"), fail("Lupa password?"), fail("Lupa password?"));
        assert!(c.rows.iter().all(|r| r.differs.is_empty()) && c.rows[3].failed_everywhere);
        assert!(c.html().contains("<b>Step 4 failed on all 3 platforms.</b> This is probably a problem in your app"));
        // A run that reported nothing one by one says so instead of guessing which step failed.
        let silent = column(Platform::Web, "Web · Chrome", 2, &Outcome::Ran { passed: false, notes: "x".into(), steps: vec![] });
        assert!(silent.cells.iter().all(|x| matches!(x, Cell::NoResult { .. })));
        // Seen on real devices: two platforms could not start a session (failed, no step results).
        // Nothing is compared there, so the summary claims no match.
        let c = three(all_pass("Lupa password?"), ran(false, vec![]), ran(false, vec![]));
        assert_eq!(c.summary().0, "1 of 3 platforms passed");
        assert_eq!((c.state(1), c.state(2)), (("bad", "Failed".into()), ("bad", "Failed".into())));
        let c = three(ran(false, vec![]), ran(false, vec![]), ran(false, vec![]));
        assert_eq!(c.summary().0, "Failed on all 3 platforms");
    }

    /// L061-5 (QA, v0.6.0 Low 5): a platform whose test failed (its app did not launch at the first
    /// step) is "Failed at step 1", and its row says "Fails on Android", never "Differs". A platform
    /// that could not start (infrastructure) says "Could not run", not "Not run".
    #[test]
    fn a_failed_platform_says_failed_not_differs() {
        let launch = ran(false, vec![StepResult { notes: "the app did not launch".into(), ..step(1, false, None) }]);
        let c = compare(
            "login",
            "Login",
            &steps(),
            vec![column(Platform::Web, "Web · Chrome", 4, &all_pass("Lupa password?")), column(Platform::Android, "Android emulator", 4, &launch)],
        );
        assert_eq!(c.state(1), ("bad", "Failed at step 1".into()));
        assert_eq!(c.summary().1, "1 platform fails · 4 steps");
        let html = c.html();
        assert!(html.contains("<span class=\"lbl\">Fails on Android</span>") && !html.contains("Differs on") && !html.contains("Differs at"), "{html}");
        let t = c.terminal();
        assert!(t.contains("Android emulator: Failed at step 1") && !t.contains("Differs"), "{t}");
        // Infrastructure: the column was not run because the session did not start.
        let mut cols = vec![
            column(Platform::Web, "Web · Chrome", 4, &all_pass("Lupa password?")),
            column(Platform::Ios, "iPhone simulator", 4, &Outcome::NotRun { why: "The iPhone simulator session did not start".into(), next: None }),
        ];
        cols[1].could_not_run = true;
        let c = compare("login", "Login", &steps(), cols);
        assert_eq!(c.state(1), ("none", "Could not run".into()));
        assert!(c.terminal().contains("iPhone simulator: Could not run: The iPhone simulator session did not start"), "{}", c.terminal());
        assert_eq!(c.html().matches("<b>Could not run</b><br>The iPhone simulator session did not start").count(), 4);
    }

    /// Page text is data: a label or found text with markup is shown as text, never as HTML.
    #[test]
    fn page_text_is_escaped() {
        let evil = "<img src=x onerror=alert(1)>\"'";
        let c = compare(
            "t",
            "<script>alert(1)</script>",
            &[format!("Tap {evil}")],
            vec![
                column(Platform::Web, "Web · Chrome", 1, &ran(true, vec![step(1, true, Some(evil))])),
                column(Platform::Ios, "iPhone simulator", 1, &ran(false, vec![StepResult { notes: evil.into(), ..step(1, false, Some("ok")) }])),
            ],
        );
        let html = c.html();
        assert!(!html.contains("<script") && !html.contains("<img src=x") && !html.contains("onerror=alert(1)>"), "{html}");
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;&quot;&#39;"), "{html}");
        // Two platforms with different text and no majority: both are named.
        assert_eq!(c.rows[0].differs, vec![0, 1]);
        assert!(c.html().contains("This test only lists web and iPhone."), "{}", c.html());
    }

    #[test]
    fn the_report_has_a_size_limit_for_pictures() {
        let dir = std::env::temp_dir().join(format!("fp-side-pics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let p = dir.join("a.jpg");
        std::fs::write(&p, vec![0xffu8; 1000]).expect("write");
        let mut left = 2500u64;
        let kinds: Vec<&str> = (0..3).map(|_| match picture(&p, &mut left) { Picture::Data(_) => "data", Picture::Cut => "cut", Picture::None => "none" }).collect();
        assert_eq!(kinds, ["data", "data", "cut"]);
        assert!(matches!(picture(&dir.join("missing.jpg"), &mut left), Picture::None));
        let _ = std::fs::remove_dir_all(&dir);
    }

}
