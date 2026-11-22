//! PF2 (v0.6): which saved tests to run first after a code change. Reads the changed files from
//! git (read only), names each file's area in plain words ("voucher code", "login page"), and
//! scores every saved test with a fixed set of explainable reasons. No score is ever shown, only
//! one short reason per test; file names stay in the details and the terminal.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The files changed since the base, and what they were compared with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    pub files: Vec<String>,
    /// "main", "origin/main", the base the user named, or "the last commit".
    pub base: String,
}

/// Why the changes could not be read (then everything runs, and the person is told why).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoChanges {
    /// Not a git project, or git is not installed.
    NotGit,
    /// The base the user named does not exist.
    BadBase(String),
    /// Nothing changed.
    Nothing,
}

impl NoChanges {
    /// The plain words for it, with how many tests run instead.
    pub fn message(&self, total: usize) -> String {
        match self {
            Self::NotGit => format!("I cannot see what you changed because this folder is not tracked by git. Running all {total} tests instead."),
            Self::BadBase(b) => format!("I cannot compare with \"{b}\": git does not know it. Running all {total} tests instead."),
            Self::Nothing => format!("I did not find any changes. Running all {total} tests instead."),
        }
    }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).current_dir(dir).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A base the user may name: a branch, tag or commit, never an option.
fn safe_base(b: &str) -> bool {
    !b.is_empty() && !b.starts_with('-') && b.len() <= 200 && b.chars().all(|c| c.is_ascii_alphanumeric() || "._/-~^@".contains(c))
}

