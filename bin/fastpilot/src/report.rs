//! Suite reports for `fastpilot run`: JUnit XML (CI dashboards) and one self-contained HTML file.

use std::fmt::Write as _;
use std::time::Duration;

use crate::run::CaseResult;

fn xml(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&apos;".into(),
            c => c.to_string(),
        })
        .collect()
}

/// JUnit XML: a failed test outside quarantine is a `<failure>` (type = failure class); a
/// quarantined failure is `<skipped>`, so CI stays green but the result is visible.
pub fn junit(results: &[CaseResult], elapsed: Duration) -> String {
    let failures = results.iter().filter(|r| !r.passed && !r.quarantined).count();
    let skipped = results.iter().filter(|r| !r.passed && r.quarantined).count();
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n<testsuite name=\"fastpilot\" tests=\"{}\" failures=\"{failures}\" errors=\"0\" skipped=\"{skipped}\" time=\"{:.3}\">\n",
        results.len(),
        elapsed.as_secs_f64()
    );
    for r in results {
        let _ = writeln!(
            out,
            "  <testcase classname=\"fastpilot.{}\" name=\"{}\" time=\"{:.3}\">",
            xml(&r.case.platform),
            xml(&format!("{}: {}", r.case.id, r.case.title)),
            r.elapsed.as_secs_f64()
        );
        let class = r.class.map(|c| c.as_str()).unwrap_or("product");
        if !r.passed && r.quarantined {
            let _ = writeln!(out, "    <skipped message=\"quarantined ({class}): {}\"/>", xml(&r.notes));
        } else if !r.passed {
            let _ = writeln!(
                out,
                "    <failure type=\"{class}\" message=\"{}\">{}</failure>",
                xml(&r.notes),
                xml(&r.transcript.join("\n\n"))
            );
        }
        let _ = writeln!(
            out,
            "    <system-out>{}</system-out>\n  </testcase>",
            xml(&format!("models: {}; turns: {}; attempts: {}", r.models.join(" -> "), r.turns, r.attempts))
        );
    }
    out.push_str("</testsuite>\n</testsuites>\n");
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// One self-contained HTML file: summary, then per test its verdict, class, timings and the
/// tool-call transcript.
pub fn html(results: &[CaseResult], elapsed: Duration) -> String {
    let passed = results.iter().filter(|r| r.passed).count();
    let mut body = String::new();
    // PF5: a test run on several platforms first, side by side; then every run in one list.
    let side = crate::side::of_results(results);
    if !side.is_empty() {
        body.push_str("<nav class=\"views\"><a href=\"#side\">Side by side</a> · <a href=\"#list\">One list</a></nav><div id=\"side\">");
        for c in &side {
            body.push_str(&c.html());
        }
        body.push_str("</div><h2 id=\"list\">One list</h2>\n");
    }
    for r in results {
        let (badge, cls) = match (r.passed, r.quarantined) {
            (true, _) => ("pass", "pass"),
            (false, true) => ("quarantined", "skip"),
            (false, false) => ("fail", "fail"),
        };
        let class = r.class.map(|c| format!(" · {}", c.as_str())).unwrap_or_default();
        let _ = write!(
            body,
            "<details class=\"case {cls}\"{}><summary><span class=\"badge\">{badge}{class}</span> <b>{}</b> {} \
             <span class=\"meta\">{:.1}s · {} turns · {} attempt(s) · {}</span></summary>\n<p class=\"notes\">{}</p>\n<ol>",
            if r.passed { "" } else { " open" },
            html_escape(&r.case.id),
            html_escape(&r.case.title),
            r.elapsed.as_secs_f64(),
            r.turns,
            r.attempts,
            html_escape(&r.models.join(" → ")),
            html_escape(&r.notes)
        );
        if let Some(p) = &r.report {
            let _ = write!(body, "<li>evidence: <a href=\"file://{0}\">{0}</a></li>", html_escape(&p.display().to_string()));
        }
        body.push_str("</ol>");
        // The run's own report.html (steps, diffs, screenshots, recordings), inlined.
        if let Some(run) = r.report.as_ref().and_then(|p| std::fs::read_to_string(p.with_file_name("report.html")).ok()) {
            let _ = write!(body, "<iframe class=\"run\" title=\"{}\" sandbox=\"allow-scripts\" srcdoc=\"{}\"></iframe>", html_escape(&r.case.id), html_escape(&run));
        }
        body.push_str("<details><summary>agent transcript</summary><ol>");
        for t in &r.transcript {
            let _ = write!(body, "<li><pre>{}</pre></li>", html_escape(t));
        }
        body.push_str("</ol></details></details>\n");
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\">\
<title>fastpilot report</title><style>
:root {{ --fg:#1d1d1f; --bg:#fff; --muted:#6e6e73; --pass:#1a7f37; --fail:#cf222e; --skip:#9a6700; --line:#d0d7de; }}
@media (prefers-color-scheme: dark) {{ :root {{ --fg:#e6edf3; --bg:#0d1117; --muted:#8b949e; --line:#30363d; }} }}
body {{ font: 14px/1.45 system-ui, sans-serif; color: var(--fg); background: var(--bg); margin: 24px auto; max-width: 1100px; padding: 0 16px; }}
.case {{ border: 1px solid var(--line); border-radius: 8px; margin: 10px 0; padding: 8px 12px; }}
.badge {{ font-weight: 600; text-transform: uppercase; font-size: 12px; }}
.pass .badge {{ color: var(--pass); }} .fail .badge {{ color: var(--fail); }} .skip .badge {{ color: var(--skip); }}
.meta, .notes {{ color: var(--muted); }} pre {{ white-space: pre-wrap; word-break: break-word; margin: 4px 0; }}
img {{ max-width: 360px; border: 1px solid var(--line); }}
iframe.run {{ width: 100%; height: 720px; border: 1px solid var(--line); border-radius: 6px; }}
{}</style></head><body><h1>fastpilot report</h1><p>{} tests · {passed} passed · {} failed · {:.1}s</p>\n{body}</body></html>\n",
        if side.is_empty() { "" } else { crate::side::CSS },
        results.len(),
        results.len() - passed,
        elapsed.as_secs_f64()
    )
}
