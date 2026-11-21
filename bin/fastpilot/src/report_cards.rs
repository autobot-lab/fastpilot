//! S8 (v0.4): the Chat shows a release verdict and a bug report as cards, not as raw text.
//!
//! The bot writes them in the release-check and bug-report skills' formats. fastpilot finds such a
//! block in the answer, reads its fields, and the page draws the card from those fields with DOM
//! text only. The cards are read-only: nothing is sent anywhere, buttons only start work in Chat
//! or copy text. Missing evidence stays visible: "Not checked" is always on the verdict card.

use serde_json::{Value, json};

const VERDICTS: [&str; 4] = ["SHIP WITH FIXES", "DO NOT SHIP", "INCONCLUSIVE", "SHIP"];

/// A line without list markers, emphasis and quote marks around it.
fn bare(line: &str) -> &str {
    line.trim().trim_start_matches(['-', '*', '•', '>']).trim().trim_start_matches("**").trim()
}

/// `Key: value` (case-insensitive key, `**Key:**` allowed).
fn field<'a>(line: &'a str, keys: &[&str]) -> Option<&'a str> {
    let b = bare(line);
    keys.iter().find_map(|k| {
        let head = b.get(..k.len())?;
        if !head.eq_ignore_ascii_case(k) {
            return None;
        }
        let rest = b[k.len()..].trim_start_matches("**").trim_start();
        rest.strip_prefix(':').map(|r| r.trim().trim_start_matches("**").trim())
    })
}

fn verdict_word(s: &str) -> Option<&'static str> {
    let up = s.trim().trim_matches(['*', '`', '.']).trim().to_uppercase();
    VERDICTS.iter().find(|v| up == **v || up.starts_with(&format!("{v} ")) || up.starts_with(&format!("{v} ("))).copied()
}

/// The lines of the block around line `at`: its code fence, or the lines up to the first blank
/// line after which no field or list item follows.
fn block(lines: &[&str], at: usize, keys: &[&str]) -> (usize, usize) {
    let fences: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.trim_start().starts_with("```")).map(|(i, _)| i).collect();
    let open = fences.iter().filter(|f| **f < at).count();
    if open % 2 == 1 {
        let start = fences.iter().rev().find(|f| **f < at).copied().unwrap_or(at);
        let end = fences.iter().find(|f| **f > at).copied().unwrap_or(lines.len() - 1);
        return (start, end);
    }
    let known = |l: &str| {
        let b = bare(l);
        !b.is_empty() && (keys.iter().any(|k| field(l, &[k]).is_some() || b.eq_ignore_ascii_case(k)) || l.starts_with([' ', '\t']) || b.starts_with(['✓', '✗', '!', '–']) || b.chars().next().is_some_and(|c| c.is_ascii_digit()) || l.trim_start().starts_with(['-', '*', '•']))
    };
    let mut start = at;
    for back in 1..=2 {
        let Some(i) = at.checked_sub(back) else { break };
        let l = bare(lines[i]).to_lowercase();
        if l.contains("release check") || l.contains("ship check") {
            start = i;
            break;
        }
        if lines[i].trim().is_empty() {
            break;
        }
    }
    let mut end = at;
    let mut i = at + 1;
    while i < lines.len() {
        if lines[i].trim().is_empty() {
            match lines[i..].iter().position(|l| !l.trim().is_empty()) {
                Some(n) if known(lines[i + n]) => i += n,
                _ => break,
            }
            continue;
        }
        end = i;
        i += 1;
    }
    (start, end)
}

/// The text without the lines `start..=end`.
fn without(lines: &[&str], start: usize, end: usize) -> String {
    lines.iter().enumerate().filter(|(i, _)| *i < start || *i > end).map(|(_, l)| *l).collect::<Vec<_>>().join("\n")
}

/// One evidence item in the card's words: checked, problem, could not run, not checked, blocked.
fn state_of(text: &str) -> &'static str {
    let t = text.to_lowercase();
    let n = |w: &str| t.match_indices(w).any(|(i, _)| t[..i].trim_end().rsplit(' ').next().is_some_and(|d| d.parse::<u32>().is_ok_and(|n| n > 0)));
    if t.contains("not checked") || t.contains("not compared") {
        "not checked"
    } else if t.contains("could not run") || t.contains("did not run") || t.contains("not run") || t.contains("no device") || t.contains("unavailable") {
        "could not run"
    } else if n("failed") || n("broken") || n("error") || n("problem") || (t.contains("crash") && !t.contains("no crash")) || t.contains("fails") {
        "problem"
    } else if t.contains("blocked") && !t.contains("passed") {
        "blocked"
    } else {
        "checked"
    }
}

