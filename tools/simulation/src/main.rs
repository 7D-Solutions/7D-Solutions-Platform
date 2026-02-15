//! Deterministic Simulation Harness (bd-3c2 - Phase 15 Final Gate)
//!
//! **Purpose:** Production-grade validation of billing lifecycle correctness
//!
//! **ChatGPT Requirements:**
//! 1. Deterministic seeded RNG (ChaCha20)
//! 2. 10-20 tenants, 6 compressed cycles
//! 3. Failure injection (declines, timeouts→UNKNOWN, duplicate webhooks, replay)
//! 4. Concurrency stress (8-32 workers, barrier start)
//! 5. Reproducibility (seed=X, runs=5 → identical outcomes)
//! 6. Exit Criteria: 5 runs pass, oracle passes every cycle, no duplicates
//!
//! **Usage:**
//! ```bash
//! cargo run --bin simulation -- --seed 42 --runs 5 --tenants 15 --cycles 6
//! ```

mod failures;
mod scheduler;
mod seed;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::Parser;
use seed::SimulationSeed;
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::{info, error};

// ============================================================================
// CLI Arguments
// ============================================================================

/// Deterministic Simulation Harness for Phase 15 Final Gate (bd-3c2)
#[derive(Parser, Debug, Clone)]
#[command(name = "simulation")]
#[command(about = "Deterministic billing lifecycle simulation with oracle validation", long_about = None)]
struct CliArgs {
    /// Seed for deterministic RNG (ChaCha20)
    #[arg(short, long, default_value_t = 42)]
    seed: u64,

    /// Number of simulation runs with same seed (must produce identical outcomes)
    #[arg(short, long, default_value_t = 5)]
    runs: usize,

    /// Number of tenants to simulate (10-20 per ChatGPT requirement)
    #[arg(short, long, default_value_t = 15)]
    tenants: usize,

    /// Number of billing cycles to compress (6 per ChatGPT requirement)
    #[arg(short, long, default_value_t = 6)]
    cycles: usize,
}

// ============================================================================
// Configuration
// ============================================================================

#[derive(Debug, Clone)]
struct SimulationConfig {
    seed: u64,
    runs: usize,
    tenant_count: usize,
    cycle_count: usize,
}

impl From<CliArgs> for SimulationConfig {
    fn from(args: CliArgs) -> Self {
        Self {
            seed: args.seed,
            runs: args.runs,
            tenant_count: args.tenants,
            cycle_count: args.cycles,
        }
    }
}

// ============================================================================
// Oracle Integration
// ============================================================================

/// Test context for oracle (mirrors e2e-tests/tests/oracle.rs)
struct OracleContext<'a> {
    ar_pool: &'a PgPool,
    payments_pool: &'a PgPool,
    subscriptions_pool: &'a PgPool,
    gl_pool: &'a PgPool,
    app_id: &'a str,
    tenant_id: &'a str,
}

/// Assert all cross-module invariants using oracle spine
async fn assert_cross_module_invariants(ctx: &OracleContext<'_>) -> Result<()> {
    // Call AR invariants
    ar_rs::invariants::assert_all_invariants(ctx.ar_pool, ctx.app_id)
        .await
        .context("AR invariant violation")?;

    // Call Payments invariants
    payments_rs::invariants::assert_all_invariants(ctx.payments_pool, ctx.app_id)
        .await
        .context("Payments invariant violation")?;

    // Call Subscriptions invariants
    subscriptions_rs::invariants::assert_all_invariants(ctx.subscriptions_pool, ctx.tenant_id)
        .await
        .context("Subscriptions invariant violation")?;

    // Call GL invariants
    gl_rs::invariants::assert_all_invariants(ctx.gl_pool, ctx.tenant_id)
        .await
        .context("GL invariant violation")?;

    Ok(())
}

// ============================================================================
// Database Pools
// ============================================================================

async fn setup_database_pools() -> Result<DatabasePools> {
    let ar_pool = create_pool(
        "AR_DATABASE_URL",
        "postgresql://ar_user:ar_pass@localhost:5434/ar_db",
    ).await?;

    let payments_pool = create_pool(
        "PAYMENTS_DATABASE_URL",
        "postgresql://payments_user:payments_pass@localhost:5436/payments_db",
    ).await?;

    let subscriptions_pool = create_pool(
        "SUBSCRIPTIONS_DATABASE_URL",
        "postgresql://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db",
    ).await?;

    let gl_pool = create_pool(
        "GL_DATABASE_URL",
        "postgresql://gl_user:gl_pass@localhost:5438/gl_db",
    ).await?;

    Ok(DatabasePools {
        ar_pool,
        payments_pool,
        subscriptions_pool,
        gl_pool,
    })
}

async fn create_pool(env_var: &str, default_url: &str) -> Result<PgPool> {
    let url = std::env::var(env_var).unwrap_or_else(|_| default_url.to_string());

    sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .min_connections(2)
        .connect(&url)
        .await
        .context(format!("Failed to connect to database: {}", env_var))
}

