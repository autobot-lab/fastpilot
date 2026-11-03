//! System UI the app under test does not own (PRD §11.2 rule 6): runtime permission prompts,
//! crash / ANR dialogs, and the soft keyboard. Detected from the agent's raw window list,
//! by resource id, so it works in any device language.

use fastpilot_core::{Point, Rect, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Permission,
    Crash,
    Anr,
}

/// What to do with a permission prompt the flow is not interacting with
/// (`FASTPILOT_PERMISSIONS`: deny (default) | allow | keep).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Deny,
    Allow,
    Keep,
}

impl Policy {
    pub fn from_env() -> Self {
        match std::env::var("FASTPILOT_PERMISSIONS").as_deref() {
            Ok("allow") => Self::Allow,
            Ok("keep") => Self::Keep,
            _ => Self::Deny,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dialog {
    pub kind: Kind,
    /// Visible text, e.g. "Allow fastpilot demo to take pictures?" or "... keeps stopping".
    pub text: String,
    /// `(resource id, center)` of every node with an id.
    ids: Vec<(String, Point)>,
    /// Boxes of every node in the dialog, to tell whether a resolved target belongs to it.
    boxes: Vec<Rect>,
}

fn rect(v: &Value) -> Option<Rect> {
    let b = v.as_array()?;
    let n = |i: usize| b.get(i).and_then(Value::as_f64).unwrap_or(0.0);
    Some(Rect { x: n(0), y: n(1), w: n(2) - n(0), h: n(3) - n(1) })
}

fn walk(v: &Value, d: &mut Dialog) {
    if let Some(b) = rect(&v["bounds"]) {
        d.boxes.push(b);
        if let Some(id) = v["id"].as_str() {
            d.ids.push((id.to_owned(), b.center()));
        }
    }
    if let Some(t) = v["text"].as_str().filter(|t| !t.is_empty()) {
        if !d.text.is_empty() {
            d.text.push_str(" / ");
        }
        d.text.push_str(t);
    }
    for c in v["children"].as_array().into_iter().flatten() {
        walk(c, d);
    }
}

impl Dialog {
    fn has(&self, suffix: &str) -> bool {
        self.ids.iter().any(|(id, _)| id.ends_with(suffix))
    }

    fn button(&self, suffixes: &[&str]) -> Option<Point> {
        suffixes.iter().find_map(|s| self.ids.iter().find(|(id, _)| id.ends_with(s)).map(|(_, p)| *p))
    }

    /// Where to tap to apply `policy` to a permission prompt, or to close a crash/ANR dialog.
    pub fn dismiss(&self, policy: Policy) -> Option<Point> {
        match (self.kind, policy) {
            (Kind::Permission, Policy::Keep) => None,
            (Kind::Permission, Policy::Deny) => {
                self.button(&[":id/permission_deny_button", ":id/permission_deny_and_dont_ask_again_button"])
            }
            (Kind::Permission, Policy::Allow) => self.button(&[
                ":id/permission_allow_foreground_only_button",
                ":id/permission_allow_button",
                ":id/permission_allow_one_time_button",
            ]),
            // OEM builds (MIUI/HyperOS, One UI) may own the dialog under their own package.
            (Kind::Crash | Kind::Anr, _) => self.button(&[":id/aerr_close"]),
        }
    }

    pub fn owns(&self, b: &Rect) -> bool {
        self.boxes.iter().any(|x| x == b)
    }
}

/// The first system dialog on screen, if any.
pub fn dialog(snapshot: &Value) -> Option<Dialog> {
    let app = snapshot["package"].as_str().unwrap_or("");
    for root in snapshot["nodes"].as_array().into_iter().flatten() {
        let pkg = root["pkg"].as_str().unwrap_or("");
        let mut d = Dialog { kind: Kind::Permission, text: String::new(), ids: Vec::new(), boxes: Vec::new() };
        walk(root, &mut d);
        let permission = pkg.contains("permissioncontroller")
            || d.ids.iter().any(|(id, _)| id.contains(":id/permission_") && id.ends_with("_button"));
        if permission {
            return Some(d);
        }
        if d.has(":id/aerr_wait") {
            return Some(Dialog { kind: Kind::Anr, ..d });
        }
        if d.has(":id/aerr_close") {
            return Some(Dialog { kind: Kind::Crash, ..d });
        }
        // Unknown windows of other packages: logged so OEM dialogs can be captured as fixtures.
        if !pkg.is_empty() && pkg != app && pkg != "android" {
            let ids: Vec<&str> = d.ids.iter().map(|(id, _)| id.as_str()).take(12).collect();
            tracing::debug!(pkg, ?ids, text = %d.text, "unrecognized system window");
        }
    }
    None
}

/// Soft keyboard box, when shown.
pub fn ime(snapshot: &Value) -> Option<Rect> {
    rect(&snapshot["ime"]).filter(|r| r.w > 0.0 && r.h > 0.0)
}

/// `Process: <pkg>, PID ...` crash entries in `logcat -b crash` output: the exception line.
pub fn crash_in_log(log: &str, pkg: &str) -> Option<String> {
    let marker = format!("Process: {pkg},");
    let mut lines = log.lines().skip_while(|l| !l.contains(&marker)).skip(1);
    let why = lines.next().map(|l| l.split_once("): ").or_else(|| l.split_once(": ")).map_or(l, |(_, m)| m).trim().to_owned());
    log.contains(&marker).then(|| why.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_system_dialogs() {
        let perm = json!({ "nodes": [
            { "pkg": "com.fastpilot.demo", "cls": "FrameLayout", "bounds": [0, 0, 720, 1280] },
            { "pkg": "com.google.android.permissioncontroller", "bounds": [40, 400, 680, 900], "children": [
                { "text": "Allow fastpilot demo to take pictures and record video?", "bounds": [60, 420, 660, 500] },
                { "id": "com.android.permissioncontroller:id/permission_allow_foreground_only_button", "text": "While using the app", "bounds": [60, 600, 660, 660] },
                { "id": "com.android.permissioncontroller:id/permission_deny_button", "text": "Don't allow", "bounds": [60, 800, 660, 860] }
            ]}
        ]});
        let d = dialog(&perm).expect("permission prompt");
        assert_eq!(d.kind, Kind::Permission);
        assert!(d.text.starts_with("Allow fastpilot demo"), "{}", d.text);
        assert_eq!(d.dismiss(Policy::Deny), Some(Point { x: 360.0, y: 830.0 }));
        assert_eq!(d.dismiss(Policy::Allow), Some(Point { x: 360.0, y: 630.0 }));
        assert_eq!(d.dismiss(Policy::Keep), None);
        assert!(d.owns(&Rect { x: 60.0, y: 800.0, w: 600.0, h: 60.0 }));
        assert!(!d.owns(&Rect { x: 0.0, y: 0.0, w: 720.0, h: 1280.0 }));

        let anr = json!({ "nodes": [{ "pkg": "android", "bounds": [40, 400, 680, 900], "children": [
            { "text": "fastpilot demo isn't responding", "bounds": [60, 420, 660, 500] },
            { "id": "android:id/aerr_close", "text": "Close app", "bounds": [60, 600, 660, 660] },
            { "id": "android:id/aerr_wait", "text": "Wait", "bounds": [60, 700, 660, 760] }
        ]}]});
        assert_eq!(dialog(&anr).map(|d| d.kind), Some(Kind::Anr));
        let crash = json!({ "nodes": [{ "pkg": "android", "bounds": [0, 0, 10, 10], "children": [
            { "id": "android:id/aerr_close", "bounds": [0, 0, 10, 10] }
        ]}]});
        let miui = json!({ "nodes": [{ "pkg": "com.miui.securitycenter", "bounds": [0, 0, 10, 10], "children": [
            { "id": "com.miui.securitycenter:id/aerr_wait", "bounds": [0, 0, 10, 10] }
        ]}]});
        assert_eq!(dialog(&miui).map(|d| d.kind), Some(Kind::Anr), "suffix match covers OEM packages");
        let c = dialog(&crash).expect("crash");
        assert_eq!((c.kind, c.dismiss(Policy::Deny)), (Kind::Crash, Some(Point { x: 5.0, y: 5.0 })));
        assert!(dialog(&json!({ "nodes": [{ "pkg": "com.x", "bounds": [0, 0, 1, 1] }] })).is_none());
        assert_eq!(ime(&json!({ "ime": [0, 800, 720, 1280] })), Some(Rect { x: 0.0, y: 800.0, w: 720.0, h: 480.0 }));

        let log = "--------- beginning of crash\n11-03 10:00:00.000 E/AndroidRuntime( 123): FATAL EXCEPTION: main\n11-03 10:00:00.000 E/AndroidRuntime( 123): Process: com.fastpilot.demo, PID: 123\n11-03 10:00:00.000 E/AndroidRuntime( 123): java.lang.IllegalStateException: demo crash\n";
        assert_eq!(crash_in_log(log, "com.fastpilot.demo").as_deref(), Some("java.lang.IllegalStateException: demo crash"));
        assert_eq!(crash_in_log(log, "com.other"), None);
    }
}
