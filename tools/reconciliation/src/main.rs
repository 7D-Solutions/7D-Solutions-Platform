//! Nightly Financial Invariant Reconciliation Runner
//!
//! Connects to each module's live PostgreSQL database and runs invariant checks.
//! Emits Prometheus metrics in textfile format (for node_exporter textfile collector).
//!
//! Usage:
//!   reconciliation              # Check all configured modules, write metrics
//!   reconciliation --dry-run    # Print what would run without connecting
//!   reconciliation --modules ar,gl  # Only run specified modules
//!
//! Environment variables:
//!   AR_DATABASE_URL          PostgreSQL URL for the AR module
//!   AP_DATABASE_URL          PostgreSQL URL for the AP module
//!   GL_DATABASE_URL          PostgreSQL URL for the GL module
//!   INVENTORY_DATABASE_URL   PostgreSQL URL for the Inventory module
//!   BOM_DATABASE_URL         PostgreSQL URL for the BOM module
//!   PRODUCTION_DATABASE_URL  PostgreSQL URL for the Production module
//!   RECON_METRICS_OUTPUT     Directory for .prom files (default: /var/lib/prometheus/node_exporter)
//!                            Set to "-" to write to stdout.
//!
//! Exit codes:
//!   0  All checks passed (or --dry-run)
//!   1  One or more invariant violations found
//!   2  Configuration error (no modules configured)
//!   3  Database connectivity error

mod checks;
mod config;
mod metrics;

use anyhow::{Context, Result};
use clap::Parser;
use sqlx::postgres::PgPoolOptions;
use std::path::Path;
use std::process;
use tracing::{error, info, warn};

use checks::Violation;
use config::Config;

#[derive(Parser, Debug)]
#[command(
    name = "reconciliation",
    about = "Nightly financial invariant reconciliation runner — real databases, no mocks"
)]
struct Cli {
    /// Print what would run without connecting to databases.
    #[arg(long)]
    dry_run: bool,

    /// Comma-separated list of modules to check (default: all configured).
    /// Valid values: ar, ap, gl, inventory, bom, production
    #[arg(long)]
    modules: Option<String>,

    /// Exit with code 0 even if violations are found (for cron jobs that should
    /// not kill downstream steps on first violation).
    #[arg(long)]
    no_fail_on_violation: bool,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "reconciliation=info".parse().unwrap()),
        )
        .init();

    match run().await {
        Ok(had_violations) => {
            if had_violations {
                process::exit(1);
            } else {
                process::exit(0);
            }
        }
        Err(e) => {
            error!("Fatal error: {e:#}");
            process::exit(3);
        }
    }
}

