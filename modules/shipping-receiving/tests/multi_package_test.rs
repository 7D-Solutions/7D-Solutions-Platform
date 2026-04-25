//! Integration tests: multi-package shipments with master + child tracking (bd-pv2yi).
//!
//! Tests 1–3 use a real PostgreSQL database. No mocks.
//! Tests 4–6 require real carrier sandbox credentials and are #[ignore] by default.
//!
//! Required env var (falls back to default container URL):
//!   DATABASE_URL — postgres://shipping_receiving_user:shipping_receiving_pass@localhost:5454/shipping_receiving_db
//!
//! Run all non-ignored tests:
//!   ./scripts/cargo-slot.sh test -p shipping-receiving-rs --test multi_package_test
//!
//! Run sandbox carrier tests (need credentials):
//!   UPS_CLIENT_ID=... UPS_CLIENT_SECRET=... UPS_ACCOUNT_NUMBER=... \
//!   FEDEX_CLIENT_ID=... FEDEX_CLIENT_SECRET=... FEDEX_ACCOUNT_NUMBER=... \
//!   RL_API_KEY=... \
//!   ./scripts/cargo-slot.sh test -p shipping-receiving-rs --test multi_package_test -- --include-ignored

use serial_test::serial;
use shipping_receiving_rs::domain::carrier_providers::{
    get_provider, MultiPackageLabelRequest, PackageInfo,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ── DB helpers ────────────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://shipping_receiving_user:shipping_receiving_pass@localhost:5454/shipping_receiving_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to shipping-receiving test DB");
    let _ = sqlx::migrate!("db/migrations").run(&pool).await;
    pool
}

fn sample_origin() -> serde_json::Value {
    serde_json::json!({
        "name": "Test Warehouse",
        "address": "123 Main St",
        "city": "Atlanta",
        "state": "GA",
        "zip": "30301"
    })
}

fn sample_destination() -> serde_json::Value {
    serde_json::json!({
        "name": "Test Customer",
        "address": "456 Market St",
        "city": "San Francisco",
        "state": "CA",
        "zip": "94105"
    })
}

fn three_packages() -> Vec<PackageInfo> {
    vec![
        PackageInfo { weight_lbs: 10.0, length_in: 12.0, width_in: 12.0, height_in: 12.0, declared_value_cents: None },
        PackageInfo { weight_lbs: 8.0,  length_in: 10.0, width_in: 10.0, height_in: 10.0, declared_value_cents: None },
        PackageInfo { weight_lbs: 6.0,  length_in: 8.0,  width_in: 8.0,  height_in: 8.0,  declared_value_cents: None },
    ]
}

async fn cleanup_shipment(pool: &sqlx::PgPool, shipment_id: Uuid) {
    let _ = sqlx::query("DELETE FROM shipments WHERE parent_shipment_id = $1")
        .bind(shipment_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM shipments WHERE id = $1")
        .bind(shipment_id)
        .execute(pool)
        .await;
}

// ── 1. DB: master + children persist correctly ────────────────

#[tokio::test]
#[serial]
async fn multi_package_db_creates_master_with_children() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let master_tn = format!("MASTER-{}", Uuid::new_v4().simple());
    let package_count = 3i32;

    // Insert master row directly (simulates what the HTTP handler does)
    let master_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO shipments
            (tenant_id, direction, status, tracking_number,
             master_tracking_number, package_count)
        VALUES ($1, 'outbound', 'draft', $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(&master_tn)
    .bind(&master_tn)
    .bind(package_count)
    .fetch_one(&pool)
    .await
    .expect("insert master");

    // Insert child rows
    let child_tns = [
        format!("CHILD-A-{}", Uuid::new_v4().simple()),
        format!("CHILD-B-{}", Uuid::new_v4().simple()),
        format!("CHILD-C-{}", Uuid::new_v4().simple()),
    ];
    let mut child_ids: Vec<Uuid> = Vec::new();
    for tn in &child_tns {
        let child_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO shipments
                (tenant_id, direction, status, tracking_number,
                 parent_shipment_id, package_count)
            VALUES ($1, 'outbound', 'draft', $2, $3, 1)
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(tn)
        .bind(master_id)
        .fetch_one(&pool)
        .await
        .expect("insert child");
        child_ids.push(child_id);
    }

    // Verify master row
    let (db_master_tn, db_pkg_count): (Option<String>, i32) = sqlx::query_as(
        "SELECT master_tracking_number, package_count FROM shipments WHERE id = $1",
    )
    .bind(master_id)
    .fetch_one(&pool)
    .await
    .expect("fetch master");

    assert_eq!(
        db_master_tn.as_deref(),
        Some(master_tn.as_str()),
        "master_tracking_number must be set on master row"
    );
    assert_eq!(db_pkg_count, 3, "package_count must be 3");

    // Verify children reference master
    let children_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM shipments WHERE parent_shipment_id = $1",
    )
    .bind(master_id)
    .fetch_one(&pool)
    .await
    .expect("count children");

    assert_eq!(children_count, 3, "must have exactly 3 child rows");

    // Verify each child has parent_shipment_id pointing to master
    for child_id in &child_ids {
        let (parent_id,): (Option<Uuid>,) =
            sqlx::query_as("SELECT parent_shipment_id FROM shipments WHERE id = $1")
                .bind(child_id)
                .fetch_one(&pool)
                .await
                .expect("fetch child");
        assert_eq!(
            parent_id,
            Some(master_id),
            "child must reference master via parent_shipment_id"
        );
    }

    cleanup_shipment(&pool, master_id).await;
}

