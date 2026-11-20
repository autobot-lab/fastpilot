//! Recordings as files and as HTML: an mp4 plays in `<video>`; web screencast frames become a
//! self-contained filmstrip player (no video encoder needed).

use base64::Engine;

use crate::Recording;

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// `&` `<` `>` `"` escaped, for text and attribute values.
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

impl Recording {
    /// File extension and contents: `mp4`, or a standalone `html` player for frames.
    pub fn file(&self, title: &str) -> (&'static str, Vec<u8>) {
        match self {
            Self::Video { data, .. } => ("mp4", data.clone()),
            Self::Frames(frames) => ("html", filmstrip(title, frames).into_bytes()),
        }
    }

    /// HTML fragment that plays the recording inside a larger page.
    pub fn embed(&self, title: &str) -> String {
        match self {
            Self::Video { mime, data } => {
                format!("<video controls preload=\"metadata\" style=\"max-width:100%;max-height:640px\" src=\"data:{mime};base64,{}\"></video>", b64(data))
            }
            Self::Frames(frames) => format!(
                "<iframe title=\"{}\" style=\"width:100%;height:640px;border:0\" srcdoc=\"{}\"></iframe>",
                escape_html(title),
                escape_html(&filmstrip(title, frames))
            ),
        }
    }
}

