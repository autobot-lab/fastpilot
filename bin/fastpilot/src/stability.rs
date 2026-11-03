//! `fastpilot stability <flow.yaml> --runs N`: the flow's pass rate over repeated runs in fresh
//! sessions, with the statistics a stability claim needs (PRD §13, G6).

use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use fastpilot_core::{RunOptions, SessionRegistry, check_flow, run_flow};
use fastpilot_mcp::PlatformLauncher;
use fastpilot_web::LaunchOptions;

/// 95% Wilson score upper bound of the failure rate for `fails` out of `n`.
pub fn failure_upper_bound(fails: usize, n: usize) -> f64 {
    if n == 0 {
        return 1.0;
    }
    let (n, p, z) = (n as f64, fails as f64 / n as f64, 1.96_f64);
    let z2 = z * z;
    (p + z2 / (2.0 * n) + z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt()) / (1.0 + z2 / n)
}

/// τ-bench pass^k: chance that k independent runs all pass, estimated from c passes of n.
pub fn pass_hat_k(c: usize, n: usize, k: usize) -> f64 {
    if k > n {
        return f64::NAN;
    }
    (0..k).map(|i| (c.saturating_sub(i)) as f64 / (n - i) as f64).product()
}

pub async fn run(path: &str, runs: usize, web: LaunchOptions) -> Result<(), Box<dyn Error + Send + Sync>> {
    let yaml = std::fs::read_to_string(path)?;
    let flow = check_flow(&yaml, &[]).map_err(|errs| errs.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))?;
    let launcher = PlatformLauncher::new(web);
    let mut passes = 0;
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut times = Vec::with_capacity(runs);
    for i in 1..=runs {
        // Fresh registry per run: the flow starts its own sessions, nothing leaks between runs.
        let mut reg = SessionRegistry::default();
        let report = run_flow(&mut reg, &launcher, &flow, &RunOptions::default()).await;
        times.push(report.elapsed);
        let verdict = if report.passed() {
            passes += 1;
            "ok".to_owned()
        } else {
            let why = report
                .records
                .iter()
                .find_map(|r| r.error.as_ref().map(|e| format!("step {} {}: {} [{}]", r.index, r.label, e.reason(), e.category())))
                .unwrap_or_else(|| "incomplete".into());
            *reasons.entry(why.clone()).or_default() += 1;
            format!("FAIL {why}")
        };
        println!("run {i:>3}/{runs}  {:>7.2}s  {}", report.elapsed.as_secs_f64(), fastpilot_core::redact(&verdict));
    }
    let fails = runs - passes;
    times.sort();
    let p50 = times.get(times.len() / 2).copied().unwrap_or(Duration::ZERO);
    println!("\n{passes}/{runs} passed ({:.1}%), p50 {:.2}s", 100.0 * passes as f64 / runs as f64, p50.as_secs_f64());
    for k in [1, 3, 8] {
        if k <= runs {
            println!("pass^{k}: {:.3}", pass_hat_k(passes, runs, k));
        }
    }
    let needed = (1..10_000).find(|&n| failure_upper_bound(0, n) < 0.01).unwrap_or(0);
    println!(
        "failure rate <= {:.1}% (95% Wilson upper bound); a <=1% claim needs >= {needed} runs without failure",
        100.0 * failure_upper_bound(fails, runs)
    );
    for (why, n) in reasons {
        println!("  {n}x {}", fastpilot_core::redact(&why));
    }
    if fails > 0 {
        return Err(format!("{fails} of {runs} runs failed").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics() {
        // 20/20 only bounds the failure rate at ~16%; it takes 381 clean runs to bound it below 1%.
        assert!((failure_upper_bound(0, 20) - 0.161).abs() < 0.001);
        assert!(failure_upper_bound(0, 381) < 0.01 && failure_upper_bound(0, 380) >= 0.01);
        assert_eq!(pass_hat_k(10, 10, 3), 1.0);
        assert!((pass_hat_k(8, 10, 2) - 56.0 / 90.0).abs() < 1e-9);
    }
}
