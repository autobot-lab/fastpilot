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

    // tap -> auto-wait -> snapshot -> diff, toggling a checkbox so every iteration changes state.
    let checkbox = Selector::Text("Ingat saya".into());
    let mut before = page.snapshot().await?;
    let mut tap_diff = Vec::with_capacity(n);
    let mut sample = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        page.tap(&checkbox).await?;
        page.wait_idle(Duration::from_secs(3)).await?;
        let after = page.snapshot().await?;
        let d = diff(&before, &after).unwrap_or_default();
        tap_diff.push(t.elapsed());
        sample = d;
        before = after;
    }

    println!("fastpilot bench (web, {n} iterations, demo login page: {lines} snapshot lines)\n");
    println!("{:<22} {:>8} {:>8} {:>8} {:>5}  target", "metric (ms)", "min", "p50", "p95", "n");
    Stats(vec![launch]).row("chrome launch", None);
    Stats(start[..1].to_vec()).row("session start (cold)", None);
    Stats(start[1..].to_vec()).row("session start (warm)", Some(Duration::from_secs(1)));
    Stats(vec![open]).row("open page", None);
    Stats(snap).row("snapshot", Some(Duration::from_millis(50)));
    Stats(tap_diff).row("tap -> diff", Some(Duration::from_millis(200)));
    println!("\nlast diff: {}", sample.join(" | "));
    Ok(())
}
