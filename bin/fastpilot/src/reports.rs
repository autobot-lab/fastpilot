//! E9 (v0.7.2): a shareable report of a run, made by fastpilot itself (it works the same with
//! Claude Code or any provider): a web page that opens offline (`report.html`, screenshots inlined)
//! and a Markdown file (`report.md`, screenshots next to it), saved with the run in
//! `.fastpilot/history/reports/<run>/`. A new report replaces the old one of that run. Nothing is
//! uploaded. Passwords and card numbers in the text show as `***`; the screenshots are the run's own
//! (the page as it was), which the report says.
//!
//! What goes in: every screenshot of a failed test, and the last one of each passing test.

use std::path::{Path, PathBuf};

use crate::control::{Fail, Surface, fail, q};
use fastpilot_memory::history::{History, Item};

pub const NOTE: &str = "Made on this Mac. Nothing was uploaded. Screenshots show the page as it was. Look at them before you share.";
const SHOTS_LINE: &str = "Screenshots show the page as it was. Look at them before you share.";

/// A report that was made: its result line (no ✓), files and screenshot count.
pub struct Made {
    pub id: String,
    pub line: String,
    pub html: PathBuf,
    pub md: PathBuf,
}

/// One test of the run, as the report shows it.
struct Test {
    name: String,
    /// `Some(true)` passed, `Some(false)` failed, `None` did not run.
    passed: Option<bool>,
    /// The run's own report.md (already masked), for a failed test's steps.
    steps: String,
    shots: Vec<(String, Vec<u8>)>,
}

fn when(ms: u64) -> String {
    format!("{} {}", fastpilot_core::claim::local_date(ms), fastpilot_core::claim::local_hm(ms))
}

/// "1 of 3 failed", "passed", "2 of 2 could not run".
fn result_of(r: &Item) -> String {
    if r.failed > 0 {
        format!("{} of {} failed", r.failed, r.tests)
    } else if r.not_run > 0 && r.passed == 0 {
        format!("{} of {} could not run", r.not_run, r.tests)
    } else {
        "passed".into()
    }
}

fn shots_word(n: usize) -> String {
    format!("{n} screenshot{}", if n == 1 { "" } else { "s" })
}

fn not_found(id: &str, s: Surface) -> Fail {
    match s {
        Surface::Cli => fail("run_not_found", format!("No run with id {}. Run fastpilot report --list to see recent runs.", q(id))),
        Surface::Mcp => fail("run_not_found", format!("No run with id {}. Use reports list to see recent runs.", q(id))),
    }
}

fn failed_to_read(s: Surface) -> Fail {
    match s {
        Surface::Cli => fail("report_failed", "Couldn’t make the report: the screenshots of this run could not be read. The run itself is kept. Run the command again."),
        Surface::Mcp => fail("report_failed", "Couldn’t make the report: the screenshots of this run could not be read. The run itself is kept. Say “make the report again”."),
    }
}

/// Card numbers in text (13 to 19 digits, spaces or dashes between them) become `***`, as
/// passwords do through `redact`.
pub fn mask(text: &str) -> String {
    let text = fastpilot_core::redact(text);
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let (mut j, mut digits, mut end) = (i, 0, i);
            while j < chars.len() && (chars[j].is_ascii_digit() || ((chars[j] == ' ' || chars[j] == '-') && j + 1 < chars.len() && chars[j + 1].is_ascii_digit())) {
                if chars[j].is_ascii_digit() {
                    digits += 1;
                    end = j + 1;
                }
                j += 1;
            }
            if (13..=19).contains(&digits) && chars.get(end).is_none_or(|c| !c.is_ascii_alphanumeric()) {
                out.push_str("***");
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The number in `fail-3.jpg`, `flow-12.jpg`, `step-2.jpg` (for their order).
fn shot_number(name: &str) -> u32 {
    name.split(['-', '.']).nth(1).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// A test run folder: its name, result, steps and the screenshots the report keeps (all of a
/// failed test, the last of a passing one). `Err` when a screenshot is there but cannot be read.
fn test_of(dir: &Path) -> Result<Test, ()> {
    let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let md = std::fs::read_to_string(dir.join("report.md")).unwrap_or_default();
    let first = md.lines().next().unwrap_or("");
    let passed = if first.ends_with("— pass") { Some(true) } else if first.ends_with("— fail") { Some(false) } else { None };
    let name = first.trim_start_matches("# ").rsplit_once(" — ").map(|(n, _)| n.to_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| folder.splitn(3, '-').nth(2).unwrap_or(&folder).to_owned());
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let kept = (n.starts_with("fail-") || n.starts_with("flow-") || n.starts_with("step-")) && n.ends_with(".jpg") && !n.contains("snapshot");
            // Only real files in the run's own folder (never a link a project could plant).
            (kept && e.file_type().is_ok_and(|t| t.is_file())).then(|| (n, e.path()))
        })
        .collect();
    // Failures first, then the screens in the order they were taken.
    files.sort_by_key(|(n, _)| (!n.starts_with("fail-"), n.starts_with("step-"), shot_number(n)));
    if passed == Some(true) {
        files.sort_by_key(|(n, _)| (n.starts_with("fail-"), shot_number(n)));
        files = files.into_iter().rfind(|(n, _)| !n.starts_with("fail-")).into_iter().collect();
    }
    let mut shots = Vec::new();
    for (n, p) in files {
        let bytes = std::fs::read(&p).map_err(|_| ())?;
        shots.push((format!("{name} · {}", n.trim_end_matches(".jpg").replace('-', " ")), bytes));
    }
    let steps = md.lines().skip(1).collect::<Vec<_>>().join("\n");
    Ok(Test { name, passed, steps: mask(&steps), shots })
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// Page text as Markdown text: no link, image, HTML tag or table cell can come from it
/// (Security L18).
fn md_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' | '\r' => o.push(' '),
            '\\' | '`' | '*' | '_' | '[' | ']' | '(' | ')' | '!' | '<' | '>' | '|' | '#' => {
                o.push('\\');
                o.push(c);
            }
            _ => o.push(c),
        }
    }
    o
}

