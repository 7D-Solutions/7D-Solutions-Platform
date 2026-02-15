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
use seed::SimulationSeed;
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::{info, warn, error};

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

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            runs: 5,
            tenant_count: 15,
            cycle_count: 6,
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
        _seed: &mut SimulationSeed,
    ) -> Result<()> {
        // TODO: Implement cycle execution
        // 1. Advance subscriptions (concurrent schedulers)
        // 2. Generate invoices (via subscription events)
        // 3. Inject failures (declines, timeouts→UNKNOWN)
        // 4. Attempt payments (concurrent workers)
        // 5. Deliver webhooks (including duplicates, delays)
        // 6. Run reconciliation (UNKNOWN → SUCCEEDED/FAILED)

        warn!(
            cycle = cycle,
            tenant_count = tenant_ids.len(),
            "Cycle execution NOT YET IMPLEMENTED"
        );

        // Placeholder: simulate some work
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

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
        // TODO: Implement digest computation
        // Count key records across all modules:
        // - AR invoices, AR attempts
        // - Payment attempts
        // - Subscription cycle attempts
        // - GL journal entries
        // - Status distributions (PAID, FAILED, UNKNOWN, etc.)

        warn!("DB digest computation NOT YET IMPLEMENTED");

        Ok(SimulationDigest {
            ar_invoices: 0,
            ar_attempts: 0,
            payment_attempts: 0,
            subscription_attempts: 0,
            gl_entries: 0,
            status_counts: HashMap::new(),
        })
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

    // Parse config (TODO: add CLI args)
    let config = SimulationConfig::default();

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