fn evidence_item(raw: &str) -> Value {
    let b = bare(raw);
    let (mark, rest) = match b.chars().next() {
        Some(c @ ('✓' | '✗' | '!' | '–')) => (Some(c), b[c.len_utf8()..].trim()),
        _ => (None, b),
    };
    // `(state)` at the end, as the Ship check writes it.
    let (rest, said) = match rest.rsplit_once(" (") {
        Some((r, s)) if ["checked)", "problem)", "could not run)", "not checked)", "blocked)"].contains(&s) => (r.trim(), Some(s.trim_end_matches(')'))),
        _ => (rest, None),
    };
    let (name, detail) = if let Some((n, d)) = rest.split_once("  ") {
        (n.trim(), d.trim())
    } else if let Some((n, d)) = rest.split_once(':').filter(|(n, _)| n.split_whitespace().count() <= 3) {
        (n.trim(), d.trim())
    } else {
        rest.split_once(' ').map_or((rest, ""), |(n, d)| (n, d.trim()))
    };
    let state = said
        .map(|s| match s {
            "checked" => "checked",
            "problem" => "problem",
            "could not run" => "could not run",
            "blocked" => "blocked",
            _ => "not checked",
        })
        .or(match mark {
            Some('✗') => Some("problem"),
            Some('!') => Some("could not run"),
            Some('–') => Some("not checked"),
            _ => None,
        })
        .unwrap_or_else(|| state_of(rest));
    let mut name = name.to_owned();
    if let Some(f) = name.get(..1) {
        name = f.to_uppercase() + &name[1..];
    }
    json!({ "name": name, "detail": detail, "state": state })
}

/// `a; b; c` -> items. A `;` inside brackets belongs to its item (V042: "smoke 2/2 (login works;
/// wrong password is refused)" is one row, not "Smoke" and "Wrong").
fn items(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut cur) = (0i32, String::new());
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            ';' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.into_iter().map(|x| x.trim().trim_end_matches('.').to_owned()).filter(|x| !x.is_empty()).collect()
}

/// The release verdict in `text`: the card's fields, and the text without the report.
pub fn verdict(text: &str) -> Option<(Value, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let at = lines.iter().position(|l| field(l, &["Verdict"]).and_then(verdict_word).is_some())?;
    const KEYS: [&str; 7] = ["Verdict", "Why", "Evidence", "Fix before or soon after release", "Fix before release", "Fixes", "Not checked"];
    let (start, end) = block(&lines, at, &KEYS);
    let word = verdict_word(field(lines[at], &["Verdict"])?)?;
    let mut card = json!({ "verdict": word, "title": "", "why": "", "evidence": [], "fixes": [], "not_checked": [], "text": "" });
    let mut section = "";
    let mut body: Vec<&str> = Vec::new();
    for (i, l) in lines.iter().enumerate().take(end + 1).skip(start) {
        body.push(l);
        let b = bare(l);
        if b.starts_with("```") || b.is_empty() {
            continue;
        }
        let low = b.to_lowercase();
        if i < at && (low.starts_with("release check") || low.starts_with("ship check")) {
            let t = b.split_once(['—', '·']).map_or("", |(_, r)| r).trim();
            card["title"] = json!(t.replace(" — ", " · "));
            continue;
        }
        if field(l, &["Verdict"]).is_some() {
            continue;
        }
        if let Some(v) = field(l, &["Why"]) {
            card["why"] = json!(v);
            section = "";
        } else if let Some(v) = field(l, &["Evidence"]) {
            section = "evidence";
            for x in items(v) {
                card["evidence"].as_array_mut().expect("array").push(evidence_item(&x));
            }
        } else if b.eq_ignore_ascii_case("Evidence") {
            section = "evidence";
        } else if let Some(v) = field(l, &["Fix before or soon after release", "Fix before release", "Fixes"]) {
            section = "fixes";
            card["fixes"] = json!(items(v));
        } else if b.to_lowercase().starts_with("fix before") {
            section = "fixes";
        } else if let Some(v) = field(l, &["Not checked"]) {
            section = "not_checked";
            let v = v.trim_end_matches('.');
            if !v.eq_ignore_ascii_case("nothing") && !v.eq_ignore_ascii_case("none") {
                card["not_checked"] = json!(items(v));
            }
        } else if !section.is_empty() {
            // An item under a heading: "1. …", "- …", "✓ Regression  14 tests: …".
            let item = b.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches(['.', ')']).trim();
            if item.is_empty() || (section == "evidence" && (item.starts_with("    ") || l.starts_with("    "))) && !b.starts_with(['✓', '✗', '!', '–']) {
                continue; // a detail line under an evidence row
            }
            match section {
                "evidence" => card["evidence"].as_array_mut().expect("array").push(evidence_item(item)),
                "fixes" => card["fixes"].as_array_mut().expect("array").push(json!(item)),
                _ => card["not_checked"].as_array_mut().expect("array").push(json!(item)),
            }
        }
    }
    card["text"] = json!(body.iter().filter(|l| !l.trim_start().starts_with("```")).copied().collect::<Vec<_>>().join("\n"));
    Some((card, without(&lines, start, end)))
}

