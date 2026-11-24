//! QA B6 (real Claude, v0.4): the dashboard Chat showed Claude's `**bold**` and the closing ```
//! of a "Techniques used" block as raw text. `markdown()` of `ui/app.js`, run in Node with a tiny
//! DOM stand-in: fences become monospace blocks (also an unclosed one), headings and **bold** bold
//! lines, `code` code; HTML stays text. Skips without `node`.

const APP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ui/app.js");

const HARNESS: &str = r#"
const src = require("fs").readFileSync(process.argv[1], "utf8");
function Node() {}
const document = { createElement: (t) => Object.assign(Object.create(Node.prototype), { tag: t, kids: [], append(...k) { this.kids.push(...k); }, setAttribute() {}, addEventListener() {} }), createTextNode: (t) => Object.assign(Object.create(Node.prototype), { text: t }) };
const pick = (name) => {
  const i = src.indexOf("function " + name + "(");
  if (i < 0) throw new Error("no function " + name);
  let d = 0;
  for (let k = src.indexOf("{", i); k < src.length; k++) {
    if (src[k] === "{") d++;
    else if (src[k] === "}" && --d === 0) return src.slice(i, k + 1);
  }
};
const markdown = new Function("document", "Node", pick("h") + ";const code=(t)=>h('code',{},t);" + pick("inline") + ";" + pick("markdown") + ";return markdown;")(document, Node);
const show = (n) => (typeof n === "string" ? n : n.text !== undefined ? n.text : `<${n.tag}>${n.kids.map(show).join("")}</${n.tag}>`);
console.log(JSON.stringify(JSON.parse(process.argv[2]).map((t) => markdown(t).map(show).join(""))));
"#;

#[test]
fn chat_shows_claudes_markdown_tidily_and_never_as_html() {
    let cases = [
        "**Done.** 3 checks\n```\nTechniques used\nBVA · Age\n```\nNext: `run`",
        "## Result\nOpen block:\n```text\nline 1",
        "plain <b>not html</b> <img src=x onerror=alert(1)>",
    ];
    let Ok(out) = std::process::Command::new("node").arg("-e").arg(HARNESS).arg(APP).arg(serde_json::to_string(&cases).expect("json")).output() else {
        return eprintln!("skipping: no node");
    };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got: Vec<String> = serde_json::from_slice(&out.stdout).expect("json");
    // v0.7.1 FC4: paragraphs, headings and code blocks as their own elements.
    assert_eq!(got[0], "<p><b>Done.</b> 3 checks</p><pre><code>Techniques used\nBVA · Age</code></pre><p>Next: <code>run</code></p>", "no ``` or ** left");
    assert_eq!(got[1], "<h3>Result</h3><p>Open block:</p><pre><code>line 1</code></pre>", "an unclosed fence still a block");
    assert_eq!(got[2], "<p>plain <b>not html</b> <img src=x onerror=alert(1)></p>", "HTML stays text (one text node)");
}
