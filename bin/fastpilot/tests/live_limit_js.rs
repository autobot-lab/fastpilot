//! L062-12: the Live page's "Tests at once" list offered 1 to 10 while `max_parallel` allows 1 to
//! 20, so a limit of 15 showed as 1. `limitPicker()` of `ui/app.js`, run in Node with a tiny DOM
//! stand-in, offers every allowed value and selects the current one. Skips without `node`.

const APP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ui/app.js");
const UI: &str = include_str!("../src/ui.rs");

const HARNESS: &str = r#"
const src = require("fs").readFileSync(process.argv[1], "utf8");
function Node() {}
const el = (t) => Object.assign(Object.create(Node.prototype), { tag: t, kids: [], attrs: {}, append(...k) { this.kids.push(...k); }, setAttribute(k, v) { this.attrs[k] = v; }, addEventListener() {} });
const document = { createElement: el, createTextNode: (t) => ({ text: t }) };
const pick = (name) => {
  const i = src.indexOf("function " + name + "(");
  if (i < 0) throw new Error("no function " + name);
  let d = 0;
  for (let k = src.indexOf("{", i); k < src.length; k++) {
    if (src[k] === "{") d++;
    else if (src[k] === "}" && --d === 0) return src.slice(i, k + 1);
  }
};
const limitPicker = new Function("document", "Node", pick("h") + ";" + pick("limitPicker") + ";return limitPicker;")(document, Node);
const sel = (box) => box.kids.find((k) => k.tag === "select");
const opts = (data) => { const s = sel(limitPicker(data)); return [s.kids.length, s.kids.filter((o) => "selected" in o.attrs).map((o) => o.attrs.value)]; };
console.log(JSON.stringify([opts({ limit: 15, limit_max: 20 }), opts({ limit: 3, limit_max: 20 })]));
"#;

#[test]
fn tests_at_once_offers_every_allowed_value() {
    assert!(UI.contains(r#""limit_max": fastpilot_core::claim::MAX_PARALLEL"#), "the Live data says the most the setting allows");
    let Ok(out) = std::process::Command::new("node").arg("-e").arg(HARNESS).arg(APP).output() else {
        return eprintln!("skipping: no node");
    };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got: Vec<(usize, Vec<String>)> = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(got[0], (20, vec!["15".to_owned()]), "a limit of 15 shows as 15, out of 1 to 20");
    assert_eq!(got[1], (20, vec!["3".to_owned()]));
}
