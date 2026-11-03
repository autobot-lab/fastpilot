//! Compact text snapshots (PRD §9): prune, stable refs, render, diff, selector resolution.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::{Error, Result, Selector};

const INTERACTIVE: &[&str] = &[
    "button", "link", "textfield", "checkbox", "radio", "combobox", "listbox", "option",
    "menuitem", "tab", "switch", "slider", "spinbutton", "clickable",
];
const EDITABLE: &[&str] = &["textfield", "combobox", "spinbutton"];
/// Non-interactive roles that survive pruning; every other role is a container and gets folded.
const CONTENT: &[&str] = &["heading", "text", "image", "dialog", "alertdialog", "alert"];
const MAX_NAME: usize = 80;
const MAX_VALUE: usize = 40;
pub const DEFAULT_MAX_LINES: usize = 120;
const MAX_DIFF_LINES: usize = 40;

/// `"text"` with only `"`, `\` and control characters escaped: every other character, Arabic
/// harakat and other combining marks included, is shown as is, so a name copied from a
/// snapshot works as a selector.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Lowercase, without Arabic harakat / tatweel, for comparing names.
fn fold(s: &str) -> Vec<char> {
    s.to_lowercase().chars().filter(|c| !matches!(*c as u32, 0x064B..=0x065F | 0x0670 | 0x0640)).collect()
}

/// 1 - normalized Levenshtein distance (characters).
fn similarity(a: &[char], b: &[char]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f64 / a.len().max(b.len()) as f64
}

/// Element box in screen (mobile) or document (web) pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn center(&self) -> crate::Point {
        crate::Point { x: self.x + self.w / 2.0, y: self.y + self.h / 2.0 }
    }

    pub fn contains(&self, p: crate::Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }
}

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
    pub pressed: bool,
    pub invalid: bool,
    pub required: bool,
    pub readonly: bool,
    pub focused: bool,
    /// Driver-specific handle (CDP backendNodeId, Android node id, ...).
    pub handle: Option<i64>,
    pub bounds: Option<Rect>,
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

/// Per-session ref allocation. A node keeps its ref while its driver handle lives (so text
/// changes show up as `~`); a re-rendered node (new handle) inherits the ref of its
/// `hash(role, name, path)` key only if the previous holder is gone, so refs never silently
/// jump to a different element.
// ponytail: grows for the session's lifetime; evict unseen keys if sessions get very long.
#[derive(Debug, Default)]
pub struct RefRegistry {
    by_key: HashMap<u64, u32>,
    by_handle: HashMap<i64, u32>,
    holder: HashMap<u32, Option<i64>>,
    next: u32,
}

