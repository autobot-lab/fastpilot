//! D10c: the skill library as people see it (v0.4 designs S10, priority-v9 G8): every built-in
//! skill in a pack, with a plain name and one line; which skills to suggest for a new agent and
//! why (Shared/fastpilot-skills/README.md "Suggested skills"); and the community libraries, shown
//! with their license and linked only: fastpilot installs nothing from them.

use serde_json::{Value, json};

/// The packs, in order: id, name, what they have in common.
pub const PACKS: &[(&str, &str, &str)] = &[
    ("black-box", "Black-box", "design from rules"),
    ("experience", "Experience-based", "no written rules needed"),
    ("collaboration", "Collaboration", "from requirements"),
    ("management", "Management", "reports and decisions"),
];

/// "Let fastpilot choose the technique": its own switch at the top.
pub const CHOOSER: &str = "choose-technique";

/// Every skill an agent can switch on: id, pack, name, one line.
pub const CATALOG: &[(&str, &str, &str, &str)] = &[
    ("equivalence-partitioning", "black-box", "Equivalence partitions", "Groups values that behave the same and tests one from each group."),
    ("boundary-value-analysis", "black-box", "Boundary values", "Tests just below, on and above every limit, where bugs hide."),
    ("decision-table", "black-box", "Decision table", "Tries every combination of the conditions behind a price or a permission."),
    ("state-transition", "black-box", "State transitions", "Tests allowed and forbidden status changes, such as cancelling a shipped order."),
    ("pairwise", "black-box", "Pairwise", "Covers many options that combine with a small set of tests."),
    ("use-case-scenarios", "black-box", "Use case scenarios", "Walks a whole journey, plus its other paths and error paths."),
    ("edge-cases", "experience", "Try to break it", "Error guessing: empty, very long, odd characters, double taps."),
    ("exploratory-check", "experience", "Explore for a set time", "Looks around one area and writes down what is wrong or unclear."),
    ("checklist-based", "experience", "Checklists", "Reports every item of a checklist as passed, failed or not checked."),
    ("safe-checkout", "experience", "Checklist · Safe checkout", "Cart, voucher, shipping, total. Stops before Pay."),
    ("login-and-otp", "experience", "Checklist · Login and one-time code", "Right and wrong logins, locked accounts, one-time codes."),
    ("form-validation", "experience", "Checklist · Forms", "Required fields, wrong formats, error messages."),
    ("accessibility-basics", "experience", "Checklist · Accessibility", "Names, labels, focus and contrast basics."),
    ("mobile-basics", "experience", "Checklist · Mobile", "Permissions, the back button, offline and slow network."),
    ("ticket-to-test", "collaboration", "Acceptance tests", "Turns a story, acceptance criteria or a ticket into tests."),
    ("regression", "management", "Regression", "Runs every saved test again and says what is newly broken."),
    ("bug-report", "management", "Bug report", "Turns a failure into a report a developer can act on."),
    ("test-report", "management", "Test report", "Sums up results for people who were not watching."),
    ("release-check", "management", "Release check", "One verdict: SHIP, SHIP WITH FIXES or DO NOT SHIP."),
];

/// Libraries made by others: linked, never installed by fastpilot.
pub const COMMUNITY: &[(&str, &str, &str, &str, &str)] = &[
    (
        "QA skills",
        "Petr Kindlmann",
        "https://github.com/petrkindlmann/qa-skills",
        "MIT — free to use, credit kept",
        "Testing skills for AI agents. Some fastpilot skills credit ideas from it.",
    ),
    (
        "Awesome QA skills",
        "naodeng",
        "https://github.com/naodeng/awesome-qa-skills",
        "Noncommercial — personal use only",
        "A large list of QA skills. Its license allows personal, non-business use only; fastpilot takes nothing from it.",
    ),
];

