//! Stabilization Gate Benchmark Harness
//!
//! Measures platform performance against real Postgres and NATS services.
//! Produces versioned JSON + Markdown reports under tools/stabilization-gate/reports/.
//!
//! Usage:
//!   cargo run -p stabilization-gate -- run-all --tenant-count 25 --events-per-tenant 200 \
//!       --recon-rows 2000 --dunning-rows 1000 --concurrency 50 --duration-secs 120
//!   cargo run -p stabilization-gate -- e2e-bench --runs 2
//!   cargo run -p stabilization-gate -- eventbus
//!   cargo run -p stabilization-gate -- projections
//!   cargo run -p stabilization-gate -- recon
//!   cargo run -p stabilization-gate -- dunning
//!   cargo run -p stabilization-gate -- tenants

mod benchmarks;
mod config;
mod dunning;
mod e2e_bench;
mod eventbus;
mod metrics;
mod projections;
mod recon;
mod report;
mod run_all;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process;
use tracing::{error, info, warn};
use uuid::Uuid;

use config::Config;
use report::BenchmarkReport;

// ── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "stabilization-gate",
    about = "Platform stabilization benchmark harness — real Postgres/NATS, no mocks"
)]
struct Cli {
    /// Directory to write reports into.
    #[arg(long, default_value = "tools/stabilization-gate/reports")]
    reports_dir: PathBuf,

