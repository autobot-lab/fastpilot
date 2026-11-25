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
const markdown = new Function("document", "Node", pick("h") + ";const code=(t)=>h('code',{},t);" + pick("inline") + ";" + pick("cells") + ";" + pick("markdown") + ";" + pick("rich") + ";" + pick("errLine") + ";return (t) => t.startsWith('red: ') ? errLine(t.slice(5)) : markdown(t);")(document, Node);
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

/// FastTest 8155 (F2): the demo result showed "Fill in the card form \\(stops before Pay\\)". A run
/// result escapes text from the run (`reports::md_escape`, Security L18/L22): Chat shows it as
/// typed, and page text still makes no row, cell, bold, link words or code of its own.
#[test]
fn escaped_run_text_shows_as_typed_and_makes_no_rows_or_links() {
    let cases = [
        "**All 3 demo tests passed.**\n\n| Test | Result |\n|---|---|\n| Fill in the card form \\(stops before Pay\\) | Passed |\n| a \\| b | Failed: \\*\\*x\\*\\* \\[c\\]\\(https://evil.example\\) \\`d\\` x\\_y\\_ C:\\\\ \\# |\n",
        "keep `a\\(b` and C:\\Users as they are",
    ];
    let Ok(out) = std::process::Command::new("node").arg("-e").arg(HARNESS).arg(APP).arg(serde_json::to_string(&cases).expect("json")).output() else {
        return eprintln!("skipping: no node");
    };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got: Vec<String> = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(
        got[0],
        "<p><b>All 3 demo tests passed.</b></p><div><table><thead><tr><th>Test</th><th>Result</th></tr></thead><tbody>\
         <tr><td>Fill in the card form (stops before Pay)</td><td>Passed</td></tr>\
         <tr><td>a | b</td><td>Failed: **x** [c](<a>https://evil.example</a>) `d` x_y_ C:\\ #</td></tr></tbody></table></div>",
        "no backslash shown; an escaped | stays in its cell; page text makes no bold, link words or code (a bare address stays itself)"
    );
    assert_eq!(got[1], "<p>keep <code>a\\(b</code> and C:\\Users as they are</p>", "code spans and other backslashes stay");
}

/// FastTest 8155 + design e1c: the F4 line is the red result line with its head in bold; any other
/// red line stays as it was (only `code` spans).
#[test]
fn the_f4_red_line_has_its_head_in_bold() {
    let cases = [
        "red: **Stopped at step 4:** no code was typed in 3 minutes. Say “run it again” when you have the code.",
        "red: Claude Code stopped: run `fastpilot doctor`",
        "red: **not closed",
    ];
    let Ok(out) = std::process::Command::new("node").arg("-e").arg(HARNESS).arg(APP).arg(serde_json::to_string(&cases).expect("json")).output() else {
        return eprintln!("skipping: no node");
    };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got: Vec<String> = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(got[0], "<b>Stopped at step 4:</b> no code was typed in 3 minutes. Say “run it again” when you have the code.");
    assert_eq!(got[1], "Claude Code stopped: run <code>fastpilot doctor</code>");
    assert_eq!(got[2], "**not closed");
}