/// A bug report in `text` (the bug-report skill's template): the card's fields, and the text
/// without it.
pub fn bug(text: &str) -> Option<(Value, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let at = lines.iter().position(|l| field(l, &["Title"]).is_some())?;
    const KEYS: [&str; 9] = ["Title", "Steps", "Expected", "Actual", "Reproduced", "Environment", "Evidence", "Impact (estimate)", "Impact"];
    let (start, end) = block(&lines, at, &KEYS);
    let span = &lines[start..=end];
    if !span.iter().any(|l| field(l, &["Expected"]).is_some()) || !span.iter().any(|l| field(l, &["Actual"]).is_some()) {
        return None;
    }
    let mut card = json!({ "title": "", "steps": [], "expected": "", "actual": "", "reproduced": "", "environment": "", "evidence": "", "impact": "", "text": "" });
    let mut in_steps = false;
    let mut body = Vec::new();
    for l in span {
        if l.trim_start().starts_with("```") {
            continue;
        }
        body.push(*l);
        // "Reproduced: yes, 2 of 2 runs  |  Environment: web Chrome".
        for part in l.split(" | ") {
            let set = |card: &mut Value, k: &str, v: &str| card[k] = json!(v.trim());
            if let Some(v) = field(part, &["Title"]) {
                set(&mut card, "title", v);
                in_steps = false;
            } else if field(part, &["Steps"]).is_some() || bare(part).eq_ignore_ascii_case("steps") {
                in_steps = true;
            } else if let Some(v) = field(part, &["Expected"]) {
                set(&mut card, "expected", v);
                in_steps = false;
            } else if let Some(v) = field(part, &["Actual"]) {
                set(&mut card, "actual", v);
                in_steps = false;
            } else if let Some(v) = field(part, &["Reproduced"]) {
                set(&mut card, "reproduced", v);
            } else if let Some(v) = field(part, &["Environment"]) {
                set(&mut card, "environment", v);
            } else if let Some(v) = field(part, &["Evidence"]) {
                set(&mut card, "evidence", v);
            } else if let Some(v) = field(part, &["Impact (estimate)", "Impact"]) {
                set(&mut card, "impact", v);
            } else if in_steps {
                let s = bare(part).trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches(['.', ')']).trim();
                if !s.is_empty() {
                    card["steps"].as_array_mut().expect("array").push(json!(s));
                }
            }
        }
    }
    card["text"] = json!(body.join("\n"));
    Some((card, without(&lines, start, end)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_check_report_becomes_the_verdict_card() {
        let text = "I ran the regression and a smoke check. Here is my recommendation.\n\n```\nRelease check — Shop staging 2.4 — 20 Nov 2026\nVerdict: SHIP WITH FIXES\nWhy: 2 minor problems, main journeys pass (sign in, search, cart → summary)\nEvidence: regression 14 tests (12 passed, 1 flaky, 1 blocked by limits); smoke 4/4; visual not checked (no baseline)\nFix before or soon after release: Login with one-time code is unstable; Voucher field accepts a space as a code\nNot checked: payment (blocked by limits); iPhone (no simulator available)\n```\nAsk me for bug reports.";
        let (card, rest) = verdict(text).expect("card");
        assert_eq!(card["verdict"], "SHIP WITH FIXES");
        assert_eq!(card["title"], "Shop staging 2.4 · 20 Nov 2026");
        assert_eq!(card["why"], "2 minor problems, main journeys pass (sign in, search, cart → summary)");
        let ev: Vec<(String, String)> = card["evidence"].as_array().expect("ev").iter().map(|e| (e["name"].as_str().unwrap_or("").to_owned(), e["state"].as_str().unwrap_or("").to_owned())).collect();
        assert_eq!(ev, [("Regression".into(), "checked".into()), ("Smoke".into(), "checked".into()), ("Visual".into(), "not checked".into())]);
        assert_eq!(card["fixes"], json!(["Login with one-time code is unstable", "Voucher field accepts a space as a code"]));
        assert_eq!(card["not_checked"], json!(["payment (blocked by limits)", "iPhone (no simulator available)"]));
        assert_eq!(rest, "I ran the regression and a smoke check. Here is my recommendation.\n\nAsk me for bug reports.");
    }

    #[test]
    fn the_ship_check_report_and_other_layouts_are_read_too() {
        let text = "Ship check · https://shop.test/\nVerdict: DO NOT SHIP\nWhy: 2 errors, 1 uncaught: the page's own script crashed\nEvidence\n✗ Console errors        2 errors, 1 uncaught (problem)\n    exception: Error: price is undefined\n✓ Failed requests       No failed requests (checked)\n! Broken links          Could not run: blocked (could not run)\nNot checked: colour contrast, keyboard use.";
        let (card, rest) = verdict(text).expect("card");
        assert_eq!(card["verdict"], "DO NOT SHIP");
        let ev = card["evidence"].as_array().expect("ev");
        assert_eq!(ev.len(), 3, "{ev:?}");
        assert_eq!((ev[0]["name"].as_str(), ev[0]["state"].as_str()), (Some("Console errors"), Some("problem")));
        assert_eq!(ev[2]["state"], "could not run");
        assert_eq!(card["not_checked"], json!(["colour contrast, keyboard use"]));
        assert_eq!(rest, "");
        // Bold markdown and a list of fixes; "Not checked: Nothing" is an empty list.
        let (card, _) = verdict("**Verdict:** SHIP\n**Why:** main journeys and the regression pass\n**Evidence:** regression 14 tests: 14 passed; Android 0 of 4 ran, no device was ready\nNot checked: Nothing").expect("card");
        assert_eq!(card["verdict"], "SHIP");
        assert_eq!(card["evidence"][1]["state"], "could not run");
        assert_eq!(card["not_checked"], json!([]));
        // "No …" is not a problem; a number of failures is.
        for (detail, state) in [("pages no crashes or error screens", "checked"), ("regression 14 tests: 0 failed", "checked"), ("main journeys 3 of 4 passed, 1 failed", "problem"), ("the app crashes on start", "problem")] {
            assert_eq!(state_of(detail), state, "{detail}");
        }
        // V042 (QA Low): a ";" inside brackets stays in its row: one "Smoke" row, never "Smoke" and "Wrong".
        let (card, _) = verdict("Verdict: SHIP WITH FIXES\nWhy: one check is slow\nEvidence: regression 14 of 14 passed; smoke 2/2 (login works; wrong password is refused); visual not checked (no baseline)").expect("card");
        let names: Vec<&str> = card["evidence"].as_array().expect("ev").iter().filter_map(|e| e["name"].as_str()).collect();
        assert_eq!(names, ["Regression", "Smoke", "Visual"], "{card}");
        assert_eq!(card["evidence"][1]["detail"], "2/2 (login works; wrong password is refused)");
        // Not a verdict.
        assert!(verdict("Verdict: looks fine to me").is_none());
        assert!(verdict("We should ship it").is_none());
    }

    #[test]
    fn a_bug_report_becomes_the_preview_card() {
        let text = "I ran it once more. It failed again.\n```\nTitle: Total does not change after applying voucher SAVE10 (cart, web staging)\n\nSteps\n  1. Open the staging shop and add 1 product to the cart\n  2. Enter voucher SAVE10\n  3. Look at the order summary\nExpected: A \"10% off\" line and a lower total.\nActual:   The page said \"Voucher not valid\". The total stayed $40.00.\nReproduced: yes, 2 of 2 runs  |  Environment: web Chrome, staging shop, version not known\nEvidence: .fastpilot/runs/x/report.html, step-3.png\nImpact (estimate): people who use a voucher cannot get the discount\n```";
        let (card, rest) = bug(text).expect("card");
        assert_eq!(card["title"], "Total does not change after applying voucher SAVE10 (cart, web staging)");
        assert_eq!(card["steps"].as_array().map(Vec::len), Some(3));
        assert_eq!(card["actual"], "The page said \"Voucher not valid\". The total stayed $40.00.");
        assert_eq!(card["reproduced"], "yes, 2 of 2 runs");
        assert_eq!(card["environment"], "web Chrome, staging shop, version not known");
        assert_eq!(card["impact"], "people who use a voucher cannot get the discount");
        assert_eq!(rest, "I ran it once more. It failed again.");
        assert!(bug("Title: just a title\nno more").is_none(), "Expected and Actual are needed");
    }
}
