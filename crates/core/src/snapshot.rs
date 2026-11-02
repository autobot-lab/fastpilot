//! Compact text snapshots (PRD §9): prune, stable refs, render, diff, selector resolution.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::{Error, Result, Selector};

const INTERACTIVE: &[&str] = &[
    "button", "link", "textfield", "checkbox", "radio", "combobox", "listbox", "option",
    "menuitem", "tab", "switch", "slider", "spinbutton",
];
const EDITABLE: &[&str] = &["textfield", "combobox", "spinbutton"];
/// Non-interactive roles that survive pruning; every other role is a container and gets folded.
const CONTENT: &[&str] = &["heading", "text", "image", "dialog", "alertdialog", "alert"];
const MAX_NAME: usize = 80;
const MAX_VALUE: usize = 40;
pub const DEFAULT_MAX_LINES: usize = 120;
const MAX_DIFF_LINES: usize = 40;

/// Raw node as produced by a driver. Drivers drop invisible nodes and map roles onto the
/// canonical vocabulary (`textfield`, `button`, `text`, ...); core does the rest.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub id: Option<String>,
    pub secure: bool,
    pub disabled: bool,
    pub checked: bool,
    pub selected: bool,
    pub expanded: bool,
    pub focused: bool,
    /// Driver-specific handle (CDP backendNodeId, Android node id, ...).
    pub handle: Option<i64>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub r#ref: String,
    pub depth: usize,
    /// Always childless; hierarchy is carried by `depth`.
    pub node: Node,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub title: String,
    /// Shown in the header, e.g. a URL or `android, com.app/.LoginActivity`.
    pub location: String,
    pub entries: Vec<Entry>,
}

/// Per-session `hash(role, name, path) -> eN` allocation, so refs survive across snapshots.
// ponytail: grows for the session's lifetime; evict unseen keys if sessions get very long.
#[derive(Debug, Default)]
pub struct RefRegistry {
    ids: HashMap<u64, u32>,
    next: u32,
}

