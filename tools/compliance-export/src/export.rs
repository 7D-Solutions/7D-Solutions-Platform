//! Compliance export: audit + ledger data extraction per tenant
//!
//! This module provides deterministic exports of audit logs and core ledger data
//! (AR invoices, payment attempts, GL journal entries) for compliance and regulatory purposes.
//!
//! Invariant: Exports are tenant-scoped, complete, and tamper-evident with stable ordering.

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::path::Path;
use uuid::Uuid;

// ============================================================================
// Data Structures
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuditEvent {
    pub audit_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub actor_id: Uuid,
    pub actor_type: String,
    pub action: String,
    pub mutation_class: String,
    pub entity_type: String,
    pub entity_id: String,
    pub before_snapshot: Option<serde_json::Value>,
    pub after_snapshot: Option<serde_json::Value>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub causation_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
    pub trace_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ArInvoice {
    pub id: i32,
    pub app_id: String,
    pub tilled_invoice_id: String,
    pub ar_customer_id: i32,
    pub subscription_id: Option<i32>,
    pub status: String,
    pub amount_cents: i32,
    pub currency: String,
    pub due_at: Option<NaiveDateTime>,
    pub paid_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub billing_period_start: Option<NaiveDateTime>,
    pub billing_period_end: Option<NaiveDateTime>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PaymentAttempt {
    pub id: Uuid,
    pub app_id: String,
    pub payment_id: Uuid,
    pub invoice_id: String,
    pub attempt_no: i32,
    pub status: String,
    pub attempted_at: NaiveDateTime,
    pub completed_at: Option<NaiveDateTime>,
    pub processor_payment_id: Option<String>,
    pub payment_method_ref: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JournalEntry {
    pub id: Uuid,
    pub tenant_id: String,
    pub source_module: String,
    pub source_event_id: Uuid,
    pub source_subject: String,
    pub posted_at: DateTime<Utc>,
    pub currency: String,
    pub description: Option<String>,
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ExportManifest {
    pub tenant_id: String,
    pub export_timestamp: DateTime<Utc>,
    pub format: String,
    pub audit_events_count: usize,
    pub audit_events_checksum: String,
    pub ar_invoices_count: usize,
    pub ar_invoices_checksum: String,
    pub payment_attempts_count: usize,
    pub payment_attempts_checksum: String,
    pub journal_entries_count: usize,
    pub journal_entries_checksum: String,
}

// ============================================================================
// Database Query Functions
// ============================================================================

/// Fetch all audit events for a tenant (filtered by entity_id pattern or metadata)
/// Orders by occurred_at ASC, then audit_id ASC for determinism
async fn fetch_audit_events(pool: &PgPool, tenant_id: &str) -> Result<Vec<AuditEvent>> {
    let rows = sqlx::query(
        r#"
        SELECT
            audit_id,
            occurred_at,
            actor_id,
            actor_type,
            action,
            mutation_class::text as mutation_class,
            entity_type,
            entity_id,
            before_snapshot,
            after_snapshot,
            before_hash,
            after_hash,
            causation_id,
            correlation_id,
            trace_id,
            metadata
        FROM audit_events
        WHERE
            entity_id LIKE '%' || $1 || '%'
            OR (metadata->>'tenant_id')::text = $1
        ORDER BY occurred_at ASC, audit_id ASC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .context("Failed to fetch audit events")?;

    let events = rows
        .into_iter()
        .map(|row| -> Result<AuditEvent> {
            Ok(AuditEvent {
                audit_id: row.try_get("audit_id").context("audit_id")?,
                occurred_at: row.try_get("occurred_at").context("occurred_at")?,
                actor_id: row.try_get("actor_id").context("actor_id")?,
                actor_type: row.try_get("actor_type").context("actor_type")?,
                action: row.try_get("action").context("action")?,
                mutation_class: row.try_get("mutation_class").context("mutation_class")?,
                entity_type: row.try_get("entity_type").context("entity_type")?,
                entity_id: row.try_get("entity_id").context("entity_id")?,
                before_snapshot: row.try_get("before_snapshot").context("before_snapshot")?,
                after_snapshot: row.try_get("after_snapshot").context("after_snapshot")?,
                before_hash: row.try_get("before_hash").context("before_hash")?,
                after_hash: row.try_get("after_hash").context("after_hash")?,
                causation_id: row.try_get("causation_id").context("causation_id")?,
                correlation_id: row.try_get("correlation_id").context("correlation_id")?,
                trace_id: row.try_get("trace_id").context("trace_id")?,
                metadata: row.try_get("metadata").context("metadata")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(events)
}

/// Fetch all AR invoices for a tenant
/// Orders by created_at ASC, then id ASC for determinism
async fn fetch_ar_invoices(pool: &PgPool, tenant_id: &str) -> Result<Vec<ArInvoice>> {
    let rows = sqlx::query(
        r#"
        SELECT
            id,
            app_id,
            tilled_invoice_id,
            ar_customer_id,
            subscription_id,
            status,
            amount_cents,
            currency,
            due_at,
            paid_at,
            created_at,
            updated_at,
            billing_period_start,
            billing_period_end
        FROM ar_invoices
        WHERE app_id = $1
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .context("Failed to fetch AR invoices")?;

    let invoices = rows
        .into_iter()
        .map(|row| -> Result<ArInvoice> {
            Ok(ArInvoice {
                id: row.try_get("id").context("id")?,
                app_id: row.try_get("app_id").context("app_id")?,
                tilled_invoice_id: row
                    .try_get("tilled_invoice_id")
                    .context("tilled_invoice_id")?,
                ar_customer_id: row.try_get("ar_customer_id").context("ar_customer_id")?,
                subscription_id: row.try_get("subscription_id").context("subscription_id")?,
                status: row.try_get("status").context("status")?,
                amount_cents: row.try_get("amount_cents").context("amount_cents")?,
                currency: row.try_get("currency").context("currency")?,
                due_at: row.try_get("due_at").context("due_at")?,
                paid_at: row.try_get("paid_at").context("paid_at")?,
                created_at: row.try_get("created_at").context("created_at")?,
                updated_at: row.try_get("updated_at").context("updated_at")?,
                billing_period_start: row
                    .try_get("billing_period_start")
                    .context("billing_period_start")?,
                billing_period_end: row
                    .try_get("billing_period_end")
                    .context("billing_period_end")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(invoices)
}

/// Fetch all payment attempts for a tenant
/// Orders by attempted_at ASC, then id ASC for determinism
async fn fetch_payment_attempts(pool: &PgPool, tenant_id: &str) -> Result<Vec<PaymentAttempt>> {
    let rows = sqlx::query(
        r#"
        SELECT
            id,
            app_id,
            payment_id,
            invoice_id,
            attempt_no,
            status::text as status,
            attempted_at,
            completed_at,
            processor_payment_id,
            payment_method_ref,
            failure_code,
            failure_message,
            created_at,
            updated_at
        FROM payment_attempts
        WHERE app_id = $1
        ORDER BY attempted_at ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .context("Failed to fetch payment attempts")?;

    let attempts = rows
        .into_iter()
        .map(|row| -> Result<PaymentAttempt> {
            Ok(PaymentAttempt {
                id: row.try_get("id").context("id")?,
                app_id: row.try_get("app_id").context("app_id")?,
                payment_id: row.try_get("payment_id").context("payment_id")?,
                invoice_id: row.try_get("invoice_id").context("invoice_id")?,
                attempt_no: row.try_get("attempt_no").context("attempt_no")?,
                status: row.try_get("status").context("status")?,
                attempted_at: row.try_get("attempted_at").context("attempted_at")?,
                completed_at: row.try_get("completed_at").context("completed_at")?,
                processor_payment_id: row
                    .try_get("processor_payment_id")
                    .context("processor_payment_id")?,
                payment_method_ref: row
                    .try_get("payment_method_ref")
                    .context("payment_method_ref")?,
                failure_code: row.try_get("failure_code").context("failure_code")?,
                failure_message: row.try_get("failure_message").context("failure_message")?,
                created_at: row.try_get("created_at").context("created_at")?,
                updated_at: row.try_get("updated_at").context("updated_at")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(attempts)
}

/// Fetch all GL journal entries for a tenant
/// Orders by posted_at ASC, then id ASC for determinism
async fn fetch_journal_entries(pool: &PgPool, tenant_id: &str) -> Result<Vec<JournalEntry>> {
    let rows = sqlx::query(
        r#"
        SELECT
            id,
            tenant_id,
            source_module,
            source_event_id,
            source_subject,
            posted_at,
            currency,
            description,
            reference_type,
            reference_id,
            created_at
        FROM journal_entries
        WHERE tenant_id = $1
        ORDER BY posted_at ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .context("Failed to fetch journal entries")?;

    let entries = rows
        .into_iter()
        .map(|row| -> Result<JournalEntry> {
            Ok(JournalEntry {
                id: row.try_get("id").context("id")?,
                tenant_id: row.try_get("tenant_id").context("tenant_id")?,
                source_module: row.try_get("source_module").context("source_module")?,
                source_event_id: row.try_get("source_event_id").context("source_event_id")?,
                source_subject: row.try_get("source_subject").context("source_subject")?,
                posted_at: row.try_get("posted_at").context("posted_at")?,
                currency: row.try_get("currency").context("currency")?,
                description: row.try_get("description").context("description")?,
                reference_type: row.try_get("reference_type").context("reference_type")?,
                reference_id: row.try_get("reference_id").context("reference_id")?,
                created_at: row.try_get("created_at").context("created_at")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(entries)
}

// ============================================================================
// Export Functions
// ============================================================================

/// Calculate SHA256 checksum of serialized data
fn calculate_checksum<T: Serialize>(data: &[T]) -> Result<String> {
    let json = serde_json::to_string(data)?;
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

/// Export data to JSON Lines format
fn export_jsonl<T: Serialize>(data: &[T], path: &Path) -> Result<()> {
    use std::fs::File;
    use std::io::{BufWriter, Write};

    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    for item in data {
        let line = serde_json::to_string(item)?;
        writeln!(writer, "{}", line)?;
    }

    writer.flush()?;
    Ok(())
}

/// Export data to CSV format
fn export_csv<T: Serialize>(data: &[T], path: &Path) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;

    for item in data {
        wtr.serialize(item)?;
    }

    wtr.flush()?;
    Ok(())
}

/// Export manifest to JSON
fn export_manifest(manifest: &ExportManifest, path: &Path) -> Result<()> {
    use std::fs::File;
    use std::io::BufWriter;

    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, manifest)?;
    Ok(())
}

/// Main export function
pub async fn export_compliance_data(tenant_id: &str, output_dir: &str, format: &str) -> Result<()> {
    tracing::info!(
        tenant_id = %tenant_id,
        output_dir = %output_dir,
        format = %format,
        "Starting compliance export"
    );

    // Create output directory
    std::fs::create_dir_all(output_dir)?;

    // Connect to databases
    let audit_url = std::env::var("PLATFORM_AUDIT_DATABASE_URL")
        .context("PLATFORM_AUDIT_DATABASE_URL not set")?;
    let ar_url = std::env::var("AR_DATABASE_URL").context("AR_DATABASE_URL not set")?;
    let payments_url =
        std::env::var("PAYMENTS_DATABASE_URL").context("PAYMENTS_DATABASE_URL not set")?;
    let gl_url = std::env::var("GL_DATABASE_URL").context("GL_DATABASE_URL not set")?;

    tracing::info!("Connecting to databases");
    let audit_pool = PgPool::connect(&audit_url)
        .await
        .context("Failed to connect to audit database")?;
    let ar_pool = PgPool::connect(&ar_url)
        .await
        .context("Failed to connect to AR database")?;
    let payments_pool = PgPool::connect(&payments_url)
        .await
        .context("Failed to connect to payments database")?;
    let gl_pool = PgPool::connect(&gl_url)
        .await
        .context("Failed to connect to GL database")?;

    // Fetch data
    tracing::info!("Fetching audit events");
    let audit_events = fetch_audit_events(&audit_pool, tenant_id).await?;

    tracing::info!("Fetching AR invoices");
    let ar_invoices = fetch_ar_invoices(&ar_pool, tenant_id).await?;

    tracing::info!("Fetching payment attempts");
    let payment_attempts = fetch_payment_attempts(&payments_pool, tenant_id).await?;

    tracing::info!("Fetching journal entries");
    let journal_entries = fetch_journal_entries(&gl_pool, tenant_id).await?;

    // Calculate checksums
    tracing::info!("Calculating checksums");
    let audit_checksum = calculate_checksum(&audit_events)?;
    let ar_checksum = calculate_checksum(&ar_invoices)?;
    let payments_checksum = calculate_checksum(&payment_attempts)?;
    let gl_checksum = calculate_checksum(&journal_entries)?;

    // Export data
    let extension = if format == "json" { "jsonl" } else { "csv" };

    tracing::info!("Exporting audit events");
    let audit_path = Path::new(output_dir).join(format!("audit_events.{}", extension));
    if format == "json" {
        export_jsonl(&audit_events, &audit_path)?;
    } else {
        export_csv(&audit_events, &audit_path)?;
    }

    tracing::info!("Exporting AR invoices");
    let ar_path = Path::new(output_dir).join(format!("ar_invoices.{}", extension));
    if format == "json" {
        export_jsonl(&ar_invoices, &ar_path)?;
    } else {
        export_csv(&ar_invoices, &ar_path)?;
    }

    tracing::info!("Exporting payment attempts");
    let payments_path = Path::new(output_dir).join(format!("payment_attempts.{}", extension));
    if format == "json" {
        export_jsonl(&payment_attempts, &payments_path)?;
    } else {
        export_csv(&payment_attempts, &payments_path)?;
    }

    tracing::info!("Exporting journal entries");
    let gl_path = Path::new(output_dir).join(format!("journal_entries.{}", extension));
    if format == "json" {
        export_jsonl(&journal_entries, &gl_path)?;
    } else {
        export_csv(&journal_entries, &gl_path)?;
    }

    // Create manifest
    let manifest = ExportManifest {
        tenant_id: tenant_id.to_string(),
        export_timestamp: Utc::now(),
        format: format.to_string(),
        audit_events_count: audit_events.len(),
        audit_events_checksum: audit_checksum,
        ar_invoices_count: ar_invoices.len(),
        ar_invoices_checksum: ar_checksum,
        payment_attempts_count: payment_attempts.len(),
        payment_attempts_checksum: payments_checksum,
        journal_entries_count: journal_entries.len(),
        journal_entries_checksum: gl_checksum,
    };

    tracing::info!("Exporting manifest");
    let manifest_path = Path::new(output_dir).join("manifest.json");
    export_manifest(&manifest, &manifest_path)?;

    // Close database connections
    audit_pool.close().await;
    ar_pool.close().await;
    payments_pool.close().await;
    gl_pool.close().await;

    tracing::info!(
        audit_events = audit_events.len(),
        ar_invoices = ar_invoices.len(),
        payment_attempts = payment_attempts.len(),
        journal_entries = journal_entries.len(),
        "Compliance export completed successfully"
    );

    println!("✓ Compliance export completed successfully");
    println!("  Tenant: {}", tenant_id);
    println!("  Output directory: {}", output_dir);
    println!("  Format: {}", format);
    println!();
    println!(
        "  Audit events:     {} records (checksum: {})",
        manifest.audit_events_count,
        &manifest.audit_events_checksum[..16]
    );
    println!(
        "  AR invoices:      {} records (checksum: {})",
        manifest.ar_invoices_count,
        &manifest.ar_invoices_checksum[..16]
    );
    println!(
        "  Payment attempts: {} records (checksum: {})",
        manifest.payment_attempts_count,
        &manifest.payment_attempts_checksum[..16]
    );
    println!(
        "  Journal entries:  {} records (checksum: {})",
        manifest.journal_entries_count,
        &manifest.journal_entries_checksum[..16]
    );
    println!();
    println!("  Manifest: {}/manifest.json", output_dir);

    Ok(())
}