/// The files changed in `dir`'s git project since `base` (default: where this branch left main),
/// committed or not, plus new files that git would track.
pub fn read(dir: &Path, base: Option<&str>) -> Result<Changes, NoChanges> {
    if git(dir, &["rev-parse", "--is-inside-work-tree"]).is_none_or(|o| o.trim() != "true") {
        return Err(NoChanges::NotGit);
    }
    let has_head = git(dir, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_some();
    let (from, label) = match base {
        Some(b) => {
            if !safe_base(b) || git(dir, &["rev-parse", "--verify", "--quiet", &format!("{b}^{{commit}}")]).is_none() {
                return Err(NoChanges::BadBase(b.chars().take(80).collect()));
            }
            (git(dir, &["merge-base", "HEAD", b]).map(|s| s.trim().to_owned()).unwrap_or_else(|| b.to_owned()), b.to_owned())
        }
        None => {
            let main = ["origin/main", "main", "origin/master", "master"].into_iter().find(|r| git(dir, &["rev-parse", "--verify", "--quiet", &format!("{r}^{{commit}}")]).is_some());
            match (main, has_head) {
                (Some(m), true) => match git(dir, &["merge-base", "HEAD", m]).map(|s| s.trim().to_owned()) {
                    Some(mb) => (mb, m.trim_start_matches("origin/").to_owned()),
                    None => ("HEAD".into(), "the last commit".into()),
                },
                (_, true) => ("HEAD".into(), "the last commit".into()),
                (_, false) => (String::new(), "nothing yet (no commit)".into()),
            }
        }
    };
    let mut files: Vec<String> = Vec::new();
    if !from.is_empty() {
        files.extend(git(dir, &["diff", "--name-only", &from]).unwrap_or_default().lines().map(str::to_owned));
    }
    files.extend(git(dir, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default().lines().map(str::to_owned));
    files.retain(|f| !f.trim().is_empty() && !f.starts_with(".fastpilot/"));
    files.sort();
    files.dedup();
    if files.is_empty() {
        return Err(NoChanges::Nothing);
    }
    Ok(Changes { files, base: label })
}

/// Words that say nothing about an area.
const GENERIC: &[&str] = &[
    "src", "lib", "app", "apps", "index", "main", "examples", "example", "packages", "docs", "test", "tests", "spec", "specs", "component", "components", "page", "pages", "view", "views", "screen", "screens",
    "the", "and", "utils", "util", "helper", "helpers", "mod", "core", "common", "shared", "public", "static", "assets", "styles", "style", "default", "file", "new",
    "html", "htm", "css", "scss", "js", "jsx", "ts", "tsx", "vue", "svelte", "rs", "kt", "java", "swift", "py", "rb", "go", "php", "json", "yaml", "yml", "md",
];

/// Lowercase words of a name: `checkoutVoucher-form.tsx` -> checkout, voucher, form, tsx.
pub fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
            cur.extend(c.to_lowercase());
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Folders that hold a whole site or app, not one part of it: a step that says "demo" or "web"
/// is not about the changed file (QA v0.6 Low 6).
const CONTAINER_FOLDERS: &[&str] = &["demo", "demos", "sample", "samples", "web", "site", "www", "client", "frontend", "backend", "server", "resources", "res", "templates", "template", "dist", "build", "website", "ui"];

fn meaningful(w: &str) -> bool {
    w.chars().count() >= 3 && !GENERIC.contains(&w) && !w.chars().all(|c| c.is_ascii_digit())
}

/// One changed file: its area in words ("voucher code", "login page") and the words that name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Area {
    pub name: String,
    pub words: Vec<String>,
    /// Words of its folders ("cart" for src/cart/voucher.ts).
    pub folder_words: Vec<String>,
}

pub fn area(path: &str) -> Area {
    let p = Path::new(path);
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let dirs: Vec<String> = p.parent().map(|d| d.iter().map(|c| c.to_string_lossy().to_lowercase()).collect()).unwrap_or_default();
    let page = matches!(ext.as_str(), "html" | "htm" | "vue" | "svelte" | "astro") || dirs.iter().any(|d| matches!(d.as_str(), "pages" | "page" | "views" | "view" | "screens" | "screen" | "routes"));
    let stem: Vec<String> = p.file_stem().map(|s| words(&s.to_string_lossy())).unwrap_or_default().into_iter().filter(|w| meaningful(w)).collect();
    let folder_words: Vec<String> = dirs.iter().flat_map(|d| words(d)).filter(|w| meaningful(w) && !CONTAINER_FOLDERS.contains(&w.as_str())).collect();
    let name_words = if stem.is_empty() { folder_words.iter().rev().take(1).cloned().collect() } else { stem };
    let what = if page { "page" } else { "code" };
    let name = if name_words.is_empty() { format!("{} {what}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()) } else { format!("{} {what}", name_words.join(" ")) };
    Area { name, words: name_words, folder_words }
}

/// A saved test as the chooser sees it.
#[derive(Debug, Clone, Default)]
pub struct TestInfo {
    /// The file name (`login.md`).
    pub file: String,
    pub id: String,
    pub title: String,
    pub target: String,
    /// The whole test text (steps included).
    pub text: String,
}

/// One test's reason to run first, from the fixed set (design state 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub index: usize,
    pub reason: String,
}

/// What to run first and what not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub picked: Vec<Pick>,
    pub not_run: Vec<usize>,
    /// No test looked related: `picked` are the ones that failed most recently.
    pub none_related: bool,
}

/// Earlier failures per test with the files changed at the time (newest last), from
/// `<memory>/changed-failures.json`.
pub type History = BTreeMap<String, Vec<Vec<String>>>;

pub fn history_path(memory_root: &Path) -> PathBuf {
    memory_root.join("changed-failures.json")
}

pub fn load_history(memory_root: &Path) -> History {
    std::fs::read_to_string(history_path(memory_root)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// After a run from changes: the tests that failed, with those files (5 sets per test kept).
pub fn record_failures(memory_root: &Path, failed: &[String], files: &[String]) -> std::io::Result<()> {
    if failed.is_empty() || files.is_empty() {
        return Ok(());
    }
    let mut h = load_history(memory_root);
    for id in failed {
        let sets = h.entry(id.clone()).or_default();
        sets.push(files.iter().take(200).cloned().collect());
        let extra = sets.len().saturating_sub(5);
        sets.drain(..extra);
    }
    std::fs::create_dir_all(memory_root)?;
    std::fs::write(history_path(memory_root), serde_json::to_string_pretty(&h).unwrap_or_default())
}

fn has_word(text: &str, w: &str) -> bool {
    words(text).iter().any(|x| x == w || x.strip_suffix('s') == Some(w) || w.strip_suffix('s') == Some(x.as_str()))
}

/// Score every test; the top `top` related ones run first. `notes`: the project notes' texts;
/// `failed_at`: each test's latest failure time (for "nothing related").
pub fn choose(tests: &[TestInfo], changes: &Changes, notes: &[String], history: &History, failed_at: &BTreeMap<String, String>, top: usize) -> Choice {
    let areas: Vec<Area> = changes.files.iter().map(|f| area(f)).collect();
    let mut scored: Vec<(usize, u32, String)> = Vec::new();
    for (i, t) in tests.iter().enumerate() {
        let target_path = t.target.split("://").nth(1).map_or(t.target.as_str(), |r| r.split_once('/').map_or("", |(_, p)| p));
        let named = format!("{} {} {}", t.title, t.id, target_path);
        let mut score = 0;
        let mut reasons: Vec<(u32, String)> = Vec::new();
        // 1. The area's name is what the test is about.
        if let Some(a) = areas.iter().find(|a| a.words.iter().any(|w| has_word(&named, w))) {
            score += 3;
            reasons.push((3, format!("you changed the {}", a.name)));
        }
        // 2. A project note ties a changed folder to this test.
        let title_words: Vec<String> = words(&t.title).into_iter().filter(|w| w.chars().count() >= 4 && meaningful(w)).collect();
        let by_note = notes.iter().any(|n| {
            let paths: Vec<&str> = n.split_whitespace().map(|w| w.trim_matches(|c: char| !(c.is_alphanumeric() || "/._-".contains(c)))).filter(|w| w.contains('/') && !w.contains("://")).collect();
            !paths.is_empty()
                && paths.iter().any(|p| changes.files.iter().any(|f| f.starts_with(p.trim_start_matches("./"))))
                && title_words.iter().any(|w| has_word(n, w))
        });
        if by_note {
            score += 2;
            reasons.push((2, "your project note says this area belongs to the test".into()));
        }
        // 3. It failed the last time these files changed.
        if history.get(&t.id).is_some_and(|sets| sets.iter().any(|s| s.iter().any(|f| changes.files.contains(f)))) {
            score += 2;
            reasons.push((2, "it failed the last time these files changed".into()));
        }
        // 4. Its steps name the changed file or its folder (the steps only, not its target line).
        // The reason says the word that matched; no word, no reason (QA v0.6 Low 6).
        let steps: String = t.text.lines().filter(|l| l.trim_start().starts_with("- ")).collect::<Vec<_>>().join("\n");
        let by_name = || areas.iter().find_map(|a| a.words.iter().find(|w| has_word(&steps, w)).map(|w| format!("its steps mention \"{w}\", and you changed the {}", a.name)));
        let by_folder = || areas.iter().find_map(|a| a.folder_words.iter().find(|w| has_word(&steps, w)).map(|w| format!("its steps mention \"{w}\", and you changed code in the {w} folder")));
        if reasons.is_empty()
            && let Some(why) = by_name().or_else(by_folder)
        {
            score += 1;
            reasons.push((1, why));
        }
        if score > 0 {
            reasons.sort_by_key(|r| std::cmp::Reverse(r.0));
            scored.push((i, score, reasons.remove(0).1));
        }
    }
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if scored.is_empty() {
        let mut failed: Vec<(usize, &String)> = tests.iter().enumerate().filter_map(|(i, t)| failed_at.get(&t.id).map(|w| (i, w))).collect();
        failed.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(&b.0)));
        let picked: Vec<Pick> = failed.into_iter().take(top).map(|(i, _)| Pick { index: i, reason: "it failed most recently".into() }).collect();
        let not_run = (0..tests.len()).filter(|i| !picked.iter().any(|p| p.index == *i)).collect();
        return Choice { picked, not_run, none_related: true };
    }
    let picked: Vec<Pick> = scored.into_iter().take(top).map(|(index, _, reason)| Pick { index, reason }).collect();
    let not_run = (0..tests.len()).filter(|i| !picked.iter().any(|p| p.index == *i)).collect();
    Choice { picked, not_run, none_related: false }
}

/// "voucher code, login page" (at most 3 areas, then "and N more").
pub fn areas_text(changes: &Changes) -> String {
    let mut names: Vec<String> = Vec::new();
    for f in &changes.files {
        let n = area(f).name;
        if !names.contains(&n) {
            names.push(n);
        }
    }
    let more = names.len().saturating_sub(3);
    let mut t = names.into_iter().take(3).collect::<Vec<_>>().join(", ");
    if more > 0 {
        t.push_str(&format!(" and {more} more"));
    }
    t
}

/// The saved tests of a folder (`tests/*.md`) as the chooser sees them.
pub fn tests_in(dir: &Path) -> Vec<TestInfo> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md") && p.is_file()).collect()).unwrap_or_default();
    files.sort();
    files.iter().filter_map(|p| std::fs::read_to_string(p).ok().map(|text| info(p, text))).collect()
}