impl RefRegistry {
    fn assign(&mut self, items: &[(u64, Option<i64>)]) -> Vec<String> {
        let live: std::collections::HashSet<i64> = items.iter().filter_map(|(_, h)| *h).collect();
        let mut used = std::collections::HashSet::new();
        items
            .iter()
            .map(|&(key, handle)| {
                let by_handle = handle.and_then(|h| self.by_handle.get(&h).copied());
                let by_key = self.by_key.get(&key).copied().filter(|n| {
                    // Reuse a key's ref only when its previous holder left the screen.
                    !matches!(self.holder.get(n), Some(Some(h)) if live.contains(h) && Some(*h) != handle)
                });
                let n = match by_handle.or(by_key) {
                    Some(n) if !used.contains(&n) => n,
                    _ => {
                        self.next += 1;
                        self.next
                    }
                };
                used.insert(n);
                self.by_key.insert(key, n);
                if let Some(h) = handle {
                    self.by_handle.insert(h, n);
                }
                self.holder.insert(n, handle);
                format!("e{n}")
            })
            .collect()
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

/// A text node made only of digits and punctuation (`(`, `11`, `)`, `4.8`, `·`).
fn symbolic(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().count() <= 12 && !t.chars().any(char::is_alphabetic)
}

fn join_inline(a: &str, b: &str) -> String {
    let (a, b) = (a.trim_end(), b.trim_start());
    let tight = a.ends_with(['(', '[', '{', '#', '@', '/']) || b.starts_with([')', ']', '}', ',', '.', ':', ';', '%', '/']);
    if tight { format!("{a}{b}") } else { format!("{a} {b}") }
}

/// Sibling text runs split by markup (a rating as `text "("`, `text "11"`, `text ")"`) become
/// one node, when the pieces are digits/punctuation; worded texts stay separate.
fn merge_inline_text(kept: Vec<Pending>) -> Vec<Pending> {
    let mut out: Vec<Pending> = Vec::with_capacity(kept.len());
    for p in kept {
        if let Some(last) = out.last_mut()
            && last.node.role == "text"
            && p.node.role == "text"
            && last.depth == p.depth
            && last.node.children.is_empty()
            && (symbolic(&last.node.name) || symbolic(&p.node.name))
        {
            last.node.name = join_inline(&last.node.name, &p.node.name);
            if let (Some(a), Some(b)) = (last.node.bounds, p.node.bounds) {
                let (x, y) = (a.x.min(b.x), a.y.min(b.y));
                last.node.bounds = Some(Rect { x, y, w: (a.x + a.w).max(b.x + b.w) - x, h: (a.y + a.h).max(b.y + b.h) - y });
            }
            continue;
        }
        out.push(p);
    }
    out
}

/// Runs of this many same-role siblings or more are summarized in capped renders.
const RUN: usize = 12;

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
        let kept: Vec<Pending> = out.into_iter().zip(redundant).filter(|(_, r)| !r).map(|(p, _)| p).collect();
        let kept = merge_inline_text(kept);
        let ids = refs.assign(&kept.iter().map(|p| (p.key, p.node.handle)).collect::<Vec<_>>());
        let entries = kept
            .into_iter()
            .zip(ids)
            .map(|(p, r#ref)| Entry {
                r#ref,
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
        format!("screen {} ({})", quote(&self.title), self.location)
    }

    /// Render with an optional case-insensitive role/name filter and a hard line cap.
    pub fn render(&self, filter: Option<&str>, max_lines: usize) -> String {
        self.render_with(filter, max_lines, false)
    }

    /// Few labeled nodes on a non-empty screen (Compose/Flutter/canvas without semantics,
    /// PRD §9.1): selectors by text will not work, coordinates must.
    pub fn low_label_coverage(&self) -> bool {
        let labeled = self.entries.iter().filter(|e| !e.node.name.is_empty()).count();
        self.entries.len() >= 3 && labeled < 3 && labeled * 2 < self.entries.len()
    }

    /// `coords` appends ` @x,y wxh` (PRD §9.1 `snapshot(coords: true)`); forced on when
    /// label coverage is low.
    pub fn render_with(&self, filter: Option<&str>, max_lines: usize, coords: bool) -> String {
        let low = self.low_label_coverage();
        let coords = coords || low;
        let filter = filter.map(str::to_lowercase);
        // A filter keeps matches plus their ancestors, so a match is shown in context (a "find").
        let mut keep = vec![filter.is_none(); self.entries.len()];
        if let Some(f) = filter.as_deref() {
            for (i, e) in self.entries.iter().enumerate() {
                if !(e.node.role.contains(f) || e.node.name.to_lowercase().contains(f)) {
                    continue;
                }
                keep[i] = true;
                let mut depth = e.depth;
                for j in (0..i).rev() {
                    if depth == 0 {
                        break;
                    }
                    if self.entries[j].depth < depth {
                        keep[j] = true;
                        depth = self.entries[j].depth;
                    }
                }
            }
        }
        let shown: Vec<&Entry> = self.entries.iter().zip(&keep).filter(|(_, k)| **k).map(|(e, _)| e).collect();
        let render = |e: &Entry| {
            let l = line(e, true);
            match (coords, e.node.bounds) {
                (true, Some(b)) => format!("{l} @{:.0},{:.0} {:.0}x{:.0}", b.x, b.y, b.w, b.h),
                _ => l,
            }
        };
        let mut lines: Vec<String> = Vec::with_capacity(shown.len());
        let mut i = 0;
        while i < shown.len() {
            let e = shown[i];
            let run = shown[i..].iter().take_while(|o| o.depth == e.depth && o.node.role == e.node.role).count();
            // A long strip of same-role siblings (a 30-day date picker): first and last few,
            // with how to list them all. Uncapped renders (diffs, `full`) keep every line.
            if run >= RUN && max_lines != usize::MAX && filter.is_none() {
                lines.extend(shown[i..i + 5].iter().map(|e| render(e)));
                let hidden = &shown[i + 5..i + run - 1];
                let (first, last) = (hidden[0], hidden[hidden.len() - 1]);
                lines.push(format!(
                    "{}… +{} {} ({}..{}; snapshot(filter: \"{}\") lists them)",
                    "  ".repeat(e.depth),
                    hidden.len(),
                    e.node.role,
                    first.r#ref,
                    last.r#ref,
                    e.node.role
                ));
                lines.push(render(shown[i + run - 1]));
            } else {
                lines.extend(shown[i..i + run].iter().map(|e| render(e)));
            }
            i += run;
        }
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
        if low {
            out.push_str("\nlow_label_coverage: few labeled nodes; tap by ref or { point: [x, y] } (coords shown), or take a screenshot (annotate: som)");
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
    /// The closest matches for a selector that failed: same role first, then name similarity
    /// (normalized edit distance, substring overlap; case and harakat ignored). Selectors
    /// without a name fall back to elements usable for the intent.
    pub fn candidates(&self, sel: &Selector, intent: Intent) -> Vec<String> {
        let usable = |e: &Entry| match intent {
            Intent::Tap => INTERACTIVE.contains(&e.node.role.as_str()),
            Intent::Type => EDITABLE.contains(&e.node.role.as_str()),
            Intent::Find => true,
        };
        let (want_role, want_name) = match sel {
            Selector::Text(t) => (None, Some(t.as_str())),
            Selector::Role { role, name } => (Some(role.as_str()), name.as_deref()),
            _ => (None, None),
        };
        let Some(want) = want_name.map(fold).filter(|w| !w.is_empty()) else {
            return self.entries.iter().filter(|e| usable(e) && want_role.is_none_or(|r| e.node.role == r)).take(5).map(|e| line(e, false)).collect();
        };
        let want_s: String = want.iter().collect();
        let mut scored: Vec<(f64, usize, &Entry)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.node.name.is_empty())
            .filter_map(|(i, e)| {
                let name = fold(&e.node.name);
                let name_s: String = name.iter().collect();
                let mut score = similarity(&want, &name);
                if name_s.contains(&want_s) || want_s.contains(&name_s) {
                    score = score.max(0.6);
                }
                if score < 0.3 {
                    return None;
                }
                if want_role.is_some_and(|r| e.node.role == r) {
                    score += 1.0;
                }
                if usable(e) {
                    score += 0.1;
                }
                Some((score, i, e))
            })
            .collect();
        if scored.is_empty() {
            // Nothing close: still offer what the step could act on.
            return self.entries.iter().filter(|e| usable(e)).take(5).map(|e| line(e, false)).collect();
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().take(5).map(|(_, _, e)| line(e, false)).collect()
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
    node.name = node.name.split_whitespace().collect::<Vec<_>>().join(" ");
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
    for (tier, matches) in tiers.iter().enumerate() {
        for &only_preferred in prefer_passes {
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
    format!("{pad}{} {}{}{}", e.r#ref, e.node.role, quoted_name(&e.node), attrs(&e.node))
}

fn quoted_name(n: &Node) -> String {
    if n.name.is_empty() { String::new() } else { format!(" {}", quote(&clip(&n.name, MAX_NAME))) }
}

fn attrs(n: &Node) -> String {
    let mut s = String::new();
    if n.secure {
        s.push_str(" secure"); // never carries a value (PRD §15)
    } else if let Some(v) = &n.value
        && (EDITABLE.contains(&n.role.as_str()) || !v.is_empty())
    {
        s.push_str(&format!(" value={}", quote(&clip(v, MAX_VALUE))));
    }
    for (on, label) in [
        (n.disabled, " disabled"),
        (n.checked, " checked"),
        (n.selected, " selected"),
        (n.expanded, " expanded"),
        (n.pressed, " pressed"),
        (n.invalid, " invalid"),
        (n.required, " required"),
        (n.readonly, " readonly"),
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

/// Everything a `~` line reports, compared unclipped so a change past the render cap still shows.
fn state(n: &Node) -> (&str, Option<&str>, bool, [bool; 8]) {
    let value = if n.secure { None } else { n.value.as_deref() };
    let flags = [n.disabled, n.checked, n.selected, n.expanded, n.pressed, n.invalid, n.required, n.readonly];
    (&n.name, value, n.secure, flags)
}

/// Changes from `prev` to `next` as `+`/`-`/`~` lines. `None` means "send the full snapshot":
/// the screen changed or the diff is too long to be useful (PRD §9.2).
pub fn diff(prev: &Snapshot, next: &Snapshot) -> Option<Vec<String>> {
    if prev.location != next.location {
        return None;
    }
    let old: HashMap<&str, &Entry> = prev.entries.iter().map(|e| (e.r#ref.as_str(), e)).collect();
    let new: HashMap<&str, &Entry> = next.entries.iter().map(|e| (e.r#ref.as_str(), e)).collect();
    let mut lines = Vec::new();
    for e in &next.entries {
        match old.get(e.r#ref.as_str()) {
            None => lines.push(format!("+ {}", line(e, false))),
            Some(o) if state(&o.node) != state(&e.node) => {
                let show = |s: String| if s.is_empty() { " (none)".to_string() } else { s };
                let (a, b) = (attrs(&o.node), attrs(&e.node));
                lines.push(if o.node.name != e.node.name {
                    format!("~ {} {}{}{a} ->{}{b}", e.r#ref, e.node.role, quoted_name(&o.node), quoted_name(&e.node))
                } else {
                    format!("~ {} {}{}{} ->{}", e.r#ref, e.node.role, quoted_name(&e.node), show(a), show(b))
                });
            }
            Some(_) => {}
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
        let c = Snapshot::build("Dashboard", "u/home", vec![], &mut refs);
        assert_eq!(diff(&b, &c), None);
    }

    #[test]
    fn refs_follow_handles_and_never_jump_to_another_element() {
        let row = |h: i64, name: &str| Node { handle: Some(h), ..n("button", name) };
        let mut refs = RefRegistry::default();
        let a = Snapshot::build("t", "u", vec![row(1, "Hapus"), row(2, "Hapus")], &mut refs);
        assert_eq!(a.entries.iter().map(|e| e.r#ref.as_str()).collect::<Vec<_>>(), ["e1", "e2"]);
        // Row 1 deleted: row 2 keeps e2, and e1 is gone instead of re-binding to row 2.
        let b = Snapshot::build("t", "u", vec![row(2, "Hapus")], &mut refs);
        assert_eq!(b.entries[0].r#ref, "e2");
        // Re-rendered node (new handle, same role/name/path) inherits the ref; renames are `~`.
        let c = Snapshot::build("t", "u", vec![row(9, "Hapus")], &mut refs);
        assert_eq!(c.entries[0].r#ref, "e2");
        let d = Snapshot::build("t", "u", vec![row(9, "Dihapus")], &mut refs);
        assert_eq!(diff(&c, &d), Some(vec!["~ e2 button \"Hapus\" -> \"Dihapus\"".to_string()]));
    }

    #[test]
    fn long_names_match_in_full_and_clip_only_when_rendered() {
        let msg = "Sesi Anda telah berakhir karena tidak ada aktivitas selama tiga puluh menit, silakan masuk kembali";
        let mut refs = RefRegistry::default();
        let s = Snapshot::build("t", "u", vec![n("text", msg)], &mut refs);
        assert!(s.resolve(&Selector::Text(msg.into()), Intent::Find).is_ok());
        assert!(s.render(None, 120).contains("…"));
        // Exact non-interactive beats fuzzy interactive.
        let s = Snapshot::build("t", "u", vec![n("link", "Account Settings"), n("text", "Settings")], &mut refs);
        assert_eq!(s.resolve(&Selector::Text("Settings".into()), Intent::Tap).map(|e| e.node.role.as_str()).ok(), Some("text"));
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
    fn filter_shows_matches_with_their_ancestors() {
        let mut refs = RefRegistry::default();
        let dialog = with(n("dialog", "Hapus?"), vec![n("text", "Yakin?"), n("button", "Ya"), n("button", "Batal")]);
        let s = Snapshot::build("t", "u", vec![n("button", "Menu"), dialog], &mut refs);
        assert_eq!(s.render(Some("batal"), 120), "screen \"t\" (u)\ne2 dialog \"Hapus?\"\n  e5 button \"Batal\"");
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

    #[test]
    fn arabic_names_render_as_is_and_rank_candidates() {
        let mut refs = RefRegistry::default();
        let n = |role: &str, name: &str| Node { role: role.into(), name: name.into(), ..Node::default() };
        let nodes = vec![
            n("link", "كن شريكًا"),
            n("link", "الرئيسية"),
            n("link", "الاستشاريين"),
            n("button", "تسجيل الدخول"),
            n("text", "say \"hi\"\n"),
        ];
        let snap = Snapshot::build("موتمينا", "https://motmaina.com/ar", nodes, &mut refs);
        let text = snap.render(None, 50);
        assert!(text.contains("e1 link \"كن شريكًا\""), "{text}");
        assert!(!text.contains("\\u{"), "no escaped harakat: {text}");
        assert!(text.contains("\"say \\\"hi\\\"\\n\""), "quotes and control chars escaped: {text}");
        // A name copied from the rendered line still resolves.
        let copied = text.lines().nth(1).and_then(|l| l.split('"').nth(1)).expect("name");
        assert!(snap.resolve(&Selector::Text(copied.into()), Intent::Tap).is_ok());
        // A near miss suggests the closest link first, not the first five elements.
        let c = snap.candidates(&Selector::Role { role: "link".into(), name: Some("المستشارين".into()) }, Intent::Tap);
        assert!(c[0].contains("الاستشاريين"), "{c:?}");
        // Harakat are ignored when comparing.
        let c = snap.candidates(&Selector::Text("كن شريكا".into()), Intent::Find);
        assert!(c[0].contains("كن شريكًا"), "{c:?}");
    }

    #[test]
    fn merges_inline_text_and_summarizes_long_runs() {
        let mut refs = RefRegistry::default();
        let n = |role: &str, name: &str| Node { role: role.into(), name: name.into(), ..Node::default() };
        let mut nodes = vec![n("text", "4.8"), n("text", "("), n("text", "11"), n("text", ")"), n("text", "Selamat datang,"), n("text", "Budi")];
        nodes.extend((1..=30).map(|d| n("button", &format!("{d} Nov"))));
        nodes.push(n("link", "Berikutnya"));
        let snap = Snapshot::build("Jadwal", "u", nodes, &mut refs);
        let text = snap.render(None, 120);
        assert!(text.contains("text \"4.8 (11)\""), "{text}");
        assert!(text.contains("text \"Selamat datang,\"") && text.contains("text \"Budi\""), "worded texts stay apart: {text}");
        assert!(text.contains("button \"5 Nov\"") && !text.contains("\"6 Nov\"") && text.contains("button \"30 Nov\""), "{text}");
        assert!(text.contains("… +24 button (") && text.contains("link \"Berikutnya\""), "{text}");
        // Every node is still there for selectors and uncapped renders.
        assert!(snap.resolve(&Selector::Text("17 Nov".into()), Intent::Tap).is_ok());
        assert!(snap.render(None, usize::MAX).contains("\"17 Nov\""));
    }
}
