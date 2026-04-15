//! E2E test for compliance export determinism
//!
//! This test verifies that compliance exports are deterministic:
//! - Two exports of the same data produce identical checksums
//! - Exports are tenant-scoped (no data leakage)
//! - Manifest includes correct checksums and counts

mod common;

use common::{get_ar_db_url, get_audit_db_url, get_gl_db_url, get_payments_db_url};
use compliance_export::export_compliance_data;
use serde_json::Value;
use sqlx::PgPool;
use std::fs;
use uuid::Uuid;

// ============================================================================
// Test Setup
// ============================================================================

async fn setup_test_data(
    ar_pool: &PgPool,
    payments_pool: &PgPool,
    gl_pool: &PgPool,
    audit_pool: &PgPool,
    tenant_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create AR customer
    let customer_id = sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status)
        VALUES ($1, 'test@example.com', 'Test Customer', 'active')
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .fetch_one(ar_pool)
    .await?;

    // Create AR invoice
    sqlx::query(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id,
            status, amount_cents, currency, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, NOW())
        "#,
    )
    .bind(tenant_id)
    .bind(format!("inv-test-{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind("paid")
    .bind(10000)
    .bind("usd")
    .execute(ar_pool)
    .await?;

    // Create payment attempt
    let payment_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO payment_attempts (
            app_id, payment_id, invoice_id, attempt_no, status, attempted_at
        ) VALUES ($1, $2, $3, $4, $5::payment_attempt_status, NOW())
        "#,
    )
    .bind(tenant_id)
    .bind(payment_id)
    .bind("inv-12345")
    .bind(0)
    .bind("succeeded")
    .execute(payments_pool)
    .await?;

    // Create GL journal entry
    let entry_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO journal_entries (
            id, tenant_id, source_module, source_event_id,
            source_subject, posted_at, currency
        ) VALUES ($1, $2, $3, $4, $5, NOW(), $6)
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind("ar")
    .bind(Uuid::new_v4())
    .bind("invoice.created")
    .bind("usd")
    .execute(gl_pool)
    .await?;

    // Create audit event
    sqlx::query(
        r#"
        INSERT INTO audit_events (
            actor_id, actor_type, action, mutation_class,
            entity_type, entity_id,
            before_snapshot, after_snapshot,
            metadata
        ) VALUES ($1, $2, $3, $4::mutation_class, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(Uuid::nil())
    .bind("system")
    .bind("invoice.created")
    .bind("CREATE")
    .bind("invoice")
    .bind(tenant_id)
    .bind(serde_json::json!({}))
    .bind(serde_json::json!({"status": "created"}))
    .bind(serde_json::json!({"tenant_id": tenant_id}))
    .execute(audit_pool)
    .await?;

    Ok(())
}

async fn cleanup_test_data(
    ar_pool: &PgPool,
    payments_pool: &PgPool,
    gl_pool: &PgPool,
    audit_pool: &PgPool,
    tenant_id: &str,
) {
    // Cleanup in reverse order of foreign keys
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(tenant_id)
        .execute(payments_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(gl_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM audit_events WHERE entity_id = $1")
        .bind(tenant_id)
        .execute(audit_pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_compliance_export_determinism() {
    // Resolve DB URLs with local-dev defaults (no panic if env vars absent).
    let ar_url = get_ar_db_url();
    let payments_url = get_payments_db_url();
    let gl_url = get_gl_db_url();
    let audit_url = get_audit_db_url();

    // Propagate resolved URLs so export_compliance_data() can read them.
    std::env::set_var("AR_DATABASE_URL", &ar_url);
    std::env::set_var("PAYMENTS_DATABASE_URL", &payments_url);
    std::env::set_var("GL_DATABASE_URL", &gl_url);
    std::env::set_var("PLATFORM_AUDIT_DATABASE_URL", &audit_url);

    let ar_pool = PgPool::connect(&ar_url)
        .await
        .expect("Failed to connect to AR database");
    let payments_pool = PgPool::connect(&payments_url)
        .await
        .expect("Failed to connect to payments database");
    let gl_pool = PgPool::connect(&gl_url)
        .await
        .expect("Failed to connect to GL database");
    let audit_pool = PgPool::connect(&audit_url)
        .await
        .expect("Failed to connect to audit database");

    let tenant_id = format!("test-tenant-{}", Uuid::new_v4());

    // Cleanup before and after
    cleanup_test_data(&ar_pool, &payments_pool, &gl_pool, &audit_pool, &tenant_id).await;

    // Setup test data
    setup_test_data(&ar_pool, &payments_pool, &gl_pool, &audit_pool, &tenant_id)
        .await
        .expect("Failed to setup test data");

    // Create temp directories for two exports
    let temp_dir = std::env::temp_dir();
    let export1_dir = temp_dir.join(format!("compliance-export-1-{}", Uuid::new_v4()));
    let export2_dir = temp_dir.join(format!("compliance-export-2-{}", Uuid::new_v4()));

    // Run first export
    export_compliance_data(
        &tenant_id,
        export1_dir.to_str().unwrap(),
        "json",
        None,
        None,
    )
    .await
    .expect("First export failed");

    // Run second export
    export_compliance_data(
        &tenant_id,
        export2_dir.to_str().unwrap(),
        "json",
        None,
        None,
    )
    .await
    .expect("Second export failed");

    // Read manifests
    let manifest1_path = export1_dir.join("manifest.json");
    let manifest2_path = export2_dir.join("manifest.json");

    let manifest1_content =
        fs::read_to_string(&manifest1_path).expect("Failed to read first manifest");
    let manifest2_content =
        fs::read_to_string(&manifest2_path).expect("Failed to read second manifest");

    let manifest1: Value =
        serde_json::from_str(&manifest1_content).expect("Failed to parse first manifest");
    let manifest2: Value =
        serde_json::from_str(&manifest2_content).expect("Failed to parse second manifest");

    // Assert: Checksums are identical (determinism)
    assert_eq!(
        manifest1["audit_events_checksum"], manifest2["audit_events_checksum"],
        "Audit events checksums don't match - export is not deterministic"
    );

    assert_eq!(
        manifest1["ar_invoices_checksum"], manifest2["ar_invoices_checksum"],
        "AR invoices checksums don't match - export is not deterministic"
    );

    assert_eq!(
        manifest1["payment_attempts_checksum"], manifest2["payment_attempts_checksum"],
        "Payment attempts checksums don't match - export is not deterministic"
    );

    assert_eq!(
        manifest1["journal_entries_checksum"], manifest2["journal_entries_checksum"],
        "Journal entries checksums don't match - export is not deterministic"
    );

    // Assert: Counts are correct
    assert!(
        manifest1["audit_events_count"].as_u64().unwrap() >= 1,
        "Expected at least 1 audit event"
    );
    assert!(
        manifest1["ar_invoices_count"].as_u64().unwrap() >= 1,
        "Expected at least 1 AR invoice"
    );
    assert!(
        manifest1["payment_attempts_count"].as_u64().unwrap() >= 1,
        "Expected at least 1 payment attempt"
    );
    assert!(
        manifest1["journal_entries_count"].as_u64().unwrap() >= 1,
        "Expected at least 1 journal entry"
    );

    // Assert: Export files exist
    assert!(export1_dir.join("audit_events.jsonl").exists());
    assert!(export1_dir.join("ar_invoices.jsonl").exists());
    assert!(export1_dir.join("payment_attempts.jsonl").exists());
    assert!(export1_dir.join("journal_entries.jsonl").exists());

    // Cleanup
    cleanup_test_data(&ar_pool, &payments_pool, &gl_pool, &audit_pool, &tenant_id).await;
    fs::remove_dir_all(&export1_dir).ok();
    fs::remove_dir_all(&export2_dir).ok();

    println!("✓ Compliance export determinism test passed");
}

#[tokio::test]
async fn test_compliance_export_tenant_isolation() {
    // Resolve DB URLs with local-dev defaults (no panic if env vars absent).
    let ar_url = get_ar_db_url();
    let payments_url = get_payments_db_url();
    let gl_url = get_gl_db_url();
    let audit_url = get_audit_db_url();

    // Propagate resolved URLs so export_compliance_data() can read them.
    std::env::set_var("AR_DATABASE_URL", &ar_url);
    std::env::set_var("PAYMENTS_DATABASE_URL", &payments_url);
    std::env::set_var("GL_DATABASE_URL", &gl_url);
    std::env::set_var("PLATFORM_AUDIT_DATABASE_URL", &audit_url);

    let ar_pool = PgPool::connect(&ar_url)
        .await
        .expect("Failed to connect to AR database");
    let payments_pool = PgPool::connect(&payments_url)
        .await
        .expect("Failed to connect to payments database");
    let gl_pool = PgPool::connect(&gl_url)
        .await
        .expect("Failed to connect to GL database");
    let audit_pool = PgPool::connect(&audit_url)
        .await
        .expect("Failed to connect to audit database");

    let tenant1_id = format!("test-tenant-1-{}", Uuid::new_v4());
    let tenant2_id = format!("test-tenant-2-{}", Uuid::new_v4());

    // Cleanup
    cleanup_test_data(&ar_pool, &payments_pool, &gl_pool, &audit_pool, &tenant1_id).await;
    cleanup_test_data(&ar_pool, &payments_pool, &gl_pool, &audit_pool, &tenant2_id).await;

    // Setup data for both tenants
    setup_test_data(&ar_pool, &payments_pool, &gl_pool, &audit_pool, &tenant1_id)
        .await
        .expect("Failed to setup tenant1 data");
    setup_test_data(&ar_pool, &payments_pool, &gl_pool, &audit_pool, &tenant2_id)
        .await
        .expect("Failed to setup tenant2 data");

    // Export tenant1 data
    let temp_dir = std::env::temp_dir();
    let export_dir = temp_dir.join(format!("compliance-export-isolation-{}", Uuid::new_v4()));

    export_compliance_data(
        &tenant1_id,
        export_dir.to_str().unwrap(),
        "json",
        None,
        None,
    )
    .await
    .expect("Export failed");

    // Read exported data and verify tenant isolation
    let ar_invoices_path = export_dir.join("ar_invoices.jsonl");
    let ar_invoices_content =
        fs::read_to_string(&ar_invoices_path).expect("Failed to read AR invoices");

    // Parse each line and check app_id
    for line in ar_invoices_content.lines() {
        let invoice: Value = serde_json::from_str(line).expect("Failed to parse invoice");
        assert_eq!(
            invoice["app_id"].as_str().unwrap(),
            tenant1_id,
            "Found invoice from wrong tenant - tenant isolation violated"
        );
    }

    // Cleanup
    cleanup_test_data(&ar_pool, &payments_pool, &gl_pool, &audit_pool, &tenant1_id).await;
    cleanup_test_data(&ar_pool, &payments_pool, &gl_pool, &audit_pool, &tenant2_id).await;
    fs::remove_dir_all(&export_dir).ok();

    println!("✓ Compliance export tenant isolation test passed");
}