async fn run() -> Result<bool> {
    let cli = Cli::parse();
    let config = Config::from_env();

    if !config.has_any_module() {
        error!(
            "No module database URLs configured. \
             Set at least one of: AR_DATABASE_URL, AP_DATABASE_URL, GL_DATABASE_URL, \
             INVENTORY_DATABASE_URL, BOM_DATABASE_URL, PRODUCTION_DATABASE_URL"
        );
        process::exit(2);
    }

    // Determine which modules to run
    let requested: Option<Vec<&str>> = cli.modules.as_deref().map(|m| m.split(',').collect());

    let run_ar = should_run("ar", &requested, config.ar_database_url.is_some());
    let run_ap = should_run("ap", &requested, config.ap_database_url.is_some());
    let run_gl = should_run("gl", &requested, config.gl_database_url.is_some());
    let run_inventory = should_run(
        "inventory",
        &requested,
        config.inventory_database_url.is_some(),
    );
    let run_bom = should_run("bom", &requested, config.bom_database_url.is_some());
    let run_production = should_run(
        "production",
        &requested,
        config.production_database_url.is_some(),
    );

    let mut run_modules: Vec<&str> = Vec::new();
    if run_ar {
        run_modules.push("ar");
    }
    if run_ap {
        run_modules.push("ap");
    }
    if run_gl {
        run_modules.push("gl");
    }
    if run_inventory {
        run_modules.push("inventory");
    }
    if run_bom {
        run_modules.push("bom");
    }
    if run_production {
        run_modules.push("production");
    }

    info!("Reconciliation run: modules={:?}", run_modules);

    if cli.dry_run {
        info!("DRY RUN — would check modules: {:?}", run_modules);
        return Ok(false);
    }

    let mut all_violations: Vec<Violation> = Vec::new();

    // ── AR ───────────────────────────────────────────────────────────────────
    if run_ar {
        let url = config.ar_database_url.as_deref().unwrap();
        match connect(url, "ar").await {
            Ok(pool) => {
                match checks::ar::run_checks(&pool).await {
                    Ok(v) => {
                        log_module_result("ar", &v);
                        all_violations.extend(v);
                    }
                    Err(e) => warn!("AR checks error: {e:#}"),
                }
                pool.close().await;
            }
            Err(e) => warn!("AR DB connection failed: {e:#}"),
        }
    }

    // ── AP ───────────────────────────────────────────────────────────────────
    if run_ap {
        let url = config.ap_database_url.as_deref().unwrap();
        match connect(url, "ap").await {
            Ok(pool) => {
                match checks::ap::run_checks(&pool).await {
                    Ok(v) => {
                        log_module_result("ap", &v);
                        all_violations.extend(v);
                    }
                    Err(e) => warn!("AP checks error: {e:#}"),
                }
                pool.close().await;
            }
            Err(e) => warn!("AP DB connection failed: {e:#}"),
        }
    }

    // ── GL ───────────────────────────────────────────────────────────────────
    if run_gl {
        let url = config.gl_database_url.as_deref().unwrap();
        match connect(url, "gl").await {
            Ok(pool) => {
                match checks::gl::run_checks(&pool).await {
                    Ok(v) => {
                        log_module_result("gl", &v);
                        all_violations.extend(v);
                    }
                    Err(e) => warn!("GL checks error: {e:#}"),
                }
                pool.close().await;
            }
            Err(e) => warn!("GL DB connection failed: {e:#}"),
        }
    }

    // ── Inventory ─────────────────────────────────────────────────────────────
    if run_inventory {
        let url = config.inventory_database_url.as_deref().unwrap();
        match connect(url, "inventory").await {
            Ok(pool) => {
                match checks::inventory::run_checks(&pool).await {
                    Ok(v) => {
                        log_module_result("inventory", &v);
                        all_violations.extend(v);
                    }
                    Err(e) => warn!("Inventory checks error: {e:#}"),
                }
                pool.close().await;
            }
            Err(e) => warn!("Inventory DB connection failed: {e:#}"),
        }
    }

    // ── BOM ───────────────────────────────────────────────────────────────────
    if run_bom {
        let url = config.bom_database_url.as_deref().unwrap();
        match connect(url, "bom").await {
            Ok(pool) => {
                match checks::bom::run_checks(&pool).await {
                    Ok(v) => {
                        log_module_result("bom", &v);
                        all_violations.extend(v);
                    }
                    Err(e) => warn!("BOM checks error: {e:#}"),
                }
                pool.close().await;
            }
            Err(e) => warn!("BOM DB connection failed: {e:#}"),
        }
    }

    // ── Production ─────────────────────────────────────────────────────────────
    if run_production {
        let url = config.production_database_url.as_deref().unwrap();
        match connect(url, "production").await {
            Ok(pool) => {
                match checks::production::run_checks(&pool).await {
                    Ok(v) => {
                        log_module_result("production", &v);
                        all_violations.extend(v);
                    }
                    Err(e) => warn!("Production checks error: {e:#}"),
                }
                pool.close().await;
            }
            Err(e) => warn!("Production DB connection failed: {e:#}"),
        }
    }

    // ── Report ────────────────────────────────────────────────────────────────
    let had_violations = !all_violations.is_empty();

    if had_violations {
        error!(
            "RECONCILIATION FAILED: {} violation(s) detected",
            all_violations.len()
        );
        for v in &all_violations {
            error!(
                "  VIOLATION module={} invariant={} count={} detail={}",
                v.module, v.invariant, v.count, v.detail
            );
        }
    } else {
        info!("RECONCILIATION PASSED: all invariants satisfied");
    }

    // ── Metrics output ────────────────────────────────────────────────────────
    let metrics_text = metrics::render(&run_modules, &all_violations);

    if config.metrics_output == "-" {
        print!("{}", metrics_text);
    } else {
        let metrics_path = Path::new(&config.metrics_output).join("recon.prom");
        std::fs::write(&metrics_path, &metrics_text)
            .with_context(|| format!("Failed to write metrics to {}", metrics_path.display()))?;
        info!("Metrics written to {}", metrics_path.display());
    }

    if cli.no_fail_on_violation {
        Ok(false)
    } else {
        Ok(had_violations)
    }
}

/// Returns true if the given module should be run in this pass.
fn should_run(module: &str, requested: &Option<Vec<&str>>, configured: bool) -> bool {
    if !configured {
        return false;
    }
    match requested {
        None => true,
        Some(list) => list.contains(&module),
    }
}

/// Open a read-only connection pool to a module database (max 2 connections).
async fn connect(database_url: &str, module: &str) -> Result<sqlx::PgPool> {
    info!("Connecting to {module} database");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url)
        .await
        .with_context(|| format!("Failed to connect to {module} database"))
}

/// Log module check result.
fn log_module_result(module: &str, violations: &[Violation]) {
    if violations.is_empty() {
        info!("{module}: all invariants PASSED");
    } else {
        warn!(
            "{module}: {} invariant violation(s) found",
            violations.len()
        );
    }
}
