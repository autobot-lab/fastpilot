//! `fastpilot bench`: driver latency against the embedded demo login page (PRD §14).

use std::error::Error;
use std::time::{Duration, Instant};

use base64::Engine;
use fastpilot_core::{Driver, Selector, diff};
use fastpilot_web::{Browser, LaunchOptions};

const DEMO: &str = include_str!("../../../examples/demo/login.html");

struct Stats(Vec<Duration>);

impl Stats {
    fn pct(&self, p: f64) -> Duration {
        let mut v = self.0.clone();
        v.sort();
        let i = ((v.len() as f64 - 1.0) * p).round() as usize;
        v.get(i).copied().unwrap_or_default()
    }

    fn row(&self, name: &str, target: Option<Duration>) {
        let ms = |d: Duration| format!("{:.1}", d.as_secs_f64() * 1000.0);
        let verdict = match target {
            Some(t) if self.pct(0.5) < t => format!("< {}ms ok", t.as_millis()),
            Some(t) => format!("< {}ms MISS", t.as_millis()),
            None => String::new(),
        };
        println!(
            "{name:<22} {:>8} {:>8} {:>8} {:>5}  {verdict}",
            ms(self.pct(0.0)),
            ms(self.pct(0.5)),
            ms(self.pct(0.95)),
            self.0.len()
        );
    }
}

async fn timed<T, E>(f: impl Future<Output = Result<T, E>>) -> Result<(T, Duration), E> {
    let t = Instant::now();
    let v = f.await?;
    Ok((v, t.elapsed()))
}

/// tap -> auto-wait -> snapshot -> diff on a checkbox, so every iteration changes state.
async fn tap_diff(d: &dyn Driver, n: usize) -> Result<(Vec<Duration>, Vec<String>), Box<dyn Error + Send + Sync>> {
    let checkbox = Selector::Text("Ingat saya".into());
    let mut before = d.snapshot().await?;
    let mut times = Vec::with_capacity(n);
    let mut sample = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        d.tap(&checkbox).await?;
        let _ = d.wait_idle(Duration::from_secs(3)).await;
        let after = d.snapshot().await?;
        sample = diff(&before, &after).unwrap_or_default();
        times.push(t.elapsed());
        before = after;
    }
    Ok((times, sample))
}

fn header(what: &str, n: usize, lines: usize) {
    println!("fastpilot bench ({what}, {n} iterations, demo login: {lines} snapshot lines)\n");
    println!("{:<22} {:>8} {:>8} {:>8} {:>5}  target", "metric (ms)", "min", "p50", "p95", "n");
}

/// Needs a booted device with the demo app (`examples/demo-android`) installed.
pub async fn android(n: usize) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (d, start) = timed(fastpilot_android::AndroidDriver::start(None)).await?;
    let ((), open) = timed(d.launch_app("com.fastpilot.demo/.LoginActivity")).await?;
    let _ = d.wait_idle(Duration::from_secs(3)).await;
    let mut snap = Vec::with_capacity(n);
    let mut lines = 0;
    for _ in 0..n {
        let (s, t) = timed(d.snapshot()).await?;
        lines = s.entries.len();
        snap.push(t);
    }
    let (taps, sample) = tap_diff(&d, n).await?;
    header("android", n, lines);
    Stats(vec![start]).row("session start", Some(Duration::from_secs(2)));
    Stats(vec![open]).row("launch app", None);
    Stats(snap).row("snapshot", Some(Duration::from_millis(100)));
    Stats(taps).row("tap -> diff", Some(Duration::from_millis(200)));
    println!("\nlast diff: {}", sample.join(" | "));
    Ok(())
}

pub async fn run(opts: LaunchOptions, n: usize) -> Result<(), Box<dyn Error + Send + Sync>> {
    let url = format!(
        "data:text/html;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(DEMO)
    );
    let (browser, launch) = timed(Browser::launch(opts)).await?;
    let mut start = Vec::new();
    let mut pages = Vec::new();
    for _ in 0..3 {
        let (p, d) = timed(browser.new_page()).await?;
        start.push(d);
        pages.push(p);
    }
    let page = pages.swap_remove(0);
    let ((), open) = timed(page.open(&url)).await?;

    let mut snap = Vec::with_capacity(n);
    let mut lines = 0;
    for _ in 0..n {
        let (s, d) = timed(page.snapshot()).await?;
        lines = s.entries.len();
        snap.push(d);
    }

    let (taps, sample) = tap_diff(&page, n).await?;
    header("web", n, lines);
    Stats(vec![launch]).row("chrome launch", None);
    Stats(start[..1].to_vec()).row("session start (cold)", None);
    Stats(start[1..].to_vec()).row("session start (warm)", Some(Duration::from_secs(1)));
    Stats(vec![open]).row("open page", None);
    Stats(snap).row("snapshot", Some(Duration::from_millis(50)));
    Stats(taps).row("tap -> diff", Some(Duration::from_millis(200)));
    println!("\nlast diff: {}", sample.join(" | "));
    Ok(())
}

