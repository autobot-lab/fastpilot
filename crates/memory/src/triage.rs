//! Failure taxonomy and run statistics (PRD §13, G6): classify a failed run, quantify a test's
//! stability over its recorded history, and the suite-level guards (retry budget, breaker).

use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

use crate::PathMemory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    /// The app is not as the test expects (bug, changed UI, crash): a human must look.
    Product,
    /// Browser, device or transport failed: rerun elsewhere / fix the environment.
    Infra,
    /// The flow the AI wrote is wrong or uses something unsupported.
    Agent,
    /// Passes and fails without a change: passed on retry, or a mixed recent history.
    Flaky,
    /// The tool acted too early: an action reported success but changed nothing on screen,
    /// and the next check failed (e.g. a tap swallowed by an animation). Not a product bug:
    /// rerun, add `waitForAnimationToEnd`, or report it against fastpilot.
    Timing,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Infra => "infra",
            Self::Agent => "agent",
            Self::Flaky => "flaky",
            Self::Timing => "timing",
        }
    }
}

/// Classify a failed run from its first failure's category (`Error::category`), whether a
/// retry of the same test passed, and the test's history before this run.
///
/// Rule: passed on retry → flaky. A product failure on a test whose recent history flips
/// (pass/fail at least twice over ≥ 5 runs, failing under half the time) → flaky as well,
/// since the same steps have passed without a product change. Otherwise the category decides;
/// a run reported failed with no failing step is a product failure (the AI judged the screen).
/// `timing` (a check failed right after an action that changed nothing on screen) is a tool
/// timing failure, never a product one.
pub fn classify(category: Option<&str>, passed_on_retry: bool, history: Option<&PathMemory>) -> Class {
    if passed_on_retry {
        return Class::Flaky;
    }
    match category {
        Some("infra") => Class::Infra,
        Some("agent") => Class::Agent,
        Some("timing") => Class::Timing,
        _ => {
            let flips = history.map(|m| {
                let s = m.stats();
                s.runs >= 5 && s.flips >= 2 && s.pass_rate > 0.5
            });
            if flips == Some(true) { Class::Flaky } else { Class::Product }
        }
    }
}

/// 95% Wilson score interval of a pass rate.
pub fn wilson(passes: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (n, p, z) = (n as f64, passes as f64 / n as f64, 1.96_f64);
    let z2 = z * z;
    let center = p + z2 / (2.0 * n);
    let margin = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    (((center - margin) / (1.0 + z2 / n)).max(0.0), ((center + margin) / (1.0 + z2 / n)).min(1.0))
}

/// τ-bench pass^k: chance that k independent runs all pass, from c passes of n (unbiased).
pub fn pass_hat_k(c: usize, n: usize, k: usize) -> Option<f64> {
    (k >= 1 && k <= n).then(|| (0..k).map(|i| c.saturating_sub(i) as f64 / (n - i) as f64).product())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub runs: usize,
    pub passes: usize,
    pub pass_rate: f64,
    /// 95% Wilson interval of the pass rate.
    pub ci: (f64, f64),
    /// pass^3 (all of 3 runs pass), when there are ≥ 3 runs.
    pub pass_k3: Option<f64>,
    /// Outcome changes between consecutive runs.
    pub flips: usize,
    /// flips / (runs - 1): 0 for a stable test (always passing or always failing).
    pub flake_rate: f64,
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.runs == 0 {
            return f.write_str("no recorded runs");
        }
        write!(
            f,
            "pass {}/{} ({:.0}%, 95% CI {:.0}–{:.0}%)",
            self.passes,
            self.runs,
            100.0 * self.pass_rate,
            100.0 * self.ci.0,
            100.0 * self.ci.1
        )?;
        if let Some(k) = self.pass_k3 {
            write!(f, ", pass^3 {k:.2}")?;
        }
        write!(f, ", flake rate {:.0}%", 100.0 * self.flake_rate)
    }
}

impl PathMemory {
    /// Statistics over the recorded window (last 20 runs).
    pub fn stats(&self) -> Stats {
        let runs = self.history.len();
        let passes = self.history.iter().filter(|p| **p).count();
        let flips = self.history.windows(2).filter(|w| w[0] != w[1]).count();
        Stats {
            runs,
            passes,
            pass_rate: if runs == 0 { 0.0 } else { passes as f64 / runs as f64 },
            ci: wilson(passes, runs),
            pass_k3: pass_hat_k(passes, runs, 3),
            flips,
            flake_rate: if runs < 2 { 0.0 } else { flips as f64 / (runs - 1) as f64 },
        }
    }
}