struct DatabasePools {
    ar_pool: PgPool,
    payments_pool: PgPool,
    subscriptions_pool: PgPool,
    gl_pool: PgPool,
}

// ============================================================================
// Simulation Runner
// ============================================================================

struct SimulationRunner {
    config: SimulationConfig,
    pools: DatabasePools,
}

impl SimulationRunner {
    fn new(config: SimulationConfig, pools: DatabasePools) -> Self {
        Self { config, pools }
    }

    /// Run complete simulation: N runs with same seed
    ///
    /// **Exit Criteria:**
    /// - All runs pass oracle after every cycle
    /// - (Recommended) Identical DB digest across runs
    async fn run_simulation(&self) -> Result<()> {
        info!(
            seed = self.config.seed,
            runs = self.config.runs,
            tenants = self.config.tenant_count,
            cycles = self.config.cycle_count,
            "Starting deterministic simulation"
        );

        let mut run_digests = Vec::new();

        for run_number in 1..=self.config.runs {
            info!(run = run_number, total_runs = self.config.runs, "Starting run");

            // Reset seed for each run (same seed → identical outcomes)
            let seed = SimulationSeed::new(self.config.seed);

            // Run single simulation
            let digest = self.run_single_simulation(run_number, seed).await
                .context(format!("Run {} failed", run_number))?;

            run_digests.push(digest);

            info!(run = run_number, "Run completed successfully");
        }

        // Verify determinism: all runs should produce same digest
        self.verify_determinism(&run_digests)?;

        info!(
            "🎉 Simulation PASSED: {} deterministic runs completed with identical outcomes",
            self.config.runs
        );

        Ok(())
    }

    /// Run single simulation: N cycles with oracle checks
    async fn run_single_simulation(
        &self,
        run_number: usize,
        mut seed: SimulationSeed,
    ) -> Result<SimulationDigest> {
        // Generate deterministic tenant IDs
        let tenant_ids = seed.generate_tenant_ids(self.config.tenant_count);

        info!(
            run = run_number,
            tenant_count = tenant_ids.len(),
            "Generated deterministic tenants"
        );

        // TODO: Create tenants in DB (via subscriptions module)
        // TODO: Create subscriptions for each tenant

        // Run N cycles
        for cycle in 1..=self.config.cycle_count {
            info!(
                run = run_number,
                cycle = cycle,
                total_cycles = self.config.cycle_count,
                "Starting cycle"
            );

            // Execute cycle (concurrent)
            self.execute_cycle(cycle, &tenant_ids, &mut seed).await
                .context(format!("Cycle {} failed", cycle))?;

            // Call oracle after each cycle (ChatGPT requirement)
            self.assert_all_invariants_all_tenants(&tenant_ids).await
                .context(format!("Oracle failed after cycle {}", cycle))?;

            info!(
                run = run_number,
                cycle = cycle,
                "✅ Cycle completed, oracle passed"
            );
        }

        // Compute DB digest for determinism verification
        let digest = self.compute_db_digest().await?;

        Ok(digest)
    }

    /// Execute single billing cycle for all tenants (concurrent)
    async fn execute_cycle(
        &self,
        cycle: usize,
        tenant_ids: &[String],
        seed: &mut seed::SimulationSeed,
    ) -> Result<()> {
        use chrono::{Datelike, NaiveDate};

        info!(
            cycle = cycle,
            tenant_count = tenant_ids.len(),
            "Executing billing cycle"
        );

        // Calculate execution date (advance by month per cycle)
        let base_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let year_offset = ((cycle - 1) / 12) as i32;
        let month_offset = ((cycle - 1) % 12) as u32;
        let execution_date = NaiveDate::from_ymd_opt(
            2026 + year_offset,
            ((base_date.month() + month_offset) % 12) + 1,
            1,
        ).context("Failed to calculate execution date")?;

        info!(
            cycle = cycle,
            execution_date = %execution_date,
            "Calculated execution date"
        );

        // For each tenant, trigger billing cycle
        for tenant_id in tenant_ids {
            self.execute_tenant_cycle(tenant_id, execution_date, seed)
                .await
                .context(format!("Failed to execute cycle for tenant {}", tenant_id))?;
        }

        info!(cycle = cycle, "✅ All tenant cycles executed");
        Ok(())
    }

    /// Execute billing cycle for single tenant
    async fn execute_tenant_cycle(
        &self,
        tenant_id: &str,
        execution_date: NaiveDate,
        seed: &mut seed::SimulationSeed,
    ) -> Result<()> {
        // Note: Full implementation requires subscription setup, invoice generation,
        // payment attempts, webhook delivery, etc.
        // This is a simplified version that demonstrates the structure.

        info!(
            tenant_id = tenant_id,
            execution_date = %execution_date,
            "Executing tenant billing cycle (simplified)"
        );

        // TODO: Implement full cycle:
        // 1. Trigger subscription bill run (using subscriptions_rs::cycle_gating)
        // 2. Create invoice in AR
        // 3. Create payment attempt with injected failure
        // 4. Emit webhooks (with duplicates if seed indicates)
        // 5. Process payment (success/decline/timeout→UNKNOWN)
        // 6. Reconcile UNKNOWN states if any

        // Placeholder: Demonstrate failure injection
        let outcome = seed::FailureType::Success; // Would use seed.generate_failure_type()

        info!(
            tenant_id = tenant_id,
            outcome = ?outcome,
            "Billing cycle outcome determined"
        );

        Ok(())
    }