/// Token cost of what the AI reads: count_tokens when credentials are set, else the stated
/// approximation (`fastpilot_llm::approx_tokens`).
struct Counter(Option<fastpilot_llm::Claude>);

impl Counter {
    fn name(&self) -> String {
        match &self.0 {
            Some(_) => format!("count_tokens API ({})", fastpilot_llm::MODEL_EXPLORE),
            None => "approximation (no ANTHROPIC_API_KEY): letter runs 1 per 5, digit runs 1 per 3, other chars 1, ±20%".into(),
        }
    }

    async fn count(&self, text: &str) -> Result<u64, Box<dyn Error + Send + Sync>> {
        Ok(match &self.0 {
            Some(c) => c.count_tokens(fastpilot_llm::MODEL_EXPLORE, text).await?,
            None => fastpilot_llm::approx_tokens(text) as u64,
        })
    }
}

/// Width and height from a JPEG's SOF segment.
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            return None;
        }
        let marker = b[i + 1];
        let len = usize::from(b[i + 2]) << 8 | usize::from(b[i + 3]);
        if matches!(marker, 0xC0..=0xC3) {
            let h = u32::from(b[i + 5]) << 8 | u32::from(b[i + 6]);
            let w = u32::from(b[i + 7]) << 8 | u32::from(b[i + 8]);
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// `fastpilot bench tokens [web|android|ios]`: tokens per snapshot, per one-step diff and per
/// full login `run_flow` on the demo app, next to what a screenshot would cost.
pub async fn tokens(platform: &str, web: LaunchOptions) -> Result<(), Box<dyn Error + Send + Sync>> {
    use fastpilot_core::{RunOptions, SessionRegistry, check_flow, run_flow};
    let demo = std::env::temp_dir().join("fastpilot-bench-login.html");
    std::fs::write(&demo, DEMO)?;
    let (setup, login, tap) = match platform {
        "web" => (
            format!("- session: {{ start: t, platform: web, target: \"{}\" }}\n", demo.display()),
            "- inputText: { into: \"Email\", text: \"a@b.com\" }\n- inputText: { into: \"Password\", text: \"rahasia123\" }\n- tapOn: \"Masuk\"\n- assertVisible: \"Budi\"\n",
            "- tapOn: \"Ingat saya\"\n",
        ),
        "android" | "ios" => (
            format!(
                "- session: {{ start: t, platform: {platform} }}\n- launchApp: {}\n",
                if platform == "android" { "com.fastpilot.demo/.LoginActivity" } else { "com.fastpilot.demo" }
            ),
            "- inputText: { into: \"Email\", text: \"a@b.com\" }\n- inputText: { into: \"Password\", text: \"rahasia123\" }\n- tapOn: \"Masuk\"\n- assertVisible: \"Selamat datang, Budi\"\n",
            "- tapOn: \"Ingat saya\"\n",
        ),
        other => return Err(format!("bench tokens: unknown platform `{other}` (web, android, ios)").into()),
    };
    let counter = Counter(fastpilot_llm::Claude::from_env());
    let launcher = fastpilot_mcp::PlatformLauncher::new(web);
    let mut reg = SessionRegistry::default();
    let run = |yaml: String| check_flow(&yaml, &[]).map_err(|e| e.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"));
    let opts = RunOptions::default();
    let started = run_flow(&mut reg, &launcher, &run(setup)?, &opts).await;
    if !started.passed() {
        return Err(format!("setup failed:\n{started}").into());
    }
    let d = reg.shared("t").ok_or("no session")?;
    let snap = format!("[t] {}", d.snapshot().await?.render(None, 120));
    let one = run_flow(&mut reg, &launcher, &check_flow(tap, &["t".into()]).map_err(|e| format!("{e:?}"))?, &opts).await;
    let one = one.to_string();
    let full = run_flow(&mut reg, &launcher, &check_flow(login, &["t".into()]).map_err(|e| format!("{e:?}"))?, &opts).await;
    let steps = full.total;
    let full = full.to_string();
    let shot = d.screenshot().await?;

    println!("fastpilot bench tokens ({platform}, demo login)\ncounter: {}\n", counter.name());
    println!("{:<34} {:>7} {:>8}", "what the AI reads", "chars", "tokens");
    let row = |what: &str, chars: usize, tokens: u64| println!("{what:<34} {chars:>7} {tokens:>8}");
    let snap_t = counter.count(&snap).await?;
    row("snapshot (login screen)", snap.len(), snap_t);
    row("run_flow, 1 step (tap → diff)", one.len(), counter.count(&one).await?);
    let full_t = counter.count(&full).await?;
    row(&format!("run_flow, login ({steps} steps)"), full.len(), full_t);
    row("  per step", full.len() / steps.max(1), full_t / steps.max(1) as u64);
    if let Some((w, h)) = jpeg_size(&shot) {
        row(&format!("screenshot {w}x{h} (w*h/750)"), 0, fastpilot_llm::image_tokens(w, h));
    }
    println!("\n1-step response:\n{one}");
    Ok(())
}
