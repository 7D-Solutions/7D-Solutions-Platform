//! demo-seed — Deterministic Demo Data Seeder
//!
//! **Purpose:** Create deterministic demo data in a provisioned tenant via real
//! module HTTP APIs. Two runs with the same seed always produce an identical
//! dataset and the same SHA256 digest of created resource IDs.
//!
//! **Key properties:**
//! - Seeded ChaCha8 RNG drives all randomness
//! - Deterministic correlation IDs derived from seed + sequence index
//! - Idempotent: calling twice with same seed creates the same resources
//!   (idempotency keys on AR endpoints protect against duplication)
//! - Dataset hash is stable across machines and Rust versions
//!
//! **Usage:**
//! ```bash
//! cargo run -p demo-seed -- --tenant t1 --seed 42 --ar-url http://localhost:8086
//! cargo run -p demo-seed -- --tenant t1 --seed 42 --print-hash
//! ```

mod seed;
mod ar;
mod digest;

use anyhow::Result;
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// demo-seed — deterministic demo data generator
#[derive(Parser, Debug)]
#[command(name = "demo-seed")]
#[command(about = "Create deterministic demo data in a provisioned tenant", long_about = None)]
struct Cli {
    /// Tenant ID (used as a namespace for idempotency keys)
    #[arg(long, env = "DEMO_TENANT_ID")]
    tenant: String,

    /// Deterministic RNG seed — same seed produces identical data
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// AR module base URL
    #[arg(long, env = "AR_BASE_URL", default_value = "http://localhost:8086")]
    ar_url: String,

    /// Number of customers to create
    #[arg(long, default_value_t = 2)]
    customers: usize,

    /// Number of invoices per customer
    #[arg(long, default_value_t = 3)]
    invoices_per_customer: usize,

    /// Print dataset digest and exit (no HTTP calls)
    #[arg(long, default_value_t = false)]
    print_hash: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();

    let cli = Cli::parse();

    if cli.print_hash {
        // Compute expected digest without hitting any APIs
        let expected = digest::expected_digest(
            &cli.tenant,
            cli.seed,
            cli.customers,
            cli.invoices_per_customer,
        );
        println!("{}", expected);
        return Ok(());
    }

    info!(
        tenant = %cli.tenant,
        seed = cli.seed,
        customers = cli.customers,
        invoices_per_customer = cli.invoices_per_customer,
        "Starting demo-seed run",
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let mut tracker = digest::DigestTracker::new();
    let mut rng = seed::DemoSeed::new(cli.seed);

    // Create customers and invoices
    for customer_idx in 0..cli.customers {
        let customer_corr_id = rng.correlation_id(&cli.tenant, "customer", customer_idx);

        let customer_id = ar::create_customer(
            &client,
            &cli.ar_url,
            &cli.tenant,
            customer_idx,
            &customer_corr_id,
        )
        .await
        .map_err(|e| {
            warn!(error = %e, customer_idx, "Failed to create customer");
            e
        })?;

        tracker.record_customer(customer_id, &customer_corr_id);
        info!(customer_idx, customer_id, "Created customer");

        for invoice_idx in 0..cli.invoices_per_customer {
            let invoice_corr_id =
                rng.correlation_id(&cli.tenant, "invoice", customer_idx * 100 + invoice_idx);
            let amount_cents = rng.amount_cents(1000, 50000);
            let due_days = rng.due_days(14, 60);

            let invoice_id = ar::create_and_finalize_invoice(
                &client,
                &cli.ar_url,
                customer_id,
                amount_cents,
                due_days,
                &invoice_corr_id,
            )
            .await
            .map_err(|e| {
                warn!(error = %e, customer_idx, invoice_idx, "Failed to create invoice");
                e
            })?;

            tracker.record_invoice(invoice_id, &invoice_corr_id, amount_cents);
            info!(customer_idx, invoice_idx, invoice_id, amount_cents, "Created invoice");
        }
    }

    let digest = tracker.finalize();
    info!(digest = %digest, tenant = %cli.tenant, seed = cli.seed, "Demo-seed complete");
    println!("{}", digest);

    Ok(())
}
