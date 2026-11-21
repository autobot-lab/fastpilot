//! FastTest's final report (v0.4): the Live header read "● 1 sessions · 0 running null Tests at
//! once". `livePill()` of `ui/app.js`, run in Node with a tiny DOM stand-in, says "1 session";
//! the header itself (no "null") is checked in Chrome by `ui_live_keyboard`. Skips without `node`.

const APP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ui/app.js");

const HARNESS: &str = r#"
const src = require("fs").readFileSync(process.argv[1], "utf8");
const document = { createElement: (t) => ({ tag: t, kids: [], append(...k) { this.kids.push(...k); }, setAttribute() {}, addEventListener() {} }), createTextNode: (t) => ({ text: t }) };
function Node() {}
const pick = (name) => {
  const i = src.indexOf("function " + name + "(");
  if (i < 0) throw new Error("no function " + name);
  let d = 0;
  for (let k = src.indexOf("{", i); k < src.length; k++) {
    if (src[k] === "{") d++;
    else if (src[k] === "}" && --d === 0) return src.slice(i, k + 1);
  }
};
const livePill = new Function("document", "Node", pick("h") + ";" + pick("livePill") + ";return livePill;")(document, Node);
const show = (n) => (typeof n === "string" ? n : n.text !== undefined ? n.text : n.kids.map(show).join(""));
const one = { flow: { finished: false, steps: [{ state: "done" }] } };
console.log(JSON.stringify([[one], [one, { flow: null }], []].map((s) => show(livePill(s)))));
"#;

#[test]
fn the_live_header_counts_one_session_as_one_session() {
    let Ok(out) = std::process::Command::new("node").arg("-e").arg(HARNESS).arg(APP).output() else {
        return eprintln!("skipping: no node");
    };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got: Vec<String> = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(got, ["● 1 session · 1 running", "● 2 sessions · 1 running", "○ No session is running"]);
}
