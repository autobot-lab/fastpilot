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