// ── 2. Stub provider: returns N children for N packages ───────

#[tokio::test]
async fn stub_multi_package_returns_master_and_n_children() {
    let provider = get_provider("stub").expect("stub must be registered");
    let req = MultiPackageLabelRequest {
        packages: three_packages(),
        origin: sample_origin(),
        destination: sample_destination(),
        service_level: None,
        billing_ref: None,
    };
    let resp = provider
        .create_multi_package_label(&req, &serde_json::json!({}))
        .await
        .expect("stub multi-package label must succeed");

    assert!(
        !resp.master_tracking_number.is_empty(),
        "master_tracking_number must not be empty"
    );
    assert_eq!(
        resp.children.len(),
        3,
        "stub must return one child per package"
    );
    for (i, child) in resp.children.iter().enumerate() {
        assert_eq!(child.package_index, i, "child package_index must match position");
        assert!(!child.tracking_number.is_empty(), "child tracking_number must not be empty");
    }
}

// ── 3. Stub: empty packages returns InvalidRequest ────────────

#[tokio::test]
async fn stub_multi_package_empty_packages_returns_error() {
    let provider = get_provider("stub").expect("stub must be registered");
    let req = MultiPackageLabelRequest {
        packages: vec![],
        origin: sample_origin(),
        destination: sample_destination(),
        service_level: None,
        billing_ref: None,
    };
    let result = provider
        .create_multi_package_label(&req, &serde_json::json!({}))
        .await;
    assert!(
        result.is_err(),
        "empty packages must return an error"
    );
}

// ── 4. LTL: R&L multi-package returns single pro_number ──────
//
// Requires RL_SANDBOX_API_KEY in env. Run with --include-ignored.

#[tokio::test]
#[ignore]
async fn ltl_multi_handling_units_returns_single_pro() {
    let api_key = std::env::var("RL_SANDBOX_API_KEY")
        .expect("RL_SANDBOX_API_KEY must be set");
    let base_url = std::env::var("RL_SANDBOX_URL")
        .unwrap_or_else(|_| "https://api.rlcarriers.com".to_string());

    let provider = get_provider("rl").expect("rl provider must be registered");
    let config = serde_json::json!({ "api_key": api_key, "base_url": base_url });

    let req = MultiPackageLabelRequest {
        packages: vec![
            PackageInfo { weight_lbs: 500.0, length_in: 48.0, width_in: 40.0, height_in: 48.0, declared_value_cents: None },
            PackageInfo { weight_lbs: 400.0, length_in: 48.0, width_in: 40.0, height_in: 40.0, declared_value_cents: None },
            PackageInfo { weight_lbs: 350.0, length_in: 40.0, width_in: 40.0, height_in: 36.0, declared_value_cents: None },
            PackageInfo { weight_lbs: 600.0, length_in: 48.0, width_in: 48.0, height_in: 48.0, declared_value_cents: None },
            PackageInfo { weight_lbs: 450.0, length_in: 48.0, width_in: 40.0, height_in: 40.0, declared_value_cents: None },
        ],
        origin: sample_origin(),
        destination: sample_destination(),
        service_level: None,
        billing_ref: None,
    };

    let resp = provider
        .create_multi_package_label(&req, &config)
        .await
        .expect("R&L multi-package BOL must succeed");

    assert!(
        !resp.master_tracking_number.is_empty(),
        "R&L must return a pro_number as master_tracking_number"
    );
    assert!(
        resp.children.is_empty(),
        "R&L LTL must return empty children (one pro_number per BOL regardless of pallet count)"
    );

    println!("R&L pro_number: {}", resp.master_tracking_number);
}

