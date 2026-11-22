//! QA (v0.6 Low 7): the Speed row of the release card uses plain words ("Main content shown",
//! "Layout shift"); the technical names LCP and CLS are only in the row's tooltip.
//! `speedTerms()` of `ui/app.js`, run in Node. Skips without `node`.

const APP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ui/app.js");

const HARNESS: &str = r#"
const src = require("fs").readFileSync(process.argv[1], "utf8");
const terms = src.split("\n").find((l) => l.startsWith("const SPEED_TERMS = "));
if (!terms) throw new Error("no SPEED_TERMS");
const i = src.indexOf("function speedTerms(");
if (i < 0) throw new Error("no speedTerms");
let d = 0, body = "";
for (let k = src.indexOf("{", i); k < src.length; k++) {
  if (src[k] === "{") d++;
  else if (src[k] === "}" && --d === 0) { body = src.slice(i, k + 1); break; }
}
const speedTerms = new Function(terms + ";" + body + ";return speedTerms;")();
console.log(JSON.stringify([
  speedTerms("Loads in 1.2 s (no change) · Main content shown 1.3 s (no change) · Layout shift 0.00 (no change)"),
  speedTerms("2 console errors"),
]));
"#;

#[test]
fn the_speed_row_explains_lcp_and_cls_in_a_tooltip() {
    let Ok(out) = std::process::Command::new("node").arg("-e").arg(HARNESS).arg(APP).output() else {
        return eprintln!("skipping: no node");
    };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got: Vec<Option<String>> = serde_json::from_slice(&out.stdout).expect("json");
    let tip = got[0].as_deref().expect("a tooltip on the Speed row");
    assert!(tip.contains("Main content shown = LCP (Largest Contentful Paint)") && tip.contains("Layout shift = CLS (Cumulative Layout Shift)"), "{tip}");
    assert_eq!(got[1], None, "other rows get no tooltip");
    let app = std::fs::read_to_string(APP).expect("app.js");
    assert!(app.contains(r#"h("span", { class: "vdetail", title: speedTerms(e.detail) }"#), "the release card uses it");
}