impl RefRegistry {
    fn get(&mut self, key: u64) -> String {
        let n = *self.ids.entry(key).or_insert_with(|| {
            self.next += 1;
            self.next
        });
        format!("e{n}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Tap,
    Type,
    Find,
}

struct Pending {
    key: u64,
    depth: usize,
    node: Node,
}

impl Snapshot {
    pub fn build(
        title: impl Into<String>,
        location: impl Into<String>,
        roots: Vec<Node>,
        refs: &mut RefRegistry,
    ) -> Self {
        let mut out = Vec::new();
        let mut dups = HashMap::new();
        let mut path = Vec::new();
        for n in roots {
            walk(n, 0, None, &mut path, &mut dups, &mut out);
        }
        // A label rendered next to the control it names is noise: `text "Email"` + `textfield "Email"`.
        let redundant: Vec<bool> = (0..out.len())
            .map(|i| {
                let e = &out[i];
                let twin = |j: usize| {
                    out.get(j).is_some_and(|o: &Pending| {
                        o.node.role != "text" && o.node.name == e.node.name && o.depth == e.depth
                    })
                };
                e.node.role == "text" && (twin(i + 1) || (i > 0 && twin(i - 1)))
            })
            .collect();
        let entries = out
            .into_iter()
            .zip(redundant)
            .filter(|(_, r)| !r)
            .map(|(p, _)| Entry {
                r#ref: refs.get(p.key),
                depth: p.depth,
                node: p.node,
            })
            .collect();
        Self {
            title: title.into(),
            location: location.into(),
            entries,
        }
    }

    pub fn header(&self) -> String {
        format!("screen {:?} ({})", self.title, self.location)
    }

    /// Render with an optional case-insensitive role/name filter and a hard line cap.
    pub fn render(&self, filter: Option<&str>, max_lines: usize) -> String {
        let filter = filter.map(str::to_lowercase);
        let lines: Vec<String> = self
            .entries
            .iter()
            .filter(|e| {
                filter.as_deref().is_none_or(|f| {
                    e.node.role.contains(f) || e.node.name.to_lowercase().contains(f)
                })
            })
            .map(|e| line(e, filter.is_none()))
            .collect();
        let mut out = self.header();
        for l in lines.iter().take(max_lines) {
            out.push('\n');
            out.push_str(l);
        }
        if lines.len() > max_lines {
            out.push_str(&format!(
                "\n... +{} nodes (scroll or snapshot(filter:) to see more)",
                lines.len() - max_lines
            ));
        }
        out
    }

    pub fn resolve(&self, sel: &Selector, intent: Intent) -> Result<&Entry> {
        let not_found = || Error::NotFound(format!("no element matches {sel}"));
        match sel {
            Selector::Ref(r) => self
                .entries
                .iter()
                .find(|e| &e.r#ref == r)
                .ok_or_else(|| Error::NotFound(format!("{r} is not on screen"))),
            Selector::Id(id) => self
                .entries
                .iter()
                .find(|e| e.node.id.as_deref() == Some(id))
                .ok_or_else(not_found),
            Selector::Focused => self
                .entries
                .iter()
                .find(|e| e.node.focused)
                .ok_or_else(|| Error::NotFound("no element has focus".into())),
            Selector::Point(_) => Err(Error::Unsupported(
                "point selectors are handled by the driver".into(),
            )),
            Selector::Role { role, name } => {
                let pool: Vec<&Entry> = self
                    .entries
                    .iter()
                    .filter(|e| e.node.role.eq_ignore_ascii_case(role))
                    .collect();
                match name {
                    None => pool.first().copied().ok_or_else(not_found),
                    Some(n) => pick(sel, &pool, n, intent, false).ok_or_else(not_found)?,
                }
            }
            Selector::Text(t) => {
                let pool: Vec<&Entry> = self.entries.iter().collect();
                pick(sel, &pool, t, intent, intent == Intent::Find).ok_or_else(not_found)?
            }
        }
    }

    /// Nearest alternatives shown when a step fails, so the AI can pick a ref.
    pub fn candidates(&self, sel: &Selector, intent: Intent) -> Vec<String> {
        let words: Vec<String> = match sel {
            Selector::Text(t) | Selector::Role { name: Some(t), .. } => t
                .split_whitespace()
                .filter(|w| w.chars().count() >= 3)
                .map(str::to_lowercase)
                .collect(),
            _ => Vec::new(),
        };
        self.entries
            .iter()
            .filter(|e| match intent {
                Intent::Tap => INTERACTIVE.contains(&e.node.role.as_str()),
                Intent::Type => EDITABLE.contains(&e.node.role.as_str()),
                Intent::Find => {
                    let name = e.node.name.to_lowercase();
                    words.iter().any(|w| name.contains(w))
                }
            })
            .take(5)
            .map(|e| line(e, false))
            .collect()
    }
}

fn walk(
    mut node: Node,
    depth: usize,
    parent: Option<(&str, &str)>,
    path: &mut Vec<String>,
    dups: &mut HashMap<u64, u32>,
    out: &mut Vec<Pending>,
) {
    node.name = clip(&node.name.split_whitespace().collect::<Vec<_>>().join(" "), MAX_NAME);
    let children = std::mem::take(&mut node.children);
    if !keep(&node, parent) {
        for c in children {
            walk(c, depth, parent, path, dups, out);
        }
        return;
    }
    let mut h = DefaultHasher::new();
    (&node.role, &node.name, &*path).hash(&mut h);
    let base = h.finish();
    let dup = dups.entry(base).or_insert(0);
    let mut h = DefaultHasher::new();
    (base, *dup).hash(&mut h);
    *dup += 1;

    let (role, name) = (node.role.clone(), node.name.clone());
    out.push(Pending {
        key: h.finish(),
        depth,
        node,
    });
    path.push(role.clone());
    for c in children {
        walk(c, depth + 1, Some((&role, &name)), path, dups, out);
    }
    path.pop();
}

fn keep(n: &Node, parent: Option<(&str, &str)>) -> bool {
    match n.role.as_str() {
        // Text already carried by its parent: the accessible name (`button "Masuk"` > `text "Masuk"`)
        // or the value of an input (never repeat a secure field's content).
        "text" => {
            !n.name.is_empty()
                && !parent.is_some_and(|(role, name)| {
                    EDITABLE.contains(&role)
                        || (role != "dialog" && role != "alertdialog" && name.contains(&n.name))
                })
        }
        "image" => !n.name.is_empty(),
        r => INTERACTIVE.contains(&r) || CONTENT.contains(&r),
    }
}

/// `pick` returns `Some(Err)` for ambiguity and `None` when nothing matches.
fn pick<'a>(
    sel: &Selector,
    pool: &[&'a Entry],
    text: &str,
    intent: Intent,
    allow_multiple_fuzzy: bool,
) -> Option<Result<&'a Entry>> {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let tl = t.to_lowercase();
    let tiers: [&dyn Fn(&Entry) -> bool; 3] = [
        &|e| e.node.name == t,
        &|e| e.node.name.to_lowercase() == tl,
        &|e| {
            e.node.name.to_lowercase().contains(&tl)
                || (intent == Intent::Find
                    && !e.node.secure
                    && e.node.value.as_deref().is_some_and(|v| v.to_lowercase().contains(&tl)))
        },
    ];
    let preferred: &[&str] = match intent {
        Intent::Tap => INTERACTIVE,
        Intent::Type => EDITABLE,
        Intent::Find => &[],
    };
    let prefer_passes: &[bool] = if preferred.is_empty() { &[false] } else { &[true, false] };
    for &only_preferred in prefer_passes {
        for (tier, matches) in tiers.iter().enumerate() {
            let hits: Vec<&Entry> = pool
                .iter()
                .copied()
                .filter(|e| matches(e) && (!only_preferred || preferred.contains(&e.node.role.as_str())))
                .collect();
            match hits.as_slice() {
                [] => continue,
                [first, ..] if tier < 2 || hits.len() == 1 || allow_multiple_fuzzy => {
                    return Some(Ok(first));
                }
                _ => {
                    return Some(Err(Error::Ambiguous {
                        selector: sel.to_string(),
                        candidates: hits.iter().take(5).map(|e| line(e, false)).collect(),
                    }));
                }
            }
        }
    }
    None
}

fn line(e: &Entry, indent: bool) -> String {
    let pad = if indent { "  ".repeat(e.depth) } else { String::new() };
    let name = if e.node.name.is_empty() {
        String::new()
    } else {
        format!(" {:?}", e.node.name)
    };
    format!("{pad}{} {}{name}{}", e.r#ref, e.node.role, attrs(&e.node))
}

fn attrs(n: &Node) -> String {
    let mut s = String::new();
    if n.secure {
        s.push_str(" secure"); // never carries a value (PRD §15)
    } else if let Some(v) = &n.value
        && (EDITABLE.contains(&n.role.as_str()) || !v.is_empty())
    {
        s.push_str(&format!(" value={:?}", clip(v, MAX_VALUE)));
    }
    for (on, label) in [
        (n.disabled, " disabled"),
        (n.checked, " checked"),
        (n.selected, " selected"),
        (n.expanded, " expanded"),
    ] {
        if on {
            s.push_str(label);
        }
    }
    s
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Changes from `prev` to `next` as `+`/`-`/`~` lines. `None` means "send the full snapshot":
/// the screen changed or the diff is too long to be useful (PRD §9.2).
pub fn diff(prev: &Snapshot, next: &Snapshot) -> Option<Vec<String>> {
    if prev.title != next.title || prev.location != next.location {
        return None;
    }
    let old: HashMap<&str, &Entry> = prev.entries.iter().map(|e| (e.r#ref.as_str(), e)).collect();
    let new: HashMap<&str, &Entry> = next.entries.iter().map(|e| (e.r#ref.as_str(), e)).collect();
    let mut lines = Vec::new();
    for e in &next.entries {
        match old.get(e.r#ref.as_str()) {
            None => lines.push(format!("+ {}", line(e, false))),
            Some(o) => {
                let (a, b) = (attrs(&o.node), attrs(&e.node));
                if a != b {
                    let show = |s: &str| if s.is_empty() { " (none)".to_string() } else { s.to_string() };
                    let name = if e.node.name.is_empty() { String::new() } else { format!(" {:?}", e.node.name) };
                    lines.push(format!("~ {} {}{name}{} ->{}", e.r#ref, e.node.role, show(&a), show(&b)));
                }
            }
        }
    }
    for e in &prev.entries {
        if !new.contains_key(e.r#ref.as_str()) {
            lines.push(format!("- {}", line(e, false)));
        }
    }
    (lines.len() <= MAX_DIFF_LINES).then_some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(role: &str, name: &str) -> Node {
        Node {
            role: role.into(),
            name: name.into(),
            ..Node::default()
        }
    }

    fn with(mut node: Node, children: Vec<Node>) -> Node {
        node.children = children;
        node
    }

    fn login_tree(email: &str) -> Vec<Node> {
        let mut field = n("textfield", "Email");
        field.value = Some(email.into());
        let mut pass = n("textfield", "Password");
        pass.secure = true;
        pass.value = Some("hunter2".into());
        vec![with(
            n("generic", ""),
            vec![
                with(n("heading", "Masuk ke Akun"), vec![n("text", "Masuk ke Akun")]),
                with(n("labeltext", ""), vec![n("text", "Email")]),
                field,
                with(n("labeltext", ""), vec![n("text", "Password")]),
                pass,
                with(n("button", "Masuk"), vec![with(n("generic", ""), vec![n("text", "Masuk")])]),
                n("image", ""),
                n("link", "Lupa password?"),
            ],
        )]
    }

    #[test]
    fn prunes_and_renders_compactly() {
        let mut refs = RefRegistry::default();
        let s = Snapshot::build("Login", "http://x/login", login_tree(""), &mut refs);
        assert_eq!(
            s.render(None, DEFAULT_MAX_LINES),
            "screen \"Login\" (http://x/login)\n\
             e1 heading \"Masuk ke Akun\"\n\
             e2 textfield \"Email\" value=\"\"\n\
             e3 textfield \"Password\" secure\n\
             e4 button \"Masuk\"\n\
             e5 link \"Lupa password?\""
        );
    }

    #[test]
    fn refs_are_stable_and_diff_is_small() {
        let mut refs = RefRegistry::default();
        let a = Snapshot::build("Login", "u", login_tree(""), &mut refs);
        let mut tree = login_tree("a@b.com");
        tree[0].children.push(n("text", "Email atau password salah"));
        let b = Snapshot::build("Login", "u", tree, &mut refs);
        assert_eq!(
            diff(&a, &b),
            Some(vec![
                "~ e2 textfield \"Email\" value=\"\" -> value=\"a@b.com\"".to_string(),
                "+ e6 text \"Email atau password salah\"".to_string(),
            ])
        );
        let c = Snapshot::build("Dashboard", "u", vec![], &mut refs);
        assert_eq!(diff(&b, &c), None);
    }

    #[test]
    fn resolves_by_intent_and_reports_ambiguity() {
        let mut refs = RefRegistry::default();
        let mut tree = login_tree("");
        tree.push(n("text", "Email"));
        let s = Snapshot::build("Login", "u", tree, &mut refs);
        let text = |t: &str| Selector::Text(t.into());
        assert_eq!(s.resolve(&text("Email"), Intent::Type).map(|e| e.r#ref.as_str()).ok(), Some("e2"));
        assert_eq!(s.resolve(&text("masuk"), Intent::Tap).map(|e| e.r#ref.as_str()).ok(), Some("e4"));
        assert_eq!(s.resolve(&Selector::Ref("e5".into()), Intent::Tap).map(|e| e.node.role.as_str()).ok(), Some("link"));
        assert!(matches!(s.resolve(&text("Daftar"), Intent::Tap), Err(Error::NotFound(_))));
        // "as" fuzzily matches several interactive nodes: refuse to guess.
        assert!(matches!(s.resolve(&text("as"), Intent::Tap), Err(Error::Ambiguous { .. })));
        // Secure values are never matched or shown.
        assert!(s.resolve(&text("hunter2"), Intent::Find).is_err());
        assert!(!s.render(None, 120).contains("hunter2"));
    }

    #[test]
    fn caps_lines() {
        let mut refs = RefRegistry::default();
        let tree = (0..10).map(|i| n("button", &format!("b{i}"))).collect();
        let s = Snapshot::build("t", "u", tree, &mut refs);
        let out = s.render(None, 3);
        assert_eq!(out.lines().count(), 5);
        assert!(out.ends_with("... +7 nodes (scroll or snapshot(filter:) to see more)"));
    }
}