/// The steps as a `text` code block, its fence longer than any backtick run inside.
fn md_fence(steps: &str) -> String {
    let longest = steps.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}text\n{}\n{fence}", steps.trim_end())
}

/// The report folder and the History folders above it, each a real folder: a link is refused, a
/// missing one is made owner-only, one level at a time, and a folder that was there is left as
/// it is (Security L17).
fn own_dir(dir: &Path) -> std::io::Result<()> {
    let reports = dir.parent().ok_or(std::io::ErrorKind::NotFound)?;
    let history = reports.parent().ok_or(std::io::ErrorKind::NotFound)?;
    for d in [history, reports, dir] {
        match std::fs::symlink_metadata(d) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(std::io::Error::other(format!("{} is not a folder. Remove it and run the command again", d.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut b = std::fs::DirBuilder::new();
                #[cfg(unix)]
                std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
                b.create(d)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// A new owner-only file in place of the old one (never through a link).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if std::fs::symlink_metadata(path).is_ok() {
        std::fs::remove_file(path)?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)?.write_all(bytes)
}

/// The newest run of this project's History (`id: None`) or the row `id` (`suite:…` / `run:…`).
pub fn make(id: Option<&str>, s: Surface) -> Result<Made, Fail> {
    let history = History::from_env();
    let id = match id.map(str::trim).filter(|i| !i.is_empty()) {
        Some(i) => i.to_owned(),
        None => history.runs().into_iter().next().map(|r| r.id).ok_or_else(|| fail("no_run", "There is no run to report yet. Run a test first."))?,
    };
    let (row, folders) = history.row(&id).ok_or_else(|| not_found(&id, s))?;
    let dir = history.report_dir(&id).ok_or_else(|| not_found(&id, s))?;
    let tests: Vec<Test> = folders.iter().map(|d| test_of(d)).collect::<Result<_, _>>().map_err(|_| failed_to_read(s))?;
    let shots: usize = tests.iter().map(|t| t.shots.len()).sum();
    let app = crate::apps::current().map(|a| a.name).unwrap_or_default();
    let title = mask(&row.title);
    let meta = [app.as_str(), &when(row.finished_ms)].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" · ");
    let verdict = if row.failed > 0 {
        format!("{} of {} tests failed, {} passed.", row.failed, row.tests, row.passed)
    } else if row.not_run > 0 && row.passed == 0 {
        format!("{} of {} tests could not run.", row.not_run, row.tests)
    } else {
        format!("{} of {} tests passed.", row.passed, row.tests)
    };
    let word = |t: &Test| match t.passed {
        Some(true) => "Passed",
        Some(false) => "Failed",
        None => "Not run",
    };
    own_dir(&dir).map_err(|e| fail("report_failed", format!("Couldn’t make the report: {e}.")))?;
    // Old screenshots of an earlier report of this run go: the new one replaces it.
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().starts_with("shot-") {
            let _ = std::fs::remove_file(e.path());
        }
    }

    // Markdown: same sections, screenshots as files next to it.
    let mut md = format!("# {}\n\n{}\n\n**{verdict}**\n\n| Test | Result |\n|---|---|\n", md_escape(&title), md_escape(&meta));
    for t in &tests {
        md.push_str(&format!("| {} | {} |\n", md_escape(&mask(&t.name)), word(t)));
    }
    let mut n = 0;
    let mut md_shots = String::new();
    for t in &tests {
        if t.passed == Some(false) {
            md.push_str(&format!("\n## What failed: {}\n\n{}\n", md_escape(&mask(&t.name)), md_fence(&t.steps)));
        }
        for (label, bytes) in &t.shots {
            n += 1;
            let file = format!("shot-{n}.jpg");
            write_private(&dir.join(&file), bytes).map_err(|_| failed_to_read(s))?;
            md_shots.push_str(&format!("![{}]({file})\n\n", md_escape(&mask(label))));
        }
    }
    md.push_str(&format!("\n## Screenshots\n\n{md_shots}{SHOTS_LINE}\n"));

    // The web page: one file that opens offline, screenshots inlined.
    use base64::Engine;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let rows: String = tests.iter().map(|t| format!("<tr><td>{}</td><td class=\"{}\">{}</td></tr>", html_escape(&mask(&t.name)), word(t).to_lowercase().replace(' ', "-"), word(t))).collect();
    let mut body = String::new();
    for t in tests.iter().filter(|t| t.passed == Some(false)) {
        body.push_str(&format!("<h2>What failed: {}</h2>", html_escape(&mask(&t.name))));
        for (label, bytes) in &t.shots {
            body.push_str(&format!("<figure><img alt=\"{0}\" src=\"data:image/jpeg;base64,{1}\"><figcaption>{0}</figcaption></figure>", html_escape(&mask(label)), b64(bytes)));
        }
        body.push_str(&format!("<h3>Steps of the failed test</h3><pre>{}</pre>", html_escape(&t.steps)));
    }
    let passing: Vec<&Test> = tests.iter().filter(|t| t.passed != Some(false) && !t.shots.is_empty()).collect();
    if !passing.is_empty() {
        body.push_str("<h2>Last screen of each passing test</h2>");
        for t in passing {
            for (label, bytes) in &t.shots {
                body.push_str(&format!("<figure><img alt=\"{0}\" src=\"data:image/jpeg;base64,{1}\"><figcaption>{0}</figcaption></figure>", html_escape(&mask(label)), b64(bytes)));
            }
        }
    }
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'\"><title>{t}</title>\
<style>body{{font:15px/1.5 -apple-system,system-ui,sans-serif;max-width:900px;margin:32px auto;padding:0 20px;color:#1d2330}}.brand{{color:#6b7280;font-size:13px}}.verdict{{border:1px solid #d6dae1;border-radius:12px;padding:12px 16px;font-weight:600}}\
table{{border-collapse:collapse;width:100%}}td,th{{border-bottom:1px solid #e5e7eb;padding:6px 8px;text-align:left}}.failed{{color:#b42f2f}}.passed{{color:#15803d}}figure{{margin:12px 0}}img{{max-width:100%;border:1px solid #e5e7eb;border-radius:8px}}\
figcaption{{color:#6b7280;font-size:12px}}pre{{white-space:pre-wrap;background:#f6f7f9;border-radius:8px;padding:12px;font-size:13px}}footer{{color:#6b7280;font-size:13px;margin-top:32px;border-top:1px solid #e5e7eb;padding-top:12px}}</style></head>\
<body><p class=\"brand\">fastpilot · test report</p><h1>{t}</h1><p>{m}</p><p class=\"verdict\">{v}</p><table><tr><th>Test</th><th>Result</th></tr>{rows}</table>{body}\
<footer>Made by fastpilot on this Mac. Passwords and card numbers in the text show as ***. {SHOTS_LINE} Nothing was uploaded.</footer></body></html>",
        t = html_escape(&title),
        m = html_escape(&meta),
        v = html_escape(&verdict),
    );
    let (html_path, md_path) = (dir.join("report.html"), dir.join("report.md"));
    write_private(&html_path, html.as_bytes()).map_err(|_| failed_to_read(s))?;
    write_private(&md_path, md.as_bytes()).map_err(|_| failed_to_read(s))?;
    let line = format!("Made report for {} · {} · {} · {}", q(&title), when(row.finished_ms), result_of(&row), shots_word(shots));
    Ok(Made { id, line, html: html_path, md: md_path })
}

