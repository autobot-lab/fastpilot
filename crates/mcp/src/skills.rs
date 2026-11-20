//! The skill library (S5, v0.4): `skills/<name>/SKILL.md` in the repository, built into fastpilot.
//! `fastpilot init` and `fastpilot demo` install them for Claude Code; the `docs` tool serves them
//! (topics `skills` and `skill/<name>`), so any MCP client, the dashboard Chat bot included, can
//! read one when a task needs it.

/// Every skill: folder name and SKILL.md text. `fastpilot` (how to drive fastpilot) comes first.
pub const SKILLS: &[(&str, &str)] = &[
    ("fastpilot", include_str!("../../../skills/fastpilot/SKILL.md")),
    ("accessibility-basics", include_str!("../../../skills/accessibility-basics/SKILL.md")),
    ("boundary-value-analysis", include_str!("../../../skills/boundary-value-analysis/SKILL.md")),
    ("bug-report", include_str!("../../../skills/bug-report/SKILL.md")),
    ("choose-technique", include_str!("../../../skills/choose-technique/SKILL.md")),
    ("decision-table", include_str!("../../../skills/decision-table/SKILL.md")),
    ("edge-cases", include_str!("../../../skills/edge-cases/SKILL.md")),
    ("equivalence-partitioning", include_str!("../../../skills/equivalence-partitioning/SKILL.md")),
    ("exploratory-check", include_str!("../../../skills/exploratory-check/SKILL.md")),
    ("form-validation", include_str!("../../../skills/form-validation/SKILL.md")),
    ("login-and-otp", include_str!("../../../skills/login-and-otp/SKILL.md")),
    ("mobile-basics", include_str!("../../../skills/mobile-basics/SKILL.md")),
    ("regression", include_str!("../../../skills/regression/SKILL.md")),
    ("release-check", include_str!("../../../skills/release-check/SKILL.md")),
    ("safe-checkout", include_str!("../../../skills/safe-checkout/SKILL.md")),
    ("state-transition", include_str!("../../../skills/state-transition/SKILL.md")),
    ("test-report", include_str!("../../../skills/test-report/SKILL.md")),
    ("ticket-to-test", include_str!("../../../skills/ticket-to-test/SKILL.md")),
];

/// A skill's text by name.
pub fn get(name: &str) -> Option<&'static str> {
    SKILLS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// The `description:` of a SKILL.md front matter.
pub fn description(text: &str) -> &str {
    text.strip_prefix("---\n")
        .and_then(|t| t.split_once("\n---\n"))
        .and_then(|(front, _)| front.lines().find_map(|l| l.strip_prefix("description: ")))
        .map_or("", str::trim)
}

/// One line per skill: name and when to use it (the `docs` topic `skills`).
pub fn index() -> String {
    let mut out = String::from(
        "# Skills\n\nRead one with docs(topic: \"skill/<name>\") before a task it fits, and follow it. The `fastpilot` skill \
         is how to drive fastpilot; the others are testing techniques and reports.\n\n",
    );
    for (name, text) in SKILLS {
        out.push_str(&format!("- {name}: {}\n", description(text)));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_skill_is_listed_once_with_a_description() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../skills");
        let mut folders: Vec<String> = std::fs::read_dir(dir).expect("skills").flatten().filter(|e| e.path().join("SKILL.md").is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        folders.sort();
        let mut listed: Vec<String> = super::SKILLS.iter().map(|(n, _)| (*n).to_owned()).collect();
        listed.sort();
        assert_eq!(listed, folders, "every skills/<name>/SKILL.md is built in");
        for (name, text) in super::SKILLS {
            assert!(!super::description(text).is_empty(), "{name}");
            assert!(super::index().contains(&format!("- {name}: ")), "{name}");
        }
    }
}
