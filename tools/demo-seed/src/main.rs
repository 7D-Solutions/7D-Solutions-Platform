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
//! cargo run -p demo-seed -- --tenant t1 --seed 42
//! cargo run -p demo-seed -- --tenant t1 --seed 42 --modules numbering
//! cargo run -p demo-seed -- --tenant t1 --seed 42 --modules numbering,ar
//! cargo run -p demo-seed -- --tenant t1 --seed 42 --print-hash
//! ```

mod ar;
mod digest;
mod numbering;
mod party;
mod seed;

use std::collections::HashSet;

use anyhow::Result;
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// All supported module names in dependency order
const MODULE_ORDER: &[&str] = &[
    "numbering",
    "gl",
    "party",
    "inventory",
    "bom",
    "production",
    "ar",
];

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

    /// Modules to seed (comma-separated: numbering,gl,party,inventory,bom,production,ar,all)
    #[arg(long, default_value = "all", value_delimiter = ',')]
    modules: Vec<String>,

    // --- Service URLs ---
    /// AR module base URL
    #[arg(long, env = "AR_BASE_URL", default_value = "http://localhost:8086")]
    ar_url: String,

    /// Numbering service base URL
    #[arg(long, env = "NUMBERING_BASE_URL", default_value = "http://localhost:8120")]
    numbering_url: String,

    /// GL service base URL
    #[arg(long, env = "GL_BASE_URL", default_value = "http://localhost:8090")]
    gl_url: String,

    /// Party service base URL
    #[arg(long, env = "PARTY_BASE_URL", default_value = "http://localhost:8098")]
    party_url: String,

    /// Inventory service base URL
    #[arg(long, env = "INVENTORY_BASE_URL", default_value = "http://localhost:8092")]
    inventory_url: String,

    /// BOM service base URL
    #[arg(long, env = "BOM_BASE_URL", default_value = "http://localhost:8107")]
    bom_url: String,

    /// Production service base URL
    #[arg(long, env = "PRODUCTION_BASE_URL", default_value = "http://localhost:8108")]
    production_url: String,

    // --- AR-specific flags ---
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

/// Parse the --modules flag into a set of active module names.
fn resolve_modules(raw: &[String]) -> HashSet<String> {
    let mut set = HashSet::new();
    for m in raw {
        if m == "all" {
            for &name in MODULE_ORDER {
                set.insert(name.to_string());
            }
        } else {
            set.insert(m.clone());
        }
    }
    set
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
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

    let active_modules = resolve_modules(&cli.modules);

    info!(
        tenant = %cli.tenant,
        seed = cli.seed,
        modules = ?active_modules,
        "Starting demo-seed run",
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let mut tracker = digest::DigestTracker::new();
    // RNG is created just before AR seeding to preserve the exact call sequence
    // for backwards compatibility. Numbering doesn't use the RNG.

    // Execute modules in dependency order
    for &module_name in MODULE_ORDER {
        if !active_modules.contains(module_name) {
            continue;
        }

        match module_name {
            "numbering" => {
                let count = numbering::seed_numbering_policies(
                    &client,
                    &cli.numbering_url,
                    &mut tracker,
                )
                .await?;
                info!(count, "Numbering policies seeded");
            }
            "party" => {
                let party_ids = party::seed_parties(
                    &client,
                    &cli.party_url,
                    &mut tracker,
                )
                .await?;
                info!(
                    customers = party_ids.customers.len(),
                    suppliers = party_ids.suppliers.len(),
                    "Party seeding complete"
                );
            }
            "gl" | "inventory" | "bom" | "production" => {
                info!(module = module_name, "Module not yet implemented — skipping");
            }
            "ar" => {
                let mut rng = seed::DemoSeed::new(cli.seed);
                seed_ar(&client, &cli, &mut rng, &mut tracker).await?;
            }
            _ => {
                warn!(module = module_name, "Unknown module — skipping");
            }
        }
    }

    let digest = tracker.finalize();
    info!(digest = %digest, tenant = %cli.tenant, seed = cli.seed, "Demo-seed complete");
    println!("{}", digest);

    Ok(())
}

/// Run AR seeding (extracted to preserve exact original logic).
async fn seed_ar(
    client: &reqwest::Client,
    cli: &Cli,
    rng: &mut seed::DemoSeed,
    tracker: &mut digest::DigestTracker,
) -> Result<()> {
    for customer_idx in 0..cli.customers {
        let customer_corr_id = rng.correlation_id(&cli.tenant, "customer", customer_idx);

        let customer_id = ar::create_customer(
            client,
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
                client,
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
            info!(
                customer_idx,
                invoice_idx, invoice_id, amount_cents, "Created invoice"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_modules_all_expands() {
        let input = vec!["all".to_string()];
        let modules = resolve_modules(&input);
        for &name in MODULE_ORDER {
            assert!(modules.contains(name), "Missing module: {name}");
        }
    }

    #[test]
    fn resolve_modules_single() {
        let input = vec!["numbering".to_string()];
        let modules = resolve_modules(&input);
        assert_eq!(modules.len(), 1);
        assert!(modules.contains("numbering"));
    }

    #[test]
    fn resolve_modules_multiple() {
        let input = vec!["numbering".to_string(), "ar".to_string()];
        let modules = resolve_modules(&input);
        assert_eq!(modules.len(), 2);
        assert!(modules.contains("numbering"));
        assert!(modules.contains("ar"));
    }

    #[test]
    fn resolve_modules_deduplicates() {
        let input = vec!["ar".to_string(), "ar".to_string()];
        let modules = resolve_modules(&input);
        assert_eq!(modules.len(), 1);
    }

    #[test]
    fn module_order_includes_all_expected() {
        let expected = ["numbering", "gl", "party", "inventory", "bom", "production", "ar"];
        assert_eq!(MODULE_ORDER, &expected);
    }
}
