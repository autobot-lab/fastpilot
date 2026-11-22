//! PF6 (v0.6): the flaky test doctor. Fixed, explainable rules over a test's recorded runs
//! (`outcome.json` in each run folder): evidence first, then a likely cause in plain words with how
//! many runs it is based on, then what to do. It never invents a cause: with fewer than 3 runs or
//! no failure it says "not enough runs yet", and when no rule fits it says "I cannot tell why".

use std::path::Path;

use serde::{Deserialize, Serialize};

/// What one finished run left behind for the doctor (written by `Run::finish`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Outcome {
    pub test_id: String,
    pub passed: bool,
    /// A pass that came from a retry of a failed attempt.
    #[serde(default)]
    pub passed_on_retry: bool,
    /// Passed with nothing failing on the way (no retry, no failed attempt): what the Memory table
    /// counts as a pass. Older files do not have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clean: Option<bool>,
    #[serde(default)]
    pub could_not_run: bool,
    pub started_ms: u64,
    pub finished_ms: u64,
    /// `web`, `android`, `ios`: the platforms of its sessions.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// The first failed step's label (`tapOn "Pay"`), when a step failed.
    #[serde(default)]
    pub failed_step: Option<String>,
    /// That failure's kind (`not_found`, `timeout`, …) and category (`product`, `infra`, …).
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    /// Its message (redacted, clipped).
    #[serde(default)]
    pub error: Option<String>,
}

impl Outcome {
    /// A pass with nothing failing on the way, as the table counts it.
    pub fn is_clean(&self) -> bool {
        self.clean.unwrap_or(self.passed && !self.passed_on_retry)
    }

    fn network(&self) -> bool {
        let e = self.error.as_deref().unwrap_or("").to_lowercase();
        self.category.as_deref() == Some("infra")
            || ["net::err", "err_connection", "err_name", "econnrefused", "econnreset", "connection refused", "502", "503", "504", "bad gateway", "service unavailable", "gateway timeout"]
                .iter()
                .any(|w| e.contains(w))
    }
}

/// Every run outcome under `<memory>/runs/` (any test), oldest first.
pub fn load_all(memory_root: &Path) -> Vec<Outcome> {
    let mut out: Vec<Outcome> = std::fs::read_dir(memory_root.join("runs"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| serde_json::from_str::<Outcome>(&std::fs::read_to_string(e.path().join("outcome.json")).ok()?).ok())
        .collect();
    out.sort_by_key(|o| o.started_ms);
    out
}

/// The likely cause, from the fixed set (design `05-pf6-flaky-doctor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    NotEnough,
    AlwaysFails,
    Timing,
    Site,
    RealBug,
    Platform,
    Busy,
    Hours,
    UnreliableStep,
    Unknown,
}

/// The panel's content.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Diagnosis {
    pub cause: Cause,
    /// Pass/fail of the runs looked at, oldest first.
    pub runs: Vec<bool>,
    /// Evidence rows: (label, text), e.g. ("Which step", "\"Pay\" · all 3 failures").
    pub evidence: Vec<(String, String)>,
    /// The cause in plain words ("Likely cause: timing. …"), with how many runs it is based on.
    pub text: String,
    /// What to do, when there is something to suggest.
    pub todo: Option<String>,
    /// The button that fits the cause: `chat`, `bug`, `set_aside`, `device` or none.
    pub main: Option<&'static str>,
    /// The step to name in "Ask Chat to fix this test" and the bug report.
    pub step: Option<String>,
}

fn hour(ms: u64) -> u32 {
    fastpilot_core::claim::local_hm(ms).split(':').next().and_then(|h| h.parse().ok()).unwrap_or(12)
}

fn platform_name(p: &str) -> &str {
    match p {
        "android" => "Android",
        "ios" => "iPhone",
        _ => "web",
    }
}

/// A step label in a sentence: `tapOn "Pay"` as it is, a bare label in quotes.
fn quoted(step: &str) -> String {
    if step.contains('"') { step.to_owned() } else { format!("\"{step}\"") }
}

fn times(n: usize) -> String {
    if n == 1 { "1 time".into() } else { format!("{n} times") }
}

/// The doctor for one test: `runs` are its outcomes (any order), `all` every test's outcomes (to see
/// what ran at the same time). Looks at the last 10 runs that could run.
pub fn diagnose(runs: &[Outcome], all: &[Outcome]) -> Diagnosis {
    diagnose_with(runs, all, None)
}