// ── 5. UPS multi-package sandbox ─────────────────────────────
//
// Requires UPS_CLIENT_ID, UPS_CLIENT_SECRET, UPS_ACCOUNT_NUMBER in env.

#[tokio::test]
#[ignore]
async fn ups_multi_package_returns_master_and_children() {
    let client_id = std::env::var("UPS_CLIENT_ID")
        .expect("UPS_CLIENT_ID must be set");
    let client_secret = std::env::var("UPS_CLIENT_SECRET")
        .expect("UPS_CLIENT_SECRET must be set");
    let account_number = std::env::var("UPS_ACCOUNT_NUMBER")
        .expect("UPS_ACCOUNT_NUMBER must be set");

    let provider = get_provider("ups").expect("ups provider must be registered");
    let config = serde_json::json!({
        "client_id": client_id,
        "client_secret": client_secret,
        "account_number": account_number,
        "base_url": "https://wwwcie.ups.com"
    });

    let req = MultiPackageLabelRequest {
        packages: three_packages(),
        origin: serde_json::json!({
            "name": "Test Shipper", "address": "123 Main St",
            "city": "New York", "state": "NY", "zip": "10001"
        }),
        destination: serde_json::json!({
            "name": "Test Recipient", "address": "456 Sunset Blvd",
            "city": "Los Angeles", "state": "CA", "zip": "90210"
        }),
        service_level: Some("03".to_string()),
        billing_ref: None,
    };

    let resp = provider
        .create_multi_package_label(&req, &config)
        .await
        .expect("UPS multi-package label must succeed");

    assert!(
        !resp.master_tracking_number.is_empty(),
        "UPS must return a ShipmentIdentificationNumber as master"
    );
    assert_eq!(
        resp.children.len(),
        3,
        "UPS must return one child label per package"
    );
    for child in &resp.children {
        assert!(
            child.tracking_number.starts_with("1Z"),
            "UPS tracking numbers must start with '1Z'"
        );
    }
    println!("UPS master: {} children: {:?}",
        resp.master_tracking_number,
        resp.children.iter().map(|c| &c.tracking_number).collect::<Vec<_>>()
    );
}

// ── 6. FedEx multi-package sandbox ───────────────────────────
//
// Requires FEDEX_CLIENT_ID, FEDEX_CLIENT_SECRET, FEDEX_ACCOUNT_NUMBER in env.

#[tokio::test]
#[ignore]
async fn fedex_multi_package_returns_master_and_children() {
    let client_id = std::env::var("FEDEX_CLIENT_ID")
        .expect("FEDEX_CLIENT_ID must be set");
    let client_secret = std::env::var("FEDEX_CLIENT_SECRET")
        .expect("FEDEX_CLIENT_SECRET must be set");
    let account_number = std::env::var("FEDEX_ACCOUNT_NUMBER")
        .expect("FEDEX_ACCOUNT_NUMBER must be set");

    let provider = get_provider("fedex").expect("fedex provider must be registered");
    let config = serde_json::json!({
        "client_id": client_id,
        "client_secret": client_secret,
        "account_number": account_number,
        "base_url": "https://apis-sandbox.fedex.com"
    });

    let req = MultiPackageLabelRequest {
        packages: three_packages(),
        origin: serde_json::json!({
            "name": "Test Shipper", "address": "123 Main St",
            "city": "New York", "state": "NY", "zip": "10001"
        }),
        destination: serde_json::json!({
            "name": "Test Recipient", "address": "456 Sunset Blvd",
            "city": "Los Angeles", "state": "CA", "zip": "90210"
        }),
        service_level: Some("FEDEX_GROUND".to_string()),
        billing_ref: None,
    };

    let resp = provider
        .create_multi_package_label(&req, &config)
        .await
        .expect("FedEx multi-package label must succeed");

    assert!(
        !resp.master_tracking_number.is_empty(),
        "FedEx must return a masterTrackingNumber"
    );
    assert_eq!(
        resp.children.len(),
        3,
        "FedEx must return one child label per package"
    );
    println!("FedEx master: {} children: {:?}",
        resp.master_tracking_number,
        resp.children.iter().map(|c| &c.tracking_number).collect::<Vec<_>>()
    );
}
