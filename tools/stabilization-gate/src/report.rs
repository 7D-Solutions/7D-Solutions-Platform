//! Report schema and artifact writer for the stabilization gate harness.
//!
//! Produces versioned JSON and Markdown reports under
//! tools/stabilization-gate/reports/<run_id>.{json,md}.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Result for a single benchmark scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    /// Short name identifying the scenario (e.g. "eventbus", "recon").
    pub name: String,
    /// Whether all thresholds were met.
    pub passed: bool,
    /// Collected metrics as a JSON object.
    pub metrics: serde_json::Value,
    /// Human-readable descriptions of any violated thresholds.
    pub threshold_violations: Vec<String>,
    /// Optional notes about the run (e.g. "dry-run: connectivity only").
    pub notes: Option<String>,
}

/// Top-level benchmark report.
#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// Unique run identifier (UUID v4).
    pub run_id: String,
    /// Short git SHA at time of run.
    pub git_sha: String,
    /// When the run started.
    pub started_at: DateTime<Utc>,
    /// When the run ended (set by `finalize()`).
    pub ended_at: DateTime<Utc>,
    /// Safe environment snapshot (no credentials).
    pub env_snapshot: serde_json::Value,
    /// Results for each scenario that ran.
    pub scenarios: Vec<ScenarioResult>,
    /// True when all scenarios passed their thresholds.
    pub overall_passed: bool,
    /// True when run with --dry-run (connectivity check only).
    pub dry_run: bool,
}

impl BenchmarkReport {
    pub fn new(
        run_id: String,
        git_sha: String,
        started_at: DateTime<Utc>,
        env_snapshot: serde_json::Value,
        dry_run: bool,
    ) -> Self {
        Self {
            run_id,
            git_sha,
            started_at,
            ended_at: Utc::now(),
            env_snapshot,
            scenarios: Vec::new(),
            overall_passed: true,
            dry_run,
        }
    }

    /// Add a scenario result. Sets overall_passed = false if the scenario failed.
    pub fn add_scenario(&mut self, result: ScenarioResult) {
        if !result.passed {
            self.overall_passed = false;
        }
        self.scenarios.push(result);
    }

    /// Stamp the end time. Call once all scenarios have been added.
    pub fn finalize(&mut self) {
        self.ended_at = Utc::now();
    }

    /// Write JSON and Markdown artifacts. Returns (json_path, md_path).
    pub fn write_artifacts(&self, reports_dir: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
        std::fs::create_dir_all(reports_dir)?;

        let json_path = reports_dir.join(format!("{}.json", self.run_id));
        let md_path = reports_dir.join(format!("{}.md", self.run_id));

        let json_str = serde_json::to_string_pretty(self)?;
        std::fs::write(&json_path, &json_str)?;
        std::fs::write(&md_path, self.to_markdown())?;

        Ok((json_path, md_path))
    }

    fn to_markdown(&self) -> String {
        let mut md = String::new();

        let status = if self.overall_passed {
            "✅ PASSED"
        } else {
            "❌ FAILED"
        };
        md.push_str(&format!("# Stabilization Gate Report — {}\n\n", status));
        md.push_str(&format!("**Run ID:** `{}`  \n", self.run_id));
        md.push_str(&format!("**Git SHA:** `{}`  \n", self.git_sha));
        md.push_str(&format!(
            "**Started:** {}  \n",
            self.started_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        md.push_str(&format!(
            "**Ended:** {}  \n",
            self.ended_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));

        if self.dry_run {
            md.push_str("**Mode:** DRY RUN (connectivity check only)  \n");
        }
        md.push('\n');

        md.push_str("## Environment\n\n```json\n");
        md.push_str(&serde_json::to_string_pretty(&self.env_snapshot).unwrap_or_default());
        md.push_str("\n```\n\n");

        md.push_str("## Scenario Results\n\n");
        md.push_str("| Scenario | Status | Ops | P50 ms | P95 ms | P99 ms | ops/s |\n");
        md.push_str("|----------|--------|-----|--------|--------|--------|-------|\n");

        for s in &self.scenarios {
            let icon = if s.passed { "✅" } else { "❌" };
            let m = &s.metrics;
            let ops = m.get("total_ops").and_then(|v| v.as_u64()).unwrap_or(0);
            let p50 = m.get("p50_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let p95 = m.get("p95_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let p99 = m.get("p99_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let tps = m
                .get("throughput_ops_per_sec")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            md.push_str(&format!(
                "| {} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} |\n",
                s.name, icon, ops, p50, p95, p99, tps
            ));
        }

        let violations: Vec<String> = self
            .scenarios
            .iter()
            .flat_map(|s| {
                s.threshold_violations
                    .iter()
                    .map(|v| format!("- **{}**: {}", s.name, v))
            })
            .collect();

        if !violations.is_empty() {
            md.push_str("\n## Threshold Violations\n\n");
            for v in &violations {
                md.push_str(v);
                md.push('\n');
            }
        }

        let notes: Vec<String> = self
            .scenarios
            .iter()
            .filter_map(|s| s.notes.as_ref().map(|n| format!("- **{}**: {}", s.name, n)))
            .collect();

        if !notes.is_empty() {
            md.push_str("\n## Notes\n\n");
            for n in &notes {
                md.push_str(n);
                md.push('\n');
            }
        }

        // Explicit gate result at end for easy grep / CI log scanning.
        let gate_result = if self.overall_passed {
            "GATE RESULT: PASS"
        } else {
            "GATE RESULT: FAIL"
        };
        md.push_str(&format!("\n---\n## {gate_result}\n\n"));
        md.push_str("*Generated by stabilization-gate*\n");
        md
    }
}
