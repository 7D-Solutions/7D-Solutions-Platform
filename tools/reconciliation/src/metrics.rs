//! Prometheus metrics output for the reconciliation runner.
//!
//! Emits two metric families in Prometheus text exposition format:
//!
//! - platform_recon_violations_total{module, invariant} — count of violating rows
//!   found for each invariant in the last run. 0 = clean.
//! - platform_recon_last_success_timestamp{module} — Unix timestamp (seconds) of
//!   the most recent clean run for the module. If a module had any violations this
//!   run, this metric is NOT updated (alerts on stale = 0 violations AND stale
//!   timestamp means the runner itself is failing).
//!
//! Alert rules (defined in alerts/recon.rules.yml):
//! - platform_recon_violation_detected: fires if any violations_total > 0
//! - platform_recon_runner_stale: fires if last_success_timestamp age > 26h
//!   (covers one missed nightly run with 2h buffer)

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::checks::Violation;

/// Render Prometheus text output for the given violation set.
///
/// `run_modules` — modules that were actually checked this run.
/// `violations`  — all violations found (may be empty for a clean run).
///
/// Returns the full text/plain content to write to the .prom file.
pub fn render(run_modules: &[&str], violations: &[Violation]) -> String {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Count violations per (module, invariant)
    let mut violation_counts: HashMap<(&str, &str), i64> = HashMap::new();
    for v in violations {
        *violation_counts
            .entry((v.module.as_str(), v.invariant.as_str()))
            .or_insert(0) += v.count;
    }

    // Track which modules had any violation
    let mut modules_with_violations: std::collections::HashSet<&str> =
        std::collections::HashSet::new();
    for v in violations {
        modules_with_violations.insert(v.module.as_str());
    }

    let mut out = String::new();

    // ── platform_recon_violations_total ──────────────────────────────────────
    out.push_str("# HELP platform_recon_violations_total Number of rows violating the named financial invariant in the last reconciliation run.\n");
    out.push_str("# TYPE platform_recon_violations_total gauge\n");

    // Emit a 0 for every module/invariant combination that was checked but had no violations.
    // This ensures alert expressions can use `== 0` reliably.
    for &module in run_modules {
        for &invariant in invariants_for_module(module) {
            let key = (module, invariant);
            let count = violation_counts.get(&key).copied().unwrap_or(0);
            out.push_str(&format!(
                "platform_recon_violations_total{{module=\"{module}\",invariant=\"{invariant}\"}} {count}\n"
            ));
        }
    }

    // Also emit any extra violations that might not be in the known list (defensive).
    for ((module, invariant), count) in &violation_counts {
        if !run_modules.contains(module) || !invariants_for_module(module).contains(invariant) {
            out.push_str(&format!(
                "platform_recon_violations_total{{module=\"{module}\",invariant=\"{invariant}\"}} {count}\n"
            ));
        }
    }

    // ── platform_recon_last_success_timestamp ────────────────────────────────
    out.push_str("\n# HELP platform_recon_last_success_timestamp Unix timestamp of the most recent clean reconciliation run for this module (no violations).\n");
    out.push_str("# TYPE platform_recon_last_success_timestamp gauge\n");

    for &module in run_modules {
        if !modules_with_violations.contains(module) {
            // Clean run for this module — update timestamp
            out.push_str(&format!(
                "platform_recon_last_success_timestamp{{module=\"{module}\"}} {now_secs}\n"
            ));
        }
        // If the module had violations, do NOT emit this metric — the previous
        // timestamp (from the last clean run stored in node_exporter textfile dir)
        // will remain, causing the staleness alert to fire if violations persist.
    }

    out
}

/// Returns the canonical invariant names for each module.
/// Used to emit zero-valued metrics for invariants that had no violations.
fn invariants_for_module(module: &str) -> &'static [&'static str] {
    match module {
        "ar" => &["invoice_line_total", "payment_allocation_cap"],
        "ap" => &["bill_line_total", "payment_allocation_cap"],
        "gl" => &["journal_entry_balanced", "closed_period_hash_present"],
        "inventory" => &["on_hand_matches_ledger", "no_negative_on_hand"],
        "bom" => &["revision_status_valid", "effective_bom_no_zero_qty"],
        "production" => &["completed_wo_output_cap", "closed_wo_has_actual_end"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::Violation;

    #[test]
    fn test_render_clean_run() {
        let modules = ["gl"];
        let violations: Vec<Violation> = vec![];
        let output = render(&modules, &violations);

        assert!(output.contains(
            "platform_recon_violations_total{module=\"gl\",invariant=\"journal_entry_balanced\"} 0"
        ));
        assert!(output.contains("platform_recon_last_success_timestamp{module=\"gl\"}"));
    }

    #[test]
    fn test_render_with_violation() {
        let modules = ["gl"];
        let violations = vec![Violation::new("gl", "journal_entry_balanced", 3, "test")];
        let output = render(&modules, &violations);

        assert!(output.contains(
            "platform_recon_violations_total{module=\"gl\",invariant=\"journal_entry_balanced\"} 3"
        ));
        // No success timestamp when there's a violation
        assert!(!output.contains("platform_recon_last_success_timestamp{module=\"gl\"}"));
    }

    #[test]
    fn test_render_zero_for_unchecked_invariant() {
        let modules = ["ar"];
        let violations: Vec<Violation> = vec![];
        let output = render(&modules, &violations);

        // Both AR invariants should appear with count 0
        assert!(output.contains(
            "platform_recon_violations_total{module=\"ar\",invariant=\"invoice_line_total\"} 0"
        ));
        assert!(output.contains(
            "platform_recon_violations_total{module=\"ar\",invariant=\"payment_allocation_cap\"} 0"
        ));
    }
}
