//! The Chrome extension's background script (`extension-chrome/background.js`), run in Node with
//! the `chrome.*` and `WebSocket` APIs stubbed: what it connects to, and when it refuses.
//! Skips without `node`.

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../extension-chrome/background.js");

/// Node program: loads background.js with stubs, clicks the toolbar button once, prints JSON
/// `{ sockets, attached, badge }`. argv: script, native host reply (JSON or "throw"), options token.
const HARNESS: &str = r#"
const fs = require("fs"), vm = require("vm");
const [script, nativeReply, optionsToken] = process.argv.slice(1);
const out = { sockets: [], attached: 0, badge: "" };
const listeners = {};
const event = (name) => ({ addListener: (f) => { listeners[name] = f; } });
const chrome = {
  storage: { local: { get: async () => ({ port: 9230, token: optionsToken }) } },
  runtime: { sendNativeMessage: async () => { if (nativeReply === "throw") throw new Error("no host"); return JSON.parse(nativeReply); } },
  debugger: { attach: async () => { out.attached++; }, detach: async () => {}, sendCommand: async () => ({}), onEvent: event("event"), onDetach: event("detach") },
  action: { setBadgeText: ({ text }) => { out.badge = text; }, setBadgeBackgroundColor: () => {}, onClicked: event("click") },
};
class WebSocket { constructor(url) { out.sockets.push(url); this.readyState = 0; } send() {} close() {} }
WebSocket.OPEN = 1;
const ctx = vm.createContext({ chrome, WebSocket, console: { log() {}, warn() {}, error() {} }, setInterval: () => 0, clearInterval() {}, JSON, encodeURIComponent, String, Error });
vm.runInContext(fs.readFileSync(script, "utf8"), ctx);
(async () => { await listeners.click({ id: 7 }); await new Promise((r) => setTimeout(r, 20)); console.log(JSON.stringify(out)); })();
"#;

fn run(native: &str, options_token: &str) -> Option<serde_json::Value> {
    let out = std::process::Command::new("node").arg("-e").arg(HARNESS).arg(SCRIPT).arg(native).arg(options_token).output().ok()?;
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).ok()
}

/// Security Low #6 / P2 1d: with no token (native host unavailable, options empty) the extension
/// connected anyway and attached the debugger; any process on port 9230 could then drive the tab.
#[test]
fn extension_never_connects_without_a_token() {
    let Some(ok) = run(r#"{"token":"c0ffee"}"#, "") else {
        return eprintln!("skipping: no node");
    };
    assert_eq!(ok["sockets"], serde_json::json!(["ws://127.0.0.1:9230/extension?token=c0ffee"]), "{ok}");
    assert_eq!(ok["attached"], 1);
    // Native host down or without a token, options empty: no socket, no debugger, a KEY badge.
    for native in ["throw", r#"{"error":"no fastpilot relay is waiting on port 9230"}"#] {
        let none = run(native, "").expect("json");
        assert_eq!(none["sockets"], serde_json::json!([]), "{native}: {none}");
        assert_eq!(none["attached"], 0, "{native}: {none}");
        assert_eq!(none["badge"], "KEY", "{native}: {none}");
    }
    // The options' token (FASTPILOT_RELAY_TOKEN) is still the fallback.
    let fallback = run("throw", "abc").expect("json");
    assert_eq!(fallback["sockets"], serde_json::json!(["ws://127.0.0.1:9230/extension?token=abc"]), "{fallback}");
}
