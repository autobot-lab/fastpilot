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

#[cfg(test)]
mod tests {
    use super::*;

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