    /// Suppress report artifact writing (still runs scenarios).
    #[arg(long)]
    no_write: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run all benchmark scenarios in parallel and write a combined report.
    ///
    /// Launches eventbus + projections + recon + dunning concurrently, then
    /// validates cross-tenant data isolation invariants.
    RunAll {
        /// Check connectivity and config only — do not run actual benchmarks.
        #[arg(long)]
        dry_run: bool,
        /// Number of tenants to simulate (overrides TENANT_COUNT env).
        #[arg(long)]
        tenant_count: Option<usize>,
        /// Events per tenant (overrides EVENTS_PER_TENANT env).
        #[arg(long)]
        events_per_tenant: Option<usize>,
        /// Total rows for reconciliation benchmark (overrides RECON_ROWS env).
        #[arg(long)]
        recon_rows: Option<usize>,
        /// Total rows for dunning benchmark (overrides DUNNING_ROWS env).
        #[arg(long)]
        dunning_rows: Option<usize>,
        /// Worker concurrency (overrides CONCURRENCY env).
        #[arg(long)]
        concurrency: Option<usize>,
        /// Duration in seconds for timed scenarios (overrides DURATION_SECS env).
        #[arg(long)]
        duration_secs: Option<u64>,
    },
    /// Benchmark NATS event bus publish/consume throughput.
    Eventbus {
        #[arg(long)]
        dry_run: bool,
        /// Number of tenants (overrides TENANT_COUNT env).
        #[arg(long)]
        tenant_count: Option<usize>,
        /// Events per tenant (overrides EVENTS_PER_TENANT env).
        #[arg(long)]
        events_per_tenant: Option<usize>,
        /// Publisher concurrency (overrides CONCURRENCY env).
        #[arg(long)]
        concurrency: Option<usize>,
        /// Drain deadline in seconds (overrides DURATION_SECS env).
        #[arg(long)]
        duration_secs: Option<u64>,
    },
    /// Benchmark projection rebuild time and steady-state lag (Wave 1, bd-3mzn).
    Projections {
        #[arg(long)]
        dry_run: bool,
        /// Number of tenants (overrides TENANT_COUNT env).
        #[arg(long)]
        tenant_count: Option<usize>,
        /// Events per tenant (overrides EVENTS_PER_TENANT env).
        #[arg(long)]
        events_per_tenant: Option<usize>,
        /// Lag-phase drain window in seconds (overrides DURATION_SECS env).
        #[arg(long)]
        duration_secs: Option<u64>,
    },
    /// Benchmark AR reconciliation batch throughput (Wave 1, bd-1kmq).
    Recon {
        #[arg(long)]
        dry_run: bool,
        /// Number of tenants (overrides TENANT_COUNT env).
        #[arg(long)]
        tenant_count: Option<usize>,
        /// Total rows to reconcile across all tenants (overrides RECON_ROWS env).
        #[arg(long)]
        recon_rows: Option<usize>,
        /// Worker concurrency (overrides CONCURRENCY env).
        #[arg(long)]
        concurrency: Option<usize>,
    },
    /// Benchmark dunning scheduler row processing (Wave 1, bd-1kmq).
    Dunning {
        #[arg(long)]
        dry_run: bool,
        /// Number of tenants (overrides TENANT_COUNT env).
        #[arg(long)]
        tenant_count: Option<usize>,
        /// Total overdue invoice rows to process (overrides DUNNING_ROWS env).
        #[arg(long)]
        dunning_rows: Option<usize>,
        /// Worker concurrency (overrides CONCURRENCY env).
        #[arg(long)]
        concurrency: Option<usize>,
    },
    /// Benchmark multi-tenant isolation and stress.
    Tenants {
        #[arg(long)]
        dry_run: bool,
    },
    /// Run `cargo test -p e2e-tests --no-fail-fast` N times and baseline timing.
    ///
    /// Captures per-run wall-clock duration, validates variance across runs,
    /// and persists a baseline in reports/e2e-baseline.json for regression detection.
    E2eBench {
        /// Number of full e2e test suite runs to execute.
        #[arg(long, default_value_t = 2)]
        runs: u32,
        /// Skip actual cargo test execution (connectivity check only).
        #[arg(long)]
        dry_run: bool,
    },
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("stabilization_gate=info".parse().unwrap()),
        )
        .init();

    if let Err(e) = run().await {
        error!("stabilization-gate failed: {:#}", e);
        process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let cfg = Config::from_env().context(
        "Configuration error — check DATABASE_URL / AR_DATABASE_URL and NATS_URL env vars",
    )?;

    // Verify connectivity before anything else.
    verify_connectivity(&cfg).await?;

    let run_id = format!("gate-{}", Uuid::new_v4().simple());
    let git_sha = current_git_sha();
    let started_at = Utc::now();

    let (scenarios, dry_run, effective_cfg) = match &cli.command {
        Commands::RunAll {
            dry_run,
            tenant_count,
            events_per_tenant,
            recon_rows,
            dunning_rows,
            concurrency,
            duration_secs,
        } => {
            let dry = *dry_run;
            let mut cfg2 = cfg.clone();
            if let Some(v) = tenant_count {
                cfg2.tenant_count = *v;
            }
            if let Some(v) = events_per_tenant {
                cfg2.events_per_tenant = *v;
            }
            if let Some(v) = recon_rows {
                cfg2.recon_rows = *v;
            }
            if let Some(v) = dunning_rows {
                cfg2.dunning_rows = *v;
            }
            if let Some(v) = concurrency {
                cfg2.concurrency = *v;
            }
            if let Some(v) = duration_secs {
                cfg2.duration_secs = *v;
            }
            let s = run_all::run(&cfg2, dry).await?;
            (s, dry, cfg2)
        }
        Commands::Eventbus {
            dry_run,
            tenant_count,
            events_per_tenant,
            concurrency,
            duration_secs,
        } => {
            let dry = *dry_run;
            let mut cfg2 = cfg.clone();
            if let Some(v) = tenant_count {
                cfg2.tenant_count = *v;
            }
            if let Some(v) = events_per_tenant {
                cfg2.events_per_tenant = *v;
            }
            if let Some(v) = concurrency {
                cfg2.concurrency = *v;
            }
            if let Some(v) = duration_secs {
                cfg2.duration_secs = *v;
            }
            let s = vec![eventbus::run(&cfg2, dry).await?];
            (s, dry, cfg2)
        }
        Commands::Projections {
            dry_run,
            tenant_count,
            events_per_tenant,
            duration_secs,
        } => {
            let dry = *dry_run;
            let mut cfg2 = cfg.clone();
            if let Some(v) = tenant_count {
                cfg2.tenant_count = *v;
            }
            if let Some(v) = events_per_tenant {
                cfg2.events_per_tenant = *v;
            }
            if let Some(v) = duration_secs {
                cfg2.duration_secs = *v;
            }
            let s = vec![projections::run(&cfg2, dry).await?];
            (s, dry, cfg2)
        }
        Commands::Recon {
            dry_run,
            tenant_count,
            recon_rows,
            concurrency,
        } => {
            let dry = *dry_run;
            let mut cfg2 = cfg.clone();
            if let Some(v) = tenant_count {
                cfg2.tenant_count = *v;
            }
            if let Some(v) = recon_rows {
                cfg2.recon_rows = *v;
            }
            if let Some(v) = concurrency {
                cfg2.concurrency = *v;
            }
            let s = vec![recon::run(&cfg2, dry).await?];
            (s, dry, cfg2)
        }
        Commands::Dunning {
            dry_run,
            tenant_count,
            dunning_rows,
            concurrency,
        } => {
            let dry = *dry_run;
            let mut cfg2 = cfg.clone();
            if let Some(v) = tenant_count {
                cfg2.tenant_count = *v;
            }
            if let Some(v) = dunning_rows {
                cfg2.dunning_rows = *v;
            }
            if let Some(v) = concurrency {
                cfg2.concurrency = *v;
            }
            let s = vec![dunning::run(&cfg2, dry).await?];
            (s, dry, cfg2)
        }
        Commands::Tenants { dry_run } => {
            let dry = *dry_run;
            let s = vec![benchmarks::bench_tenants(&cfg, dry).await?];
            (s, dry, cfg.clone())
        }
        Commands::E2eBench { runs, dry_run } => {
            let dry = *dry_run;
            let scenario = if dry {
                e2e_bench::e2e_dry_run()
            } else {
                e2e_bench::run_e2e_bench(*runs, &cli.reports_dir).await?
            };
            (vec![scenario], dry, cfg.clone())
        }
    };

    let mut report = BenchmarkReport::new(
        run_id.clone(),
        git_sha,
        started_at,
        effective_cfg.env_snapshot(),
        dry_run,
    );

    for s in scenarios {
        report.add_scenario(s);
    }
    report.finalize();

    let passed = report.overall_passed;

    if !cli.no_write {
        let (json_path, md_path) = report
            .write_artifacts(&cli.reports_dir)
            .context("Failed to write report artifacts")?;
        info!("Report written: {}", json_path.display());
        info!("Summary written: {}", md_path.display());
    }

    if passed {
        info!("Gate result: PASS (run_id={})", run_id);
    } else {
        warn!(
            "Gate result: FAIL (run_id={}) — see report for threshold violations",
            run_id
        );
        process::exit(2);
    }

    Ok(())
}

// ── Connectivity check ────────────────────────────────────────────────────────

async fn verify_connectivity(cfg: &Config) -> Result<()> {
    info!("Verifying Postgres connectivity…");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&cfg.database_url)
        .await
        .with_context(|| {
            format!(
                "Cannot connect to Postgres. DATABASE_URL host: {}",
                extract_host(&cfg.database_url)
            )
        })?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .context("Postgres ping failed")?;
    info!("Postgres OK");

    info!("Verifying NATS connectivity…");
    let nc = async_nats::connect(&cfg.nats_url)
        .await
        .with_context(|| format!("Cannot connect to NATS at {}", cfg.nats_url))?;
    nc.flush().await.context("NATS flush failed")?;
    info!("NATS OK ({})", cfg.nats_url);

    Ok(())
}

fn extract_host(url: &str) -> String {
    url.split('@')
        .next_back()
        .and_then(|s| s.split('/').next())
        .unwrap_or("unknown")
        .to_string()
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
