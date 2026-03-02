//! Cross-tenant parallel stress runner (bd-tjsl, Wave 2).
//!
//! Runs all benchmark scenarios concurrently (eventbus + projections + recon +
//! dunning) and then validates cross-tenant data isolation invariants.
//!
//! Thresholds (hard, non-configurable):
//!   0 invariant violations across all scenarios
//!   0 cross-tenant data contamination (payment_id / invoice_id bleed)

use std::time::Duration;

use anyhow::Result;
use tracing::info;

use crate::config::Config;
use crate::report::ScenarioResult;

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run all benchmark scenarios in parallel and append a cross-tenant isolation result.
pub async fn run(cfg: &Config, dry_run: bool) -> Result<Vec<ScenarioResult>> {
    info!(
        "run-all: launching eventbus + projections + recon + dunning in parallel \
         (dry_run={} tenants={} concurrency={})",
        dry_run, cfg.tenant_count, cfg.concurrency
    );

    let cfg_eb = cfg.clone();
    let cfg_pr = cfg.clone();
    let cfg_rc = cfg.clone();
    let cfg_dn = cfg.clone();

    // All 4 scenarios run concurrently.
    let (eb, pr, rc, dn) = tokio::join!(
        crate::eventbus::run(&cfg_eb, dry_run),
        crate::projections::run(&cfg_pr, dry_run),
        crate::recon::run(&cfg_rc, dry_run),
        crate::dunning::run(&cfg_dn, dry_run),
    );

    let eb_result = eb?;
    let pr_result = pr?;
    let rc_result = rc?;
    let dn_result = dn?;

    info!("run-all: all parallel scenarios complete — running isolation check");

    let isolation_result = if dry_run {
        isolation_dry()
    } else {
        run_isolation_check(cfg).await?
    };

    info!(
        "run-all: done — eventbus={} projections={} recon={} dunning={} isolation={}",
        pass_label(eb_result.passed),
        pass_label(pr_result.passed),
        pass_label(rc_result.passed),
        pass_label(dn_result.passed),
        pass_label(isolation_result.passed),
    );

    Ok(vec![
        eb_result,
        pr_result,
        rc_result,
        dn_result,
        isolation_result,
    ])
}

// ── Cross-tenant isolation invariant check ────────────────────────────────────

/// Validate that parallel execution produced no cross-tenant data contamination.
///
/// Check 1 — recon: no `payment_id` in `ar_recon_matches` appears under more
///   than one bench tenant `app_id` (would indicate a cross-tenant match).
///
/// Check 2 — dunning: no `invoice_id` in `ar_dunning_states` is referenced by
///   an `ar_dunning_states.app_id` that differs from the owning invoice's
///   `ar_invoices.app_id` (would indicate a cross-tenant state write).
async fn run_isolation_check(cfg: &Config) -> Result<ScenarioResult> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&cfg.database_url)
        .await?;

    // Check 1: payment_id shared across multiple bench-rc-* app_ids.
    let recon_bleed: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM (
            SELECT payment_id
            FROM ar_recon_matches
            WHERE app_id LIKE 'bench-rc-%'
            GROUP BY payment_id
            HAVING COUNT(DISTINCT app_id) > 1
        ) violations
        "#,
    )
    .fetch_one(&pool)
    .await?;

    // Check 2: dunning state references an invoice that belongs to a different app_id.
    let dunning_bleed: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM ar_dunning_states d
        JOIN ar_invoices i ON i.id = d.invoice_id
        WHERE d.app_id LIKE 'bench-dn-%'
          AND i.app_id != d.app_id
        "#,
    )
    .fetch_one(&pool)
    .await?;

    let total_contamination = recon_bleed + dunning_bleed;

    let mut violations: Vec<String> = Vec::new();

    if recon_bleed > 0 {
        violations.push(format!(
            "cross-tenant contamination: {} payment_id(s) matched across multiple \
             bench-rc-* app_ids — recon engine crossed tenant boundary",
            recon_bleed
        ));
    }
    if dunning_bleed > 0 {
        violations.push(format!(
            "cross-tenant contamination: {} dunning state(s) reference an invoice \
             owned by a different app_id — dunning engine crossed tenant boundary",
            dunning_bleed
        ));
    }

    info!(
        "isolation: recon_bleed={} dunning_bleed={} total={}",
        recon_bleed, dunning_bleed, total_contamination
    );

    Ok(ScenarioResult {
        name: "cross_tenant_isolation".to_string(),
        passed: violations.is_empty(),
        metrics: serde_json::json!({
            "recon_cross_tenant_bleed": recon_bleed,
            "dunning_cross_tenant_bleed": dunning_bleed,
            "total_contamination_count": total_contamination,
        }),
        threshold_violations: violations,
        notes: None,
    })
}

fn isolation_dry() -> ScenarioResult {
    ScenarioResult {
        name: "cross_tenant_isolation".to_string(),
        passed: true,
        metrics: serde_json::json!({
            "recon_cross_tenant_bleed": 0,
            "dunning_cross_tenant_bleed": 0,
            "total_contamination_count": 0,
        }),
        threshold_violations: vec![],
        notes: Some("dry-run: isolation DB check skipped".to_string()),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn pass_label(passed: bool) -> &'static str {
    if passed {
        "PASS"
    } else {
        "FAIL"
    }
}
