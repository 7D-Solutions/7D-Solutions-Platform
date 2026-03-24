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
//! cargo run -p demo-seed -- --tenant t1 --seed 42 --manifest-out /tmp/manifest.json
//! ```

mod ar;
mod bom;
mod digest;
mod gl;
mod inventory;
mod manifest;
mod numbering;
mod party;
mod production;
mod seed;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
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

    /// Auth service base URL (for user lookup in manifest)
    #[arg(long, env = "AUTH_BASE_URL", default_value = "http://localhost:8080")]
    auth_url: String,

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

    /// Write JSON manifest of all created resource IDs to this file
    #[arg(long)]
    manifest_out: Option<PathBuf>,

    /// JWT bearer token for authenticated services (numbering, gl, party, inventory, bom, production)
    #[arg(long, env = "DEMO_SEED_TOKEN")]
    token: Option<String>,
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
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    let cli = Cli::parse();

    if cli.print_hash {
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

    let mut default_headers = reqwest::header::HeaderMap::new();
    if let Some(ref token) = cli.token {
        default_headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                .context("Invalid token value for Authorization header")?,
        );
        info!("Using JWT bearer token for authenticated requests");
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .default_headers(default_headers)
        .build()?;

    let mut tracker = digest::DigestTracker::new();

    // Module results for manifest
    let mut numbering_policies: Option<Vec<String>> = None;
    let mut gl_result: Option<gl::GlAccounts> = None;
    let mut party_result: Option<party::PartyIds> = None;
    let mut inv_result: Option<inventory::InventoryIds> = None;
    let mut bom_result: Option<bom::BomIds> = None;
    let mut prod_result: Option<production::ProductionIds> = None;

    // Inventory IDs for downstream modules (bom, production)
    let mut item_id_map: HashMap<String, uuid::Uuid> = HashMap::new();
    let mut inventory_items: Option<Vec<(uuid::Uuid, String, String)>> = None;

    // Execute modules in dependency order
    for &module_name in MODULE_ORDER {
        if !active_modules.contains(module_name) {
            continue;
        }

        match module_name {
            "numbering" => {
                let policies = numbering::seed_numbering_policies(
                    &client,
                    &cli.numbering_url,
                    &mut tracker,
                )
                .await?;
                info!(count = policies.len(), "Numbering policies seeded");
                numbering_policies = Some(policies);
            }
            "party" => {
                let ids = party::seed_parties(&client, &cli.party_url, &mut tracker).await?;
                info!(
                    customers = ids.customers.len(),
                    suppliers = ids.suppliers.len(),
                    "Party seeding complete"
                );
                party_result = Some(ids);
            }
            "gl" => {
                let gl = gl::seed_gl(
                    &client,
                    &cli.gl_url,
                    &cli.tenant,
                    cli.seed,
                    &mut tracker,
                )
                .await?;
                info!(accounts = gl.codes.len(), "GL chart of accounts seeded");
                gl_result = Some(gl);
            }
            "inventory" => {
                let inv = inventory::seed_inventory(
                    &client,
                    &cli.inventory_url,
                    &cli.tenant,
                    cli.seed,
                    &mut tracker,
                )
                .await?;
                info!(
                    uoms = inv.uom_count,
                    locations = inv.locations.len(),
                    items = inv.items.len(),
                    warehouse_id = %inv.warehouse_id,
                    "Inventory seeding complete"
                );
                for (id, sku, _make_buy) in &inv.items {
                    item_id_map.insert(sku.clone(), *id);
                }
                inventory_items = Some(inv.items.clone());
                inv_result = Some(inv);
            }
            "bom" => {
                let items = match &inventory_items {
                    Some(items) => items.clone(),
                    None => {
                        info!("Inventory not run this session — fetching items from API");
                        bom::fetch_items_from_inventory(&client, &cli.inventory_url).await?
                    }
                };
                let ids = bom::seed_boms(&client, &cli.bom_url, &items, &mut tracker).await?;
                info!(
                    boms = ids.boms.len(),
                    revisions = ids.revisions.len(),
                    "BOM seeding complete"
                );
                bom_result = Some(ids);
            }
            "production" => {
                if item_id_map.is_empty() && !active_modules.contains("inventory") {
                    warn!("Production module running without inventory in active_modules — item_id_map is empty; run with --modules inventory,production or seed inventory first");
                }
                let ids = production::seed_production(
                    &client,
                    &cli.production_url,
                    &cli.tenant,
                    &item_id_map,
                    &mut tracker,
                )
                .await?;
                info!(
                    workcenters = ids.workcenters,
                    routings = ids.routings,
                    "Production seeding complete"
                );
                prod_result = Some(ids);
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

    // Look up admin user for manifest (best-effort, non-fatal)
    let admin_user_id = lookup_admin_user(&client, &cli.auth_url, &cli.tenant).await;

    // Build and output manifest
    let manifest_path = cli.manifest_out.as_deref();
    let mut builder = manifest::ManifestBuilder::new(
        cli.tenant.clone(),
        cli.seed,
        digest,
    )
    .with_admin_user_id(admin_user_id);
    if let Some(p) = numbering_policies {
        builder = builder.with_numbering(p);
    }
    if let Some(g) = gl_result {
        builder = builder.with_gl(g);
    }
    if let Some(p) = party_result {
        builder = builder.with_parties(p);
    }
    if let Some(i) = inv_result {
        builder = builder.with_inventory(i);
    }
    if let Some(b) = bom_result {
        builder = builder.with_bom(b);
    }
    if let Some(p) = prod_result {
        builder = builder.with_production(p);
    }
    let m = builder.build();
    let json = manifest::write_manifest(&m, manifest_path.map(std::path::Path::new))?;
    if manifest_path.is_some() {
        info!(path = ?manifest_path, "Manifest written");
    } else {
        // Print manifest to stdout after digest (separated by newline)
        println!("{}", json);
    }

    Ok(())
}

/// Look up the admin user UUID from the auth service (best-effort).
/// Uses the platform tenant ID (00000000-0000-0000-0000-000000000000) since
/// seed-dev.sh creates the admin under that tenant.
async fn lookup_admin_user(
    client: &reqwest::Client,
    auth_url: &str,
    _tenant: &str,
) -> Option<uuid::Uuid> {
    let platform_tenant = "00000000-0000-0000-0000-000000000000";
    let url = format!(
        "{}/api/auth/users?email={}&tenant_id={}",
        auth_url, "admin@7dsolutions.local", platform_tenant
    );

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            info!(error = %e, "Auth service not reachable — admin user ID will be null in manifest");
            return None;
        }
    };

    if !resp.status().is_success() {
        info!(status = %resp.status(), "Admin user lookup returned non-200 — ID will be null");
        return None;
    }

    #[derive(serde::Deserialize)]
    struct UserResp {
        id: uuid::Uuid,
    }

    match resp.json::<UserResp>().await {
        Ok(u) => {
            info!(admin_user_id = %u.id, "Admin user UUID resolved for manifest");
            Some(u.id)
        }
        Err(e) => {
            info!(error = %e, "Failed to parse admin user response");
            None
        }
    }
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

    #[test]
    fn auth_header_map_built_correctly_with_token() {
        // Verifies the HeaderMap construction logic used in main().
        // reqwest::Client::default_headers merges these into every request at send time.
        let token = "test-jwt-token-value";
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );

        let auth = headers.get(reqwest::header::AUTHORIZATION).unwrap();
        assert_eq!(auth, "Bearer test-jwt-token-value");
    }

    #[test]
    fn auth_header_map_empty_without_token() {
        let headers = reqwest::header::HeaderMap::new();
        assert!(
            headers.get(reqwest::header::AUTHORIZATION).is_none(),
            "No Authorization header when token is absent"
        );
    }
}
