//! E2E test suite runtime baseline (bd-tjsl, Wave 2).
//!
//! Runs `cargo test -p e2e-tests --no-fail-fast` N times, captures per-run
//! wall-clock timing, validates variance across runs, and persists a baseline
//! JSON for regression detection in future gate runs.
//!
//! Env thresholds:
//!   E2E_MAX_SECS         — max runtime per run in seconds (default 600)
//!   E2E_MAX_VARIANCE_PCT — max allowed % spread between fastest/slowest run (default 20)

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::report::ScenarioResult;

// ── Thresholds ────────────────────────────────────────────────────────────────

const DEFAULT_E2E_MAX_SECS: f64 = 600.0;
const DEFAULT_E2E_MAX_VARIANCE_PCT: f64 = 20.0;

// ── Baseline persistence ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2eBaseline {
    /// Average runtime across runs that established this baseline (seconds).
    pub baseline_secs: f64,
    /// ISO-8601 timestamp when the baseline was recorded.
    pub recorded_at: String,
    /// Git SHA at time of recording.
    pub git_sha: String,
}

fn baseline_path(reports_dir: &Path) -> PathBuf {
    reports_dir.join("e2e-baseline.json")
}

fn load_baseline(reports_dir: &Path) -> Option<E2eBaseline> {
    let data = std::fs::read_to_string(baseline_path(reports_dir)).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_baseline(reports_dir: &Path, baseline: &E2eBaseline) -> Result<()> {
    std::fs::create_dir_all(reports_dir)?;
    let json = serde_json::to_string_pretty(baseline)?;
    std::fs::write(baseline_path(reports_dir), json)?;
    Ok(())
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run the E2E test suite `runs` times, produce timing metrics, and compare
/// variance. Persists a new baseline at the end.
pub async fn run_e2e_bench(runs: u32, reports_dir: &Path) -> Result<ScenarioResult> {
    let max_ceiling = read_env_f64("E2E_MAX_SECS", DEFAULT_E2E_MAX_SECS);
    let max_variance_pct = read_env_f64("E2E_MAX_VARIANCE_PCT", DEFAULT_E2E_MAX_VARIANCE_PCT);

    info!(
        "e2e-bench: {} iteration(s), ceiling={:.0}s variance_threshold={:.0}%",
        runs, max_ceiling, max_variance_pct
    );

    let mut run_durations: Vec<f64> = Vec::with_capacity(runs as usize);
    let mut all_tests_passed = true;

    for i in 1..=runs {
        info!("e2e-bench: starting run {}/{}", i, runs);
        let (duration_secs, exit_ok) =
            run_cargo_test().await.context(format!("e2e-bench run {}", i))?;
        info!(
            "e2e-bench: run {}/{} finished in {:.1}s (success={})",
            i, runs, duration_secs, exit_ok
        );
        if !exit_ok {
            all_tests_passed = false;
        }
        run_durations.push(duration_secs);
    }

    // ── Statistical summary ───────────────────────────────────────────────────

    let avg_secs = run_durations.iter().sum::<f64>() / run_durations.len() as f64;
    let min_secs = run_durations.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_secs = run_durations.iter().cloned().fold(0.0_f64, f64::max);
    let variance_pct = if min_secs > 0.0 {
        (max_secs - min_secs) / min_secs * 100.0
    } else {
        0.0
    };

    // ── Baseline comparison ───────────────────────────────────────────────────

    let prior = load_baseline(reports_dir);
    let regression_pct = prior
        .as_ref()
        .map(|b| (avg_secs - b.baseline_secs) / b.baseline_secs.max(1.0) * 100.0);

    if let Some(pct) = regression_pct {
        let direction = if pct >= 0.0 { "slower" } else { "faster" };
        info!(
            "e2e-bench: {:.1}% {} than prior baseline ({:.1}s vs {:.1}s)",
            pct.abs(),
            direction,
            avg_secs,
            prior.as_ref().unwrap().baseline_secs,
        );
    } else {
        info!("e2e-bench: no prior baseline — establishing new one");
    }

    // Save updated baseline.
    let git_sha = current_git_sha();
    let new_baseline = E2eBaseline {
        baseline_secs: avg_secs,
        recorded_at: chrono::Utc::now().to_rfc3339(),
        git_sha: git_sha.clone(),
    };
    if let Err(e) = save_baseline(reports_dir, &new_baseline) {
        warn!("e2e-bench: could not save baseline: {}", e);
    } else {
        info!("e2e-bench: baseline updated ({:.1}s)", avg_secs);
    }

    // ── Threshold enforcement ─────────────────────────────────────────────────

    let mut violations: Vec<String> = Vec::new();

    if !all_tests_passed {
        violations.push(
            "one or more e2e test runs exited non-zero — suite has failures".to_string(),
        );
    }
    for (i, &secs) in run_durations.iter().enumerate() {
        if secs > max_ceiling {
            violations.push(format!(
                "run {} runtime {:.1}s exceeds ceiling {:.1}s",
                i + 1,
                secs,
                max_ceiling
            ));
        }
    }
    if runs >= 2 && variance_pct > max_variance_pct {
        violations.push(format!(
            "variance {:.1}% across {} runs exceeds threshold {:.1}%",
            variance_pct, runs, max_variance_pct
        ));
    }

    // ── Metrics JSON ──────────────────────────────────────────────────────────

    let mut metrics = serde_json::json!({
        "runs": runs,
        "run_durations_secs": run_durations,
        "avg_secs": avg_secs,
        "min_secs": min_secs,
        "max_secs": max_secs,
        "variance_pct": variance_pct,
        "ceiling_secs": max_ceiling,
        "max_variance_pct_threshold": max_variance_pct,
        "all_tests_passed": all_tests_passed,
        "git_sha": git_sha,
    });

    if let Some(pct) = regression_pct {
        metrics["regression_vs_baseline_pct"] = serde_json::json!(pct);
    }
    if let Some(ref b) = prior {
        metrics["prior_baseline_secs"] = serde_json::json!(b.baseline_secs);
        metrics["prior_baseline_git_sha"] = serde_json::json!(b.git_sha);
    }

    info!(
        "e2e-bench: avg={:.1}s min={:.1}s max={:.1}s variance={:.1}% — {}",
        avg_secs,
        min_secs,
        max_secs,
        variance_pct,
        if violations.is_empty() { "PASS" } else { "FAIL" }
    );

    Ok(ScenarioResult {
        name: "e2e_bench".to_string(),
        passed: violations.is_empty(),
        metrics,
        threshold_violations: violations,
        notes: None,
    })
}

/// Return a dry-run placeholder — does not execute cargo test.
pub fn e2e_dry_run() -> ScenarioResult {
    ScenarioResult {
        name: "e2e_bench".to_string(),
        passed: true,
        metrics: serde_json::json!({
            "runs": 0,
            "run_durations_secs": [],
            "avg_secs": 0.0,
        }),
        threshold_violations: vec![],
        notes: Some("dry-run: cargo test not executed".to_string()),
    }
}

// ── Cargo test subprocess ─────────────────────────────────────────────────────

/// Spawn `cargo test -p e2e-tests --no-fail-fast` and return (elapsed_secs, success).
///
/// stdout/stderr are inherited so test output flows to the terminal in real time.
async fn run_cargo_test() -> Result<(f64, bool)> {
    let start = Instant::now();

    let status = tokio::process::Command::new("cargo")
        .args(["test", "-p", "e2e-tests", "--no-fail-fast", "--", "--nocapture"])
        .status()
        .await
        .context("failed to spawn `cargo test -p e2e-tests`")?;

    Ok((start.elapsed().as_secs_f64(), status.success()))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn read_env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn current_git_sha() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