/// The doctor with the runs the Memory table counts (`history`: clean or not, oldest first, up to
/// 10): the panel then shows and counts exactly those, and a run that passed only after a retry is
/// counted as a failure in both (QA v0.6 Low 5). The details come from the outcomes.
pub fn diagnose_with(runs: &[Outcome], all: &[Outcome], history: Option<&[bool]>) -> Diagnosis {
    let mut mine: Vec<&Outcome> = runs.iter().filter(|o| !o.could_not_run).collect();
    mine.sort_by_key(|o| o.started_ms);
    let mine: Vec<&Outcome> = mine.iter().rev().take(10).rev().copied().collect();
    let n = mine.len();
    // Hard failures (the step evidence comes from these) and every run that was not clean.
    let fails: Vec<&Outcome> = mine.iter().filter(|o| !o.passed).copied().collect();
    let f = fails.len();
    let not_clean = mine.iter().filter(|o| !o.is_clean()).count();
    let strip: Vec<bool> = match history {
        Some(h) if !h.is_empty() => h.iter().rev().take(10).rev().copied().collect(),
        _ => mine.iter().map(|o| o.is_clean()).collect(),
    };
    let (hn, hf) = (strip.len(), strip.iter().filter(|p| !**p).count());
    let mut d = Diagnosis { cause: Cause::NotEnough, runs: strip, evidence: vec![], text: String::new(), todo: None, main: None, step: None };
    let details = (n < hn).then(|| format!("fastpilot has the details of {} of them (it records them from version 0.6 on)", if n == 0 { "none".to_owned() } else { n.to_string() }));
    if n < 3 || not_clean == 0 {
        d.text = match &details {
            Some(det) if hf > 0 => format!("Not enough runs yet. It failed {hf} of the last {hn} runs, but {det}. fastpilot needs at least 3 runs with details, with a failure among them, to say what might be wrong. It will show here after the next run."),
            _ => format!(
                "Not enough runs yet. This test has run {} since fastpilot started recording the details. fastpilot needs at least 3 runs, with a failure among them, to say what might be wrong. It will show here after the next run.",
                times(n)
            ),
        };
        return d;
    }
    // Evidence.
    let mut steps: Vec<(String, usize)> = Vec::new();
    for o in &fails {
        let s = o.failed_step.clone().unwrap_or_else(|| "(no step failed: the result check failed)".into());
        match steps.iter_mut().find(|(x, _)| *x == s) {
            Some(e) => e.1 += 1,
            None => steps.push((s, 1)),
        }
    }
    steps.sort_by_key(|s| std::cmp::Reverse(s.1));
    let (top_step, top_n) = steps[0].clone();
    let real_step = fails.iter().any(|o| o.failed_step.as_deref() == Some(top_step.as_str()));
    d.step = real_step.then(|| top_step.clone());
    let retried = mine.iter().filter(|o| o.passed && !o.is_clean()).count();
    let network = fails.iter().filter(|o| o.network()).count();
    let evening = fails.iter().filter(|o| { let h = hour(o.started_ms); !(6..18).contains(&h) }).count();
    let overlap = |o: &Outcome| all.iter().any(|x| x.test_id != o.test_id && x.started_ms < o.finished_ms && o.started_ms < x.finished_ms);
    let busy_fails = fails.iter().filter(|o| overlap(o)).count();
    let passes: Vec<&Outcome> = mine.iter().filter(|o| o.passed).copied().collect();
    let busy_passes = passes.iter().filter(|o| overlap(o)).count();
    let mut platforms: Vec<String> = mine.iter().flat_map(|o| o.platforms.clone()).collect();
    platforms.sort();
    platforms.dedup();
    let same_way = fails.iter().all(|o| o.failed_step.as_deref() == Some(top_step.as_str()) && o.reason == fails[0].reason);
    let how = fails.iter().find(|o| o.failed_step.as_deref() == Some(top_step.as_str())).and_then(|o| o.error.clone()).map(|e| e.chars().take(160).collect::<String>());
    d.evidence.push(("What happened".into(), format!("Failed {hf} of the last {hn} runs (oldest → newest)")));
    if let Some(det) = &details {
        d.evidence.push(("Details".into(), format!("{}{}", det[..1].to_uppercase(), &det[1..])));
    }
    if real_step {
        let all_of = if top_n == f { if f == 2 { "both failures".into() } else { format!("all {f} failures") } } else { format!("{top_n} of {f} failures") };
        d.evidence.push(("Which step".into(), format!("{} · {all_of}", quoted(&top_step))));
    }
    if let Some(h) = &how {
        d.evidence.push(("How it failed".into(), h.clone()));
    }
    if evening > 0 {
        d.evidence.push(("When".into(), format!("{evening} of {f} failures between 18:00 and 06:00")));
    }
    if !platforms.is_empty() {
        let names: Vec<&str> = platforms.iter().map(|p| platform_name(p)).collect();
        d.evidence.push(("Where".into(), if names.len() == 1 { format!("{} only", names[0].replace("web", "Web")) } else { names.join(" and ") }));
    }
    d.evidence.push(("Passes on retry".into(), format!("{retried} of {not_clean} times")));
    if busy_fails > 0 {
        d.evidence.push(("Other tests at the same time".into(), format!("during {busy_fails} of {f} failures")));
    }
    let based = format!(" Based on {n} runs. This is a likely cause, not a certainty.");
    // The rules, in order.
    // Its latest runs all failed (at least 3 in a row), the same way: failing, not unstable.
    let last_k = mine.iter().rev().take_while(|o| !o.passed).count();
    if last_k >= 3 && same_way {
        d.cause = Cause::AlwaysFails;
        d.text = format!("This test is not unstable. It always fails. It has failed in the last {last_k} runs at {}, with the same message. This looks like a real problem.", quoted(&top_step));
        d.todo = Some("Make a bug report.".into());
        d.main = Some("bug");
    } else if retried >= 2 || (retried >= 1 && retried * 2 >= f) {
        d.cause = Cause::Timing;
        // A concrete example of what timing means (QA v0.6: "timing" alone says too little).
        let target = d.step.as_deref().and_then(|s| s.split_once('"').and_then(|(_, r)| r.rsplit_once('"')).map(|(t, _)| t.to_owned())).filter(|t| !t.is_empty());
        let example = match &target {
            Some(t) => format!(" For example: the test looks for \"{t}\" a moment before the page has finished showing it, so it fails, and on the next try it is there."),
            None => " For example: the test checks the screen a moment before the page has finished loading, so it fails, and on the next try it works.".to_owned(),
        };
        d.text = format!("Likely cause: timing. The step fails now and then, and passes when the test waits or tries again.{example}{based}");
        d.todo = Some(match (&d.step, &target) {
            (Some(s), Some(t)) => format!("Make the test wait until \"{t}\" is on the screen before {}, for example with the step `extendedWaitUntil: {{ visible: \"{t}\", timeout: 10000 }}`. It is a small change to the test, not to your app.", quoted(s)),
            (Some(s), None) => format!("Make the test wait at {} until the screen has finished updating, before it checks. It is a small change to the test, not to your app.", quoted(s)),
            _ => "Make the test wait for what it needs before it checks. It is a small change to the test, not to your app.".into(),
        });
        d.main = Some("chat");
    } else if network >= 2 && network * 5 >= f * 3 {
        d.cause = Cause::Site;
        d.text = format!("Likely cause: the site, not your test. {network} of the {f} failures came with network errors or slow answers from the site.{based}");
        d.todo = Some("Set the test aside until the site is steady, or check the site.".into());
        d.main = Some("set_aside");
    } else if f >= 3 && same_way && retried == 0 {
        d.cause = Cause::RealBug;
        d.text = format!("This looks like a real problem in your app. It fails at the same step, the same way, every time it fails, and retrying does not help.{based}");
        d.todo = Some("Make a bug report.".into());
        d.main = Some("bug");
    } else if let Some((bad, good)) = (platforms.len() >= 2)
        .then(|| {
            platforms.iter().find_map(|p| {
                let on = |o: &Outcome| o.platforms.iter().any(|x| x == p);
                let all_fails_here = f >= 2 && fails.iter().all(|o| on(o));
                let runs_here = mine.iter().filter(|o| on(o)).count();
                let fails_here = fails.iter().filter(|o| on(o)).count();
                let passes_elsewhere = passes.iter().any(|o| !on(o));
                (all_fails_here && passes_elsewhere && fails_here * 2 >= runs_here).then(|| (p.clone(), platforms.iter().find(|x| *x != p).cloned().unwrap_or_default()))
            })
        })
        .flatten()
    {
        let runs_here = mine.iter().filter(|o| o.platforms.contains(&bad)).count();
        d.cause = Cause::Platform;
        d.text = format!(
            "Likely cause: {}. It passes on {} and fails on {} {} out of {runs_here}.{based}",
            platform_name(&bad),
            platform_name(&good),
            platform_name(&bad),
            times(f)
        );
        d.todo = Some(if bad == "android" { "Restart the emulator, or run this test on a phone.".into() } else { format!("Check the {} device, or run this test on another one.", platform_name(&bad)) });
        d.main = Some("device");
    } else if f >= 2 && busy_fails * 3 >= f * 2 && busy_passes * 2 < passes.len().max(1) {
        d.cause = Cause::Busy;
        d.text = format!("Likely cause: a busy computer. {busy_fails} of {f} failures happened while another test was running on this Mac.{based}");
        d.todo = Some("Run fewer tests at the same time.".into());
        d.main = Some("chat");
    } else if f >= 3 && evening * 4 >= f * 3 && passes.iter().filter(|o| { let h = hour(o.started_ms); !(6..18).contains(&h) }).count() * 2 < passes.len().max(1) {
        d.cause = Cause::Hours;
        d.text = format!("It fails mostly in the evening or at night. {evening} of {f} failures were between 18:00 and 06:00. fastpilot cannot tell why. It may be a scheduled job on your site.{based}");
        d.todo = Some("Check what runs on your site at that time.".into());
        d.main = Some("bug");
    } else if f >= 2 && real_step && top_n * 3 >= f * 2 {
        d.cause = Cause::UnreliableStep;
        d.text = format!("Likely cause: this step is unreliable. {top_n} of {f} failures were at {}, not always in the same way.{based}", quoted(&top_step));
        d.todo = Some(format!("Make {} steadier: wait until the screen has settled before it acts or checks.", quoted(&top_step)));
        d.main = Some("chat");
    } else {
        d.cause = Cause::Unknown;
        d.text = format!("I cannot tell why. It failed {not_clean} of {n} times, at different steps and with no pattern in the time, the device or the network. Here are the runs, so you can look.");
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run at `h` o'clock local time on day `day` (each 10 min long).
    fn run(test: &str, day: u64, h: u32, passed: bool) -> Outcome {
        // Local midnight of a fixed day plus `h` hours: find the offset with local_hm.
        let base = 1_795_000_000_000u64 + day * 86_400_000;
        let local_h = hour(base);
        let start = base + ((24 + h as u64 - local_h as u64) % 24) * 3_600_000;
        Outcome { test_id: test.into(), passed, started_ms: start, finished_ms: start + 600_000, platforms: vec!["web".into()], ..Outcome::default() }
    }
    fn fail_at(mut o: Outcome, step: &str, reason: &str, error: &str) -> Outcome {
        o.failed_step = Some(step.into());
        o.reason = Some(reason.into());
        o.category = Some("product".into());
        o.error = Some(error.into());
        o
    }

    #[test]
    fn not_enough_runs_and_no_failures() {
        let two = vec![run("t", 1, 10, true), fail_at(run("t", 2, 10, false), "tapOn \"Pay\"", "not_found", "no Pay")];
        let d = diagnose(&two, &two);
        assert_eq!(d.cause, Cause::NotEnough);
        assert!(d.text.starts_with("Not enough runs yet. This test has run 2 times") && d.text.contains("at least 3 runs, with a failure among them"), "{}", d.text);
        let ok: Vec<Outcome> = (0..5).map(|i| run("t", i, 10, true)).collect();
        assert_eq!(diagnose(&ok, &ok).cause, Cause::NotEnough, "no failure: nothing to explain");
    }

    #[test]
    fn each_rule_explains_its_cause() {
        let step = "assertVisible \"10% off\"";
        // Timing: failures, and passes that came from a retry.
        let mut t = vec![];
        for i in 0..10 {
            let mut o = run("t", i, 10, i % 3 != 2);
            if i % 3 == 0 && i > 0 {
                o.passed_on_retry = true;
            }
            if !o.passed {
                o = fail_at(o, step, "not_found", "not visible yet");
            }
            t.push(o);
        }
        let d = diagnose(&t, &t);
        assert_eq!(d.cause, Cause::Timing, "{d:?}");
        assert!(d.text.starts_with("Likely cause: timing.") && d.text.ends_with("Based on 10 runs. This is a likely cause, not a certainty."), "{}", d.text);
        assert_eq!((d.main, d.step.as_deref()), (Some("chat"), Some(step)));
        assert!(d.evidence.iter().any(|(k, v)| k == "Which step" && v == "assertVisible \"10% off\" · all 3 failures"), "{:?}", d.evidence);

        // A real bug: same step, same way, no retry helps (and it passes in between).
        let r: Vec<Outcome> = (0..8).map(|i| if [1, 4, 6].contains(&i) { fail_at(run("t", i, 10, false), "tapOn \"Export\"", "not_found", "no Export") } else { run("t", i, 10, true) }).collect();
        let d = diagnose(&r, &r);
        assert_eq!((d.cause, d.main), (Cause::RealBug, Some("bug")), "{d:?}");

        // Always fails at the same step: not "unstable".
        let a: Vec<Outcome> = (0..6).map(|i| if i < 2 { run("t", i, 10, true) } else { fail_at(run("t", i, 10, false), "tapOn \"Download\"", "not_found", "no file") }).collect();
        let d = diagnose(&a, &a);
        assert_eq!(d.cause, Cause::AlwaysFails, "{d:?}");
        assert!(d.text.starts_with("This test is not unstable. It always fails. It has failed in the last 4 runs at tapOn \"Download\", with the same message."), "{}", d.text);

        // The site: network errors.
        let s: Vec<Outcome> = (0..6)
            .map(|i| if i % 2 == 0 { let mut o = fail_at(run("t", i, 10, false), &format!("step {i}"), "transient", "net::ERR_CONNECTION_REFUSED"); o.category = Some("infra".into()); o } else { run("t", i, 10, true) })
            .collect();
        let d = diagnose(&s, &s);
        assert_eq!((d.cause, d.main), (Cause::Site, Some("set_aside")), "{d:?}");
        assert!(d.text.contains("3 of the 3 failures came with network errors"), "{}", d.text);

        // Only on Android.
        let p: Vec<Outcome> = (0..6)
            .map(|i| {
                let mut o = run("t", i, 10, i % 2 == 0);
                if i % 2 == 1 {
                    o.platforms = vec!["android".into()];
                    o = fail_at(o, &format!("step {i}"), &format!("r{i}"), "x");
                }
                o
            })
            .collect();
        let d = diagnose(&p, &p);
        assert_eq!((d.cause, d.main), (Cause::Platform, Some("device")), "{d:?}");
        assert!(d.text.starts_with("Likely cause: Android. It passes on web and fails on Android 3 times out of 3."), "{}", d.text);

        // A busy computer: the failures overlapped another test's run, the passes did not.
        let b: Vec<Outcome> = (0..6).map(|i| if i % 2 == 1 { fail_at(run("t", i, 10, false), &format!("step {i}"), &format!("r{i}"), "x") } else { run("t", i, 10, true) }).collect();
        let others: Vec<Outcome> = b.iter().filter(|o| !o.passed).map(|o| Outcome { test_id: "other".into(), started_ms: o.started_ms + 60_000, finished_ms: o.finished_ms + 60_000, passed: true, ..Outcome::default() }).collect();
        let all: Vec<Outcome> = b.iter().cloned().chain(others).collect();
        let d = diagnose(&b, &all);
        assert_eq!(d.cause, Cause::Busy, "{d:?}");
        assert!(d.text.contains("3 of 3 failures happened while another test was running"), "{}", d.text);

        // Certain hours.
        let h: Vec<Outcome> = (0..8).map(|i| if i % 2 == 1 { fail_at(run("t", i, 20, false), &format!("step {i}"), &format!("r{i}"), "x") } else { run("t", i, 10, true) }).collect();
        let d = diagnose(&h, &h);
        assert_eq!(d.cause, Cause::Hours, "{d:?}");
        assert!(d.text.contains("4 of 4 failures were between 18:00 and 06:00. fastpilot cannot tell why."), "{}", d.text);

        // The same step most of the time, in different ways.
        let u: Vec<Outcome> = (0..9)
            .map(|i| match i {
                1 => fail_at(run("t", i, 10, false), "tapOn \"Save\"", "not_found", "a"),
                4 => fail_at(run("t", i, 10, false), "tapOn \"Save\"", "timeout", "b"),
                7 => fail_at(run("t", i, 10, false), "tapOn \"Other\"", "not_actionable", "c"),
                _ => run("t", i, 10, true),
            })
            .collect();
        let d = diagnose(&u, &u);
        assert_eq!(d.cause, Cause::UnreliableStep, "{d:?}");
        assert!(d.text.starts_with("Likely cause: this step is unreliable. 2 of 3 failures were at tapOn \"Save\", not always"), "{}", d.text);
    }

    #[test]
    fn no_pattern_means_i_cannot_tell_why() {
        let x: Vec<Outcome> = (0..10)
            .map(|i| match i {
                2 => fail_at(run("t", i, 10, false), "a", "r1", "x"),
                5 => fail_at(run("t", i, 11, false), "b", "r2", "y"),
                8 => fail_at(run("t", i, 12, false), "c", "r3", "z"),
                _ => run("t", i, 10, true),
            })
            .collect();
        let d = diagnose(&x, &x);
        assert_eq!((d.cause, d.main, d.todo.as_deref()), (Cause::Unknown, None, None), "{d:?}");
        assert!(d.text.starts_with("I cannot tell why. It failed 3 of 10 times"), "{}", d.text);
        assert_eq!(d.runs.len(), 10, "the evidence is still shown");
    }
}