/// One HTML file: the frames with their timestamps, a slider and play/pause at real speed.
pub fn filmstrip(title: &str, frames: &[(f64, Vec<u8>)]) -> String {
    let data: Vec<String> = frames.iter().map(|(t, j)| format!("[{t:.3},\"data:image/jpeg;base64,{}\"]", b64(j))).collect();
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>{title}</title><style>
body {{ font: 13px system-ui, sans-serif; margin: 8px; background: #fff; color: #111; }}
img {{ max-width: 100%; max-height: 560px; border: 1px solid #ccc; display: block; }}
.bar {{ display: flex; gap: 8px; align-items: center; margin-top: 6px; }} input {{ flex: 1; }}
</style></head><body><img id="f" alt="frame"><div class="bar"><button id="p">play</button>
<input id="s" type="range" min="0" value="0"><span id="t"></span></div><script>
const F = [{frames}];
const img = document.getElementById('f'), s = document.getElementById('s'), t = document.getElementById('t'), p = document.getElementById('p');
s.max = Math.max(F.length - 1, 0);
let timer = null;
function show(i) {{ if (!F.length) {{ t.textContent = 'no frames'; return; }} s.value = i; img.src = F[i][1]; t.textContent = F[i][0].toFixed(2) + ' s · ' + (i + 1) + '/' + F.length; }}
function step() {{ const i = +s.value; if (i + 1 >= F.length) {{ stop(); return; }} show(i + 1); timer = setTimeout(step, Math.max(16, (F[i + 1][0] - F[i][0]) * 1000)); }}
function stop() {{ clearTimeout(timer); timer = null; p.textContent = 'play'; }}
p.onclick = () => {{ if (timer) stop(); else {{ if (+s.value + 1 >= F.length) show(0); p.textContent = 'pause'; step(); }} }};
s.oninput = () => {{ stop(); show(+s.value); }};
show(0);
</script></body></html>
"#,
        title = escape_html(title),
        frames = data.join(",")
    )
}

/// One step on a recording's timeline: seconds since the recording started.
pub struct Mark {
    pub at: f64,
    pub flow: usize,
    pub ok: bool,
    pub label: String,
}

/// One flow (one run_flow call) on a recording's timeline, in seconds since it started.
pub struct Span {
    pub flow: usize,
    pub from: f64,
    pub to: f64,
    pub ok: bool,
}

/// The recording with its steps on it, inside an `<iframe srcdoc>` (the report page itself stays
/// script-free): a player, a bar with one marker per step (red where a step failed; a click seeks
/// there) and one button per flow that plays only that flow's part. Frames (web) play at real
/// speed; a video (android, ios) uses `<video>`.
pub fn timeline(title: &str, rec: &Recording, marks: &[Mark], spans: &[Span]) -> String {
    // JSON inside <script>: `<` escaped so a label can never close the tag.
    let js = |v: serde_json::Value| v.to_string().replace('<', "\\u003c");
    let marks = js(serde_json::Value::Array(
        marks.iter().map(|m| serde_json::json!({ "t": m.at, "flow": m.flow, "ok": m.ok, "label": m.label })).collect(),
    ));
    let spans = js(serde_json::Value::Array(
        spans.iter().map(|s| serde_json::json!({ "flow": s.flow, "from": s.from, "to": s.to, "ok": s.ok })).collect(),
    ));
    let (media, frames) = match rec {
        Recording::Video { mime, data } => (format!("<video id=\"v\" preload=\"metadata\" src=\"data:{mime};base64,{}\"></video>", b64(data)), "null".to_owned()),
        Recording::Frames(f) => (
            "<img id=\"v\" alt=\"frame\">".to_owned(),
            format!("[{}]", f.iter().map(|(t, j)| format!("[{t:.3},\"data:image/jpeg;base64,{}\"]", b64(j))).collect::<Vec<_>>().join(",")),
        ),
    };
    let doc = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>{title}</title><style>
body {{ font: 13px system-ui, sans-serif; margin: 8px; background: #fff; color: #111; }}
#v {{ max-width: 100%; max-height: 520px; border: 1px solid #ccc; display: block; }}
.track {{ position: relative; height: 18px; margin: 8px 0; background: #eef0f3; border-radius: 3px; }}
.track i {{ position: absolute; top: 0; bottom: 0; width: 1px; background: #94a3b8; }}
.track b {{ position: absolute; top: 2px; width: 10px; height: 14px; margin-left: -5px; border: 0; border-radius: 2px; cursor: pointer; background: #1a7f37; }}
.track b.bad {{ background: #cf222e; width: 14px; margin-left: -7px; }}
.bar {{ display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }} .bar button.bad {{ color: #cf222e; }}
</style></head><body>{media}<div class="track" id="track"><i id="head"></i></div>
<div class="bar"><button id="p">play</button><span id="t"></span><span id="flows"></span></div><script>
const M = {marks}, S = {spans}, F = {frames};
const v = document.getElementById('v'), track = document.getElementById('track'), head = document.getElementById('head');
const t = document.getElementById('t'), p = document.getElementById('p');
let stopAt = null, timer = null, now = 0;
const total = () => F ? (F.length ? F[F.length - 1][0] : 0) : (v.duration || 0);
const cur = () => F ? now : v.currentTime;
function draw() {{ const d = total() || 1; head.style.left = (100 * cur() / d) + '%'; t.textContent = cur().toFixed(1) + ' / ' + total().toFixed(1) + ' s'; }}
function frameAt(x) {{ let i = 0; while (i + 1 < F.length && F[i + 1][0] <= x) i++; return i; }}
function seek(x) {{ if (F) {{ now = Math.max(0, x); if (F.length) v.src = F[frameAt(now)][1]; }} else v.currentTime = Math.max(0, x); draw(); }}
function pause() {{ if (F) {{ clearTimeout(timer); timer = null; }} else v.pause(); stopAt = null; p.textContent = 'play'; }}
function tick() {{ const i = frameAt(now); if (i + 1 >= F.length || (stopAt !== null && F[i + 1][0] > stopAt)) {{ pause(); return; }}
  const wait = (F[i + 1][0] - now) * 1000; now = F[i + 1][0]; v.src = F[i + 1][1]; draw(); timer = setTimeout(tick, Math.max(16, wait)); }}
function play(until) {{ stopAt = until; p.textContent = 'pause'; if (F) {{ clearTimeout(timer); timer = setTimeout(tick, 16); }} else v.play(); }}
if (!F) {{ v.addEventListener('timeupdate', () => {{ draw(); if (stopAt !== null && v.currentTime >= stopAt) pause(); }}); v.addEventListener('loadedmetadata', layout); }}
p.onclick = () => (p.textContent === 'pause' ? pause() : play(null));
function layout() {{ const d = total() || 1;
  for (const m of M) {{ const b = document.createElement('b'); if (!m.ok) b.className = 'bad'; b.style.left = (100 * Math.min(m.t, d) / d) + '%';
    b.title = 'flow ' + m.flow + ': ' + (m.ok ? '✓ ' : '✗ ') + m.label; b.onclick = () => {{ pause(); seek(m.t); }}; track.appendChild(b); }}
  const fl = document.getElementById('flows');
  for (const s of S) {{ const b = document.createElement('button'); if (!s.ok) b.className = 'bad'; b.textContent = '▶ flow ' + s.flow + (s.ok ? ' ✓' : ' ✗');
    b.onclick = () => {{ pause(); seek(s.from); play(s.to); }}; fl.appendChild(b); }} draw(); }}
if (F) {{ seek(0); layout(); }}
</script></body></html>
"#,
        title = escape_html(title)
    );
    format!("<iframe title=\"{}\" style=\"width:100%;height:640px;border:0\" srcdoc=\"{}\"></iframe>", escape_html(title), escape_html(&doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E4: the steps sit on the recording's timeline, a failed step is red, each flow has its own
    /// segment, and a label cannot end the player's script.
    #[test]
    fn timeline_marks_steps_and_flows() {
        let frames = Recording::Frames(vec![(0.0, vec![0xff, 0xd8]), (1.0, vec![0xff, 0xd8]), (2.5, vec![0xff, 0xd9])]);
        let marks = [
            Mark { at: 0.2, flow: 1, ok: true, label: "tapOn \"Masuk\"".into() },
            Mark { at: 1.4, flow: 2, ok: false, label: "assertVisible </script><script>alert(1)</script>".into() },
        ];
        let spans = [Span { flow: 1, from: 0.0, to: 1.2, ok: true }, Span { flow: 2, from: 0.9, to: 2.5, ok: false }];
        let html = timeline("w <login>", &frames, &marks, &spans);
        assert!(html.starts_with("<iframe title=\"w &lt;login&gt;\""), "{html}");
        // Inside srcdoc everything is attribute-escaped: one level of &quot; for the JSON quotes.
        // Keys in JSON order (sorted): flow, label, ok, t / flow, from, ok, to.
        assert!(html.contains("{&quot;flow&quot;:1,&quot;label&quot;:&quot;tapOn \\&quot;Masuk\\&quot;&quot;,&quot;ok&quot;:true,&quot;t&quot;:0.2}"), "first step");
        assert!(html.contains("&quot;ok&quot;:false,&quot;t&quot;:1.4}"), "failed step");
        assert!(html.contains("{&quot;flow&quot;:2,&quot;from&quot;:0.9,&quot;ok&quot;:false,&quot;to&quot;:2.5}"), "flow 2 segment");
        assert!(html.contains(".track b.bad {{ background: #cf222e".replace("{{", "{").as_str()), "red marker style");
        assert!(!html.contains("&lt;/script&gt;&lt;script&gt;alert"), "a label must not close the script");
        assert!(html.contains("\\u003c/script&gt;"), "escaped as \\u003c");
        let video = Recording::Video { mime: "video/mp4", data: vec![1, 2, 3] };
        assert!(timeline("a", &video, &marks, &spans).contains("&lt;video id=&quot;v&quot;"));
    }

    #[test]
    fn files_and_embeds() {
        let frames = Recording::Frames(vec![(0.0, vec![0xff, 0xd8]), (0.5, vec![0xff, 0xd9])]);
        let (ext, html) = frames.file("login <web>");
        let html = String::from_utf8(html).expect("utf8");
        assert_eq!(ext, "html");
        assert!(html.contains("[0.500,\"data:image/jpeg;base64,/9k=\"]") && html.contains("login &lt;web&gt;"), "{html}");
        assert!(frames.embed("x").starts_with("<iframe") && !frames.embed("x").contains("<script>"));
        let video = Recording::Video { mime: "video/mp4", data: vec![1, 2, 3] };
        assert_eq!(video.file("x").0, "mp4");
        assert!(video.embed("x").contains("data:video/mp4;base64,AQID"));
    }
}