pub fn info(path: &Path, text: String) -> TestInfo {
    let field = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_owned()));
    TestInfo {
        file: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        id: field("id:").unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()),
        title: text.lines().find_map(|l| l.strip_prefix("# ").map(str::to_owned)).unwrap_or_default(),
        target: field("target:").unwrap_or_default(),
        text,
    }
}

/// PF2 for the AI (the `related_tests` tool, "test what I changed" in Chat): the tests of `tests`
/// (a folder) to run first with their reasons, the ones not run, and the files for "Details".
pub fn for_ai(project: &Path, tests: &Path, base: Option<&str>, memory: &fastpilot_memory::Memory, notes: &[String]) -> String {
    let all = tests_in(tests);
    if all.is_empty() {
        return format!("No saved tests in {}. Nothing to choose from.", tests.display());
    }
    let rel = |t: &TestInfo| format!("tests/{}", t.file);
    let ch = match read(project, base) {
        Ok(c) => c,
        Err(why) => return format!("{}\nRun every saved test: {}", why.message(all.len()), all.iter().map(rel).collect::<Vec<_>>().join(", ")),
    };
    let failed_at: BTreeMap<String, String> = memory.remembered().into_iter().filter_map(|m| Some((m.test_id.clone(), m.failed_at.last()?.clone()))).collect();
    let choice = choose(&all, &ch, notes, &load_history(memory.root()), &failed_at, 5);
    if choice.picked.is_empty() {
        return format!("No test looks related to your changes, and none failed recently. Run all {} tests: {}", all.len(), all.iter().map(rel).collect::<Vec<_>>().join(", "));
    }
    let mut out = if choice.none_related {
        format!("No test looks related to your changes. Run the {} that failed most recently first.\n", choice.picked.len())
    } else {
        format!("{} of {} tests chosen from your changes (compared with {}).\n", choice.picked.len(), all.len(), ch.base)
    };
    out.push_str(&format!("Changed: {} ({})\nRun these first, and give each one's reason as \"Chosen because: …\":\n", if ch.files.len() == 1 { "1 file".to_owned() } else { format!("{} files", ch.files.len()) }, areas_text(&ch)));
    for p in &choice.picked {
        let t = &all[p.index];
        out.push_str(&format!("- {} \"{}\": chosen because {}\n", rel(t), t.title, p.reason));
    }
    if !choice.not_run.is_empty() {
        out.push_str(&format!(
            "Not run ({}): {}. Say they did not look related, that a related-test run is not a full regression, and offer to run them.\n",
            choice.not_run.len(),
            choice.not_run.iter().map(|i| rel(&all[*i])).collect::<Vec<_>>().join(", ")
        ));
    }
    out.push_str(&format!("Details (file names; show only if asked): {}", ch.files.iter().take(20).cloned().collect::<Vec<_>>().join(", ")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, title: &str, target: &str, steps: &str) -> TestInfo {
        TestInfo { file: format!("{id}.md"), id: id.into(), title: title.into(), target: target.into(), text: format!("# {title}\nid: {id}\ntarget: {target}\n{steps}") }
    }

    #[test]
    fn areas_in_plain_words() {
        assert_eq!(area("src/cart/voucher.ts").name, "voucher code");
        assert_eq!(area("demo/login.html").name, "login page");
        assert_eq!(area("src/pages/checkoutSummary.tsx").name, "checkout summary page");
        assert_eq!(area("src/cart/index.ts").name, "cart code", "a generic file name: its folder");
        assert_eq!(words("checkoutVoucher-form.tsx"), ["checkout", "voucher", "form", "tsx"]);
    }

    /// QA v0.6 Low 6: a change to the tickets page picked the login test "because it uses the same
    /// code or screen as the tickets page" (its step said "demo"). Only a concrete word counts, and
    /// the reason says it.
    #[test]
    fn a_weak_match_is_not_a_reason() {
        let tests = vec![
            t("login", "Login", "http://127.0.0.1:8080/demo/login.html", "- Open the demo login page and sign in\n- Check the Dashboard\n"),
            t("notes", "Save a note", "http://127.0.0.1:8080/demo/widgets.html", "- Type a note and save it\n"),
            t("board", "Help desk board", "http://127.0.0.1:8080/demo/board.html", "- Open the board, then the tickets list\n"),
        ];
        let changes = Changes { files: vec!["demo/tickets.html".into()], base: "main".into() };
        let c = choose(&tests, &changes, &[], &History::new(), &BTreeMap::new(), 5);
        let got: Vec<(&str, &str)> = c.picked.iter().map(|p| (tests[p.index].id.as_str(), p.reason.as_str())).collect();
        assert_eq!(got, [("board", "its steps mention \"tickets\", and you changed the tickets page")]);
        assert_eq!(c.not_run.len(), 2, "login and notes are not related: not run, not chosen");
        assert!(area("demo/tickets.html").folder_words.is_empty() && area("web/src/cart/a.ts").folder_words == ["cart"]);
    }

    #[test]
    fn related_tests_first_each_with_one_reason() {
        let tests = vec![
            t("voucher", "Checkout with voucher SAVE10", "https://shop.test/cart", "- Enter voucher SAVE10\n"),
            t("plain", "Plain checkout", "https://shop.test/cart", "- Open the cart and pay later\n"),
            t("login", "Login with one-time code", "https://shop.test/login", "- Sign in\n"),
            t("search", "Search for shoes", "https://shop.test/search", "- Search shoes\n"),
            t("profile", "Edit the profile", "https://shop.test/profile", "- Change the name\n"),
        ];
        let changes = Changes { files: vec!["src/cart/voucher.ts".into(), "demo/login.html".into()], base: "main".into() };
        let mut history = History::new();
        history.insert("search".into(), vec![vec!["src/cart/voucher.ts".into()]]);
        let notes = vec!["Profile lives in src/account/ and src/cart/".to_owned()];
        let c = choose(&tests, &changes, &notes, &history, &BTreeMap::new(), 5);
        let got: Vec<(&str, &str)> = c.picked.iter().map(|p| (tests[p.index].id.as_str(), p.reason.as_str())).collect();
        assert_eq!(
            got,
            [
                ("voucher", "you changed the voucher code"),
                ("login", "you changed the login page"),
                ("search", "it failed the last time these files changed"),
                ("profile", "your project note says this area belongs to the test"),
                ("plain", "its steps mention \"cart\", and you changed code in the cart folder"),
            ]
        );
        assert!(c.not_run.is_empty() && !c.none_related);
        // Top 2: the rest is "not run", never counted as passed.
        let c = choose(&tests, &changes, &notes, &history, &BTreeMap::new(), 2);
        assert_eq!((c.picked.len(), c.not_run.len()), (2, 3));
        assert_eq!(areas_text(&changes), "voucher code, login page");
    }

    #[test]
    fn nothing_related_runs_the_latest_failures() {
        let tests = vec![t("a", "Alpha", "", "- x\n"), t("b", "Beta", "", "- y\n"), t("c", "Gamma", "", "- z\n")];
        let changes = Changes { files: vec!["docs/readme-notes.txt".into()], base: "main".into() };
        let failed: BTreeMap<String, String> = [("a".to_owned(), "2026-11-01T10:00:00Z".to_owned()), ("c".to_owned(), "2026-11-20T10:00:00Z".to_owned())].into();
        let c = choose(&tests, &changes, &[], &History::new(), &failed, 5);
        assert!(c.none_related);
        assert_eq!(c.picked.iter().map(|p| tests[p.index].id.as_str()).collect::<Vec<_>>(), ["c", "a"]);
        assert_eq!(c.not_run, [1]);
    }

    #[test]
    fn git_states_and_a_safe_base() {
        let dir = std::env::temp_dir().join(format!("fp-pf2-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        assert_eq!(read(&dir, None), Err(NoChanges::NotGit));
        let g = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&dir).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").output().expect("git").status.success(), "{args:?}");
        g(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "1").expect("w");
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "one"]);
        assert_eq!(read(&dir, None), Err(NoChanges::Nothing));
        g(&["checkout", "-q", "-b", "feature"]);
        std::fs::create_dir_all(dir.join("demo")).expect("d");
        std::fs::write(dir.join("demo/login.html"), "<h1>x</h1>").expect("w");
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "two"]);
        std::fs::write(dir.join("a.txt"), "2").expect("w");
        std::fs::write(dir.join("new.ts"), "x").expect("w");
        let c = read(&dir, None).expect("changes");
        assert_eq!(c.files, ["a.txt", "demo/login.html", "new.ts"], "committed, uncommitted and new files since main");
        assert_eq!(c.base, "main");
        assert_eq!(read(&dir, Some("--output=/tmp/x")), Err(NoChanges::BadBase("--output=/tmp/x".into())), "an option is never passed to git");
        assert!(matches!(read(&dir, Some("nope")), Err(NoChanges::BadBase(_))));
        assert_eq!(NoChanges::NotGit.message(14), "I cannot see what you changed because this folder is not tracked by git. Running all 14 tests instead.");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What the Chat bot gets from `related_tests`: the order, one reason each, the tests not run
    /// (with what to say about them), and file names only as details.
    #[test]
    fn the_ai_gets_the_choice_with_reasons_and_what_to_say() {
        let dir = std::env::temp_dir().join(format!("fp-pf2-ai-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tests")).expect("tests");
        std::fs::create_dir_all(dir.join("demo")).expect("demo");
        let memory = fastpilot_memory::Memory::new(dir.join(".fastpilot"), fastpilot_memory::Mode::ReadWrite);
        assert!(for_ai(&dir, &dir.join("tests"), None, &memory, &[]).starts_with("No saved tests in"));
        std::fs::write(dir.join("tests/login.md"), "# Login works\nid: login-ok\ntarget: demo/login.html\n- Sign in\n").expect("w");
        std::fs::write(dir.join("tests/search.md"), "# Search for shoes\nid: search-shoes\ntarget: demo/search.html\n- Search\n").expect("w");
        let t = for_ai(&dir, &dir.join("tests"), None, &memory, &[]);
        assert!(t.starts_with("I cannot see what you changed because this folder is not tracked by git. Running all 2 tests instead.") && t.contains("tests/login.md"), "{t}");
        let g = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&dir).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").output().expect("git").status.success());
        std::fs::write(dir.join(".gitignore"), ".fastpilot/\n").expect("w");
        std::fs::write(dir.join("demo/login.html"), "<h1>1</h1>").expect("w");
        g(&["init", "-q", "-b", "main"]);
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "one"]);
        std::fs::write(dir.join("demo/login.html"), "<h1>2</h1>").expect("w");
        let t = for_ai(&dir, &dir.join("tests"), None, &memory, &[]);
        assert!(t.starts_with("1 of 2 tests chosen from your changes (compared with main)."), "{t}");
        assert!(t.contains("Changed: 1 file (login page)") && t.contains("- tests/login.md \"Login works\": chosen because you changed the login page"), "{t}");
        assert!(t.contains("Not run (1): tests/search.md.") && t.contains("not a full regression") && t.contains("offer to run them"), "{t}");
        assert!(t.ends_with("Details (file names; show only if asked): demo/login.html"), "{t}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