/// Skills to tick for a new agent, with the reason in the user's own words (3–4).
pub fn suggest(job: &str, app: &str, devices: &[String]) -> Vec<(String, String)> {
    let text = format!(" {} {} ", job.to_lowercase(), app.to_lowercase());
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |id: &str, why: String| {
        if out.len() < 4 && !out.iter().any(|(x, _)| x == id) {
            out.push((id.to_owned(), why));
        }
    };
    let rules: &[(&[&str], &[&str])] = &[
        (&["checkout", "cart", "payment", "voucher", "promo"], &["safe-checkout", "edge-cases"]),
        (&["login", "account", "otp", "sign up", "signup", "password"], &["login-and-otp", "form-validation"]),
        (&["form", "register", "profile", "address"], &["form-validation", "edge-cases"]),
        (&["release", "deploy", "staging", "go/no-go"], &["release-check", "test-report"]),
        (&["ticket", "jira", "story"], &["ticket-to-test"]),
    ];
    for (words, skills) in rules {
        if let Some(w) = words.iter().find(|w| text.contains(*w)) {
            for s in *skills {
                add(s, format!("you said \"{w}\""));
            }
        }
    }
    if devices.iter().any(|d| d != "web") {
        add("mobile-basics", "you picked a phone or an emulator".into());
    }
    add("regression", "useful for every agent".into());
    add("bug-report", "useful for every agent".into());
    out
}

/// The library for the page: packs, skills (with their license and credit) and the community.
pub fn library() -> Value {
    let skills: Vec<Value> = CATALOG
        .iter()
        .filter_map(|(id, pack, name, line)| {
            let text = fastpilot_mcp::skills::get(id)?;
            let license = text.lines().take(8).find_map(|l| l.strip_prefix("license: ")).unwrap_or("Apache-2.0");
            let credit = text.contains("petrkindlmann/qa-skills").then_some("Ideas adapted from open-source work (MIT)");
            Some(json!({ "id": id, "pack": pack, "name": name, "line": line, "description": fastpilot_mcp::skills::description(text), "license": license, "credit": credit }))
        })
        .collect();
    let packs: Vec<Value> = PACKS.iter().map(|(id, name, about)| json!({ "id": id, "name": name, "about": about })).collect();
    let community: Vec<Value> =
        COMMUNITY.iter().map(|(name, by, url, license, about)| json!({ "name": name, "by": by, "url": url, "license": license, "about": about, "personal_only": license.starts_with("Noncommercial") })).collect();
    json!({ "packs": packs, "skills": skills, "chooser": CHOOSER, "community": community })
}

/// Whether `id` is a skill an agent may switch on.
pub fn known(id: &str) -> bool {
    id == CHOOSER || CATALOG.iter().any(|(x, ..)| *x == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_skill_is_in_a_pack_once() {
        let built_in: Vec<&str> = fastpilot_mcp::skills::SKILLS.iter().map(|(n, _)| *n).filter(|n| *n != "fastpilot" && *n != CHOOSER).collect();
        let mut listed: Vec<&str> = CATALOG.iter().map(|(id, ..)| *id).collect();
        listed.sort();
        let mut want = built_in.clone();
        want.sort();
        assert_eq!(listed, want, "every built-in skill has a place in a pack");
        assert!(CATALOG.iter().all(|(_, pack, ..)| PACKS.iter().any(|(p, ..)| p == pack)));
    }

    #[test]
    fn suggestions_follow_the_rules_with_reasons() {
        let s = suggest("Keep cart and payment working", "Shop (staging)", &["android_emulator".into()]);
        assert_eq!(s.len(), 4, "{s:?}");
        assert_eq!(s[0], ("safe-checkout".to_owned(), "you said \"cart\"".to_owned()));
        assert!(s.iter().any(|(id, _)| id == "edge-cases"));
        let s = suggest("Check the login", "", &["web".into()]);
        assert_eq!(s.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["login-and-otp", "form-validation", "regression", "bug-report"]);
        let s = suggest("Test the app", "", &["web".into()]);
        assert_eq!(s.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["regression", "bug-report"]);
        let lib = library();
        assert!(lib["community"].as_array().is_some_and(|c| c.iter().any(|x| x["personal_only"] == true && x["url"].as_str().is_some_and(|u| u.contains("naodeng")))));
        assert!(lib["skills"].as_array().is_some_and(|s| s.iter().any(|x| x["id"] == "bug-report" && x["credit"].is_string())));
    }
}