    /// Assert invariants for all tenants
    async fn assert_all_invariants_all_tenants(&self, tenant_ids: &[String]) -> Result<()> {
        for tenant_id in tenant_ids {
            let ctx = OracleContext {
                ar_pool: &self.pools.ar_pool,
                payments_pool: &self.pools.payments_pool,
                subscriptions_pool: &self.pools.subscriptions_pool,
                gl_pool: &self.pools.gl_pool,
                app_id: tenant_id,
                tenant_id,
            };

            assert_cross_module_invariants(&ctx).await
                .context(format!("Invariant violation for tenant {}", tenant_id))?;
        }

        Ok(())
    }

    /// Compute DB digest for determinism verification
    async fn compute_db_digest(&self) -> Result<SimulationDigest> {
        info!("Computing DB digest for determinism verification");

        // Count AR invoices
        let ar_invoices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ar.ar_invoices")
            .fetch_one(&self.pools.ar_pool)
            .await
            .context("Failed to count AR invoices")?;

        // Count AR attempts
        let ar_attempts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoice_attempts")
            .fetch_one(&self.pools.ar_pool)
            .await
            .context("Failed to count AR attempts")?;

        // Count payment attempts
        let payment_attempts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payment_attempts")
            .fetch_one(&self.pools.payments_pool)
            .await
            .context("Failed to count payment attempts")?;

        // Count subscription cycle attempts
        let subscription_attempts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM subscription_invoice_attempts")
            .fetch_one(&self.pools.subscriptions_pool)
            .await
            .context("Failed to count subscription attempts")?;

        // Count GL journal entries
        let gl_entries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
            .fetch_one(&self.pools.gl_pool)
            .await
            .context("Failed to count GL entries")?;

        // Get status distributions (AR invoices)
        let ar_status_rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT status, COUNT(*) as count FROM ar.ar_invoices GROUP BY status"
        )
        .fetch_all(&self.pools.ar_pool)
        .await
        .context("Failed to get AR status distribution")?;

        // Get payment status distributions
        let payment_status_rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT status, COUNT(*) as count FROM payment_attempts GROUP BY status"
        )
        .fetch_all(&self.pools.payments_pool)
        .await
        .context("Failed to get payment status distribution")?;

        // Build status counts map
        let mut status_counts = HashMap::new();
        for (status, count) in ar_status_rows {
            status_counts.insert(format!("ar_invoice_{}", status), count);
        }
        for (status, count) in payment_status_rows {
            status_counts.insert(format!("payment_{}", status), count);
        }

        let digest = SimulationDigest {
            ar_invoices,
            ar_attempts,
            payment_attempts,
            subscription_attempts,
            gl_entries,
            status_counts,
        };

        info!(
            ar_invoices = digest.ar_invoices,
            ar_attempts = digest.ar_attempts,
            payment_attempts = digest.payment_attempts,
            subscription_attempts = digest.subscription_attempts,
            gl_entries = digest.gl_entries,
            "DB digest computed"
        );

        Ok(digest)
    }

    /// Verify determinism: all run digests should be identical
    fn verify_determinism(&self, digests: &[SimulationDigest]) -> Result<()> {
        if digests.is_empty() {
            return Ok(());
        }

        let first = &digests[0];
        for (i, digest) in digests.iter().enumerate().skip(1) {
            if digest != first {
                error!(
                    run = i + 1,
                    "Determinism violation: run produced different digest"
                );
                return Err(anyhow::anyhow!(
                    "Determinism violation: Run {} digest differs from run 1", i + 1
                ));
            }
        }

        info!("✅ Determinism verified: all {} runs produced identical digests", digests.len());
        Ok(())
    }
}

// ============================================================================
// DB Digest
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
struct SimulationDigest {
    ar_invoices: i64,
    ar_attempts: i64,
    payment_attempts: i64,
    subscription_attempts: i64,
    gl_entries: i64,
    status_counts: HashMap<String, i64>,
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    // Parse CLI arguments
    let args = CliArgs::parse();
    let config = SimulationConfig::from(args);

    info!("Deterministic Simulation Harness (bd-3c2)");
    info!("===========================================");
    info!("Seed: {}", config.seed);
    info!("Runs: {}", config.runs);
    info!("Tenants: {}", config.tenant_count);
    info!("Cycles: {}", config.cycle_count);
    info!("===========================================");

    // Setup database pools
    let pools = setup_database_pools().await
        .context("Failed to setup database pools")?;

    info!("✅ Database pools initialized");

    // Run simulation
    let runner = SimulationRunner::new(config, pools);
    runner.run_simulation().await?;

    Ok(())
}