/// The newest runs, each with whether it has a report.
pub fn list(limit: usize) -> Vec<(Item, bool)> {
    let history = History::from_env();
    history.runs().into_iter().take(limit).map(|r| {
        let has = history.report_dir(&r.id).is_some_and(|d| d.join("report.html").is_file());
        (r, has)
    }).collect()
}

/// A report's file (`html` or `md`) of row `id`, only when it is a real file in its own folder.
pub fn file(id: &str, kind: &str) -> Option<PathBuf> {
    let dir = History::from_env().report_dir(id)?;
    let p = dir.join(match kind {
        "md" => "report.md",
        _ => "report.html",
    });
    std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()).then_some(p)
}

pub const USAGE: &str = "fastpilot report — make a report of a run

Usage: fastpilot report [<run-id>] [--open] [--markdown] [--list]

  (no id)       The newest run of the app in use.
  <run-id>      This run. Run fastpilot report --list to see the ids.
  --open        Open the web page in your browser.
  --markdown    Print only the Markdown (to copy it: fastpilot report --markdown | pbcopy).
  --list        Show recent reports.

The report is saved with the run. A new one replaces the old one of that run.";

/// `fastpilot report …` (TEXT-terminal-mcp.md 8.9).
pub fn cli(args: &[String]) -> Result<(), Fail> {
    let flag = |f: &str| args.iter().any(|a| a == f);
    let id = args.iter().find(|a| !a.starts_with("--")).map(String::as_str);
    if flag("--list") {
        let all = list(20);
        if all.is_empty() {
            println!("No reports yet. Make one: fastpilot report");
            return Ok(());
        }
        println!("Reports");
        let w = all.iter().map(|(r, _)| r.id.chars().count()).max().unwrap_or(0);
        for (r, has) in &all {
            println!("  {:w$}   {:24} {:20} {}", r.id, mask(&r.title).chars().take(24).collect::<String>(), when(r.finished_ms), if *has { result_of(r) } else { "no report yet".into() });
        }
        return Ok(());
    }
    let made = make(id, Surface::Cli)?;
    if flag("--markdown") {
        print!("{}", std::fs::read_to_string(&made.md).unwrap_or_default());
        return Ok(());
    }
    println!("✓ {}", made.line);
    println!("  Web page   {}", made.html.display());
    println!("  Markdown   {}", made.md.display());
    println!("  Made on this Mac. Nothing was uploaded.");
    println!("  {SHOTS_LINE}");
    if flag("--open") {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        if std::process::Command::new(opener).arg(&made.html).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success()) {
            println!("  Opened report.html in your browser.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Security L18: page text in the Markdown makes no link, image or tag, and the steps stay
    /// inside their code block even when they hold a fence.
    #[test]
    fn page_text_in_the_markdown_is_only_text() {
        assert_eq!(md_escape("![x](https://e.test/a.png) <img> [a](b) | c"), "\\!\\[x\\]\\(https://e.test/a.png\\) \\<img\\> \\[a\\]\\(b\\) \\| c");
        assert_eq!(md_fence("- step\n```\n[a](b)"), "````text\n- step\n```\n[a](b)\n````");
    }

    /// Security L17: a report folder (or a History folder above it) that is a link is refused, and
    /// nothing is written through it; a new folder is owner-only.
    #[cfg(unix)]
    #[test]
    fn a_linked_report_folder_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("fp-own-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(root.join("history")).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("history/reports")).unwrap();
        assert!(own_dir(&root.join("history/reports/suite-1")).is_err());
        assert!(!elsewhere.join("suite-1").exists(), "nothing made through the link");
        std::fs::remove_file(root.join("history/reports")).unwrap();
        own_dir(&root.join("history/reports/suite-1")).unwrap();
        assert_eq!(std::fs::metadata(root.join("history/reports/suite-1")).unwrap().permissions().mode() & 0o777, 0o700);
        std::os::unix::fs::symlink(&elsewhere, root.join("history/reports/run-2")).unwrap();
        assert!(own_dir(&root.join("history/reports/run-2")).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    /// E9: card numbers in a report's text show as ***, like passwords; other numbers stay.
    #[test]
    fn card_numbers_are_masked_in_report_text() {
        assert_eq!(mask("Paid with 4111 1111 1111 1111 on 25 Nov"), "Paid with *** on 25 Nov");
        assert_eq!(mask("card 4111-1111-1111-1111."), "card ***.");
        assert_eq!(mask("order 1234567890 · step 3 of 7"), "order 1234567890 · step 3 of 7");
        assert_eq!(shot_number("fail-12.jpg"), 12);
    }
}