/// Suite-wide budget of reruns for failed tests (`FASTPILOT_RETRY_BUDGET`, default 3), so
/// retries cannot hide a broken build by turning every failure into a rerun.
#[derive(Debug)]
pub struct RetryBudget(AtomicU32);

impl RetryBudget {
    pub fn new(n: u32) -> Self {
        Self(AtomicU32::new(n))
    }

    pub fn from_env() -> Self {
        Self::new(std::env::var("FASTPILOT_RETRY_BUDGET").ok().and_then(|v| v.parse().ok()).unwrap_or(3))
    }

    /// Spend one retry; false when the budget is used up.
    pub fn take(&self) -> bool {
        // A compare-exchange loop: `fetch_update` is deprecated on newer toolchains.
        let mut n = self.0.load(Ordering::SeqCst);
        loop {
            let Some(next) = n.checked_sub(1) else { return false };
            match self.0.compare_exchange_weak(n, next, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return true,
                Err(now) => n = now,
            }
        }
    }

    pub fn left(&self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Opens after `threshold` consecutive infra failures (`FASTPILOT_BREAKER`, default 3): the
/// environment is down, so the remaining tests are skipped as infra instead of each timing
/// out. Any non-infra result closes it again.
#[derive(Debug)]
pub struct Breaker {
    threshold: u32,
    streak: AtomicU32,
}

impl Breaker {
    pub fn new(threshold: u32) -> Self {
        Self { threshold: threshold.max(1), streak: AtomicU32::new(0) }
    }

    pub fn from_env() -> Self {
        Self::new(std::env::var("FASTPILOT_BREAKER").ok().and_then(|v| v.parse().ok()).unwrap_or(3))
    }

    /// `None` for a pass.
    pub fn record(&self, class: Option<Class>) {
        if class == Some(Class::Infra) {
            self.streak.fetch_add(1, Ordering::SeqCst);
        } else {
            self.streak.store(0, Ordering::SeqCst);
        }
    }

    pub fn is_open(&self) -> bool {
        self.streak.load(Ordering::SeqCst) >= self.threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(h: &[bool]) -> PathMemory {
        PathMemory { history: h.to_vec(), ..PathMemory::default() }
    }

    #[test]
    fn statistics_and_classification() {
        let s = history(&[true; 20]).stats();
        assert_eq!((s.runs, s.passes, s.flips, s.flake_rate), (20, 20, 0, 0.0));
        assert!((s.ci.0 - 0.839).abs() < 0.001 && (s.ci.1 - 1.0).abs() < 1e-9, "{s}");
        assert_eq!(s.pass_k3, Some(1.0));
        let mixed = history(&[true, false, true, true, false, true, true, true]);
        let m = mixed.stats();
        assert_eq!((m.flips, m.passes), (4, 6));
        assert_eq!(m.to_string(), "pass 6/8 (75%, 95% CI 41–93%), pass^3 0.36, flake rate 57%");

        assert_eq!(classify(Some("product"), true, None), Class::Flaky);
        assert_eq!(classify(Some("infra"), false, None), Class::Infra);
        assert_eq!(classify(Some("agent"), false, Some(&mixed)), Class::Agent);
        assert_eq!(classify(Some("product"), false, Some(&mixed)), Class::Flaky);
        assert_eq!(classify(Some("product"), false, Some(&history(&[true; 10]))), Class::Product);
        assert_eq!(classify(None, false, None), Class::Product);
        // Issue #7: a check failing after an action with no visible effect is not a product bug.
        assert_eq!(classify(Some("timing"), false, None), Class::Timing);
        assert_eq!(classify(Some("timing"), false, Some(&history(&[true; 10]))), Class::Timing);
        assert_eq!(classify(Some("timing"), true, None), Class::Flaky);
        assert_eq!(Class::Timing.as_str(), "timing");
        assert_eq!(pass_hat_k(1, 2, 3), None);
    }

    #[test]
    fn budget_and_breaker() {
        let b = RetryBudget::new(2);
        assert!(b.take() && b.take() && !b.take());
        assert_eq!(b.left(), 0);
        let br = Breaker::new(2);
        br.record(Some(Class::Infra));
        assert!(!br.is_open());
        br.record(Some(Class::Infra));
        assert!(br.is_open());
        br.record(None);
        assert!(!br.is_open());
    }
}
