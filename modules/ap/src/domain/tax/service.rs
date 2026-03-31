//! AP tax service — quote, commit, void lifecycle for vendor bill tax.
//!
//! All operations are idempotent:
//! - quote: returns existing snapshot if quote_hash matches
//! - commit: no-op if already committed
//! - void: no-op if already voided
//!
//! Tax snapshots are persisted in `ap_tax_snapshots` (AP-owned table).

use sqlx::PgPool;
use uuid::Uuid;

use tax_core::{TaxCommitRequest, TaxProvider, TaxProviderError, TaxQuoteRequest, TaxVoidRequest};

use super::models::ApTaxSnapshot;

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug, thiserror::Error)]
pub enum ApTaxError {
    #[error("no tax quote found for bill {0}")]
    NoQuoteFound(Uuid),
    #[error("tax provider error: {0}")]
    Provider(#[from] TaxProviderError),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<ApTaxError> for platform_http_contracts::ApiError {
    fn from(err: ApTaxError) -> Self {
        match err {
            ApTaxError::NoQuoteFound(id) => {
                Self::not_found(format!("No tax quote found for bill {}", id))
            }
            ApTaxError::Provider(e) => {
                Self::new(422, "tax_error", e.to_string())
            }
            ApTaxError::Database(e) => {
                tracing::error!("AP tax DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

// ============================================================================
// Quote hash
// ============================================================================

/// Compute SHA-256 hash of a TaxQuoteRequest for idempotency.
///
/// Hash is derived from the taxable content (line_items, addresses, currency,
/// invoice_date) so identical requests produce identical hashes.
pub fn compute_quote_hash(req: &TaxQuoteRequest) -> String {
    use sha2::{Digest, Sha256};

    let canonical = serde_json::json!({
        "line_items": req.line_items,
        "ship_to": req.ship_to,
        "ship_from": req.ship_from,
        "currency": req.currency,
        "invoice_date": req.invoice_date.to_rfc3339(),
    });
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    hex::encode(Sha256::digest(&bytes))
}

// ============================================================================
// Snapshot lookup
// ============================================================================

/// Fetch the active (non-voided) tax snapshot for a bill, if any.
pub async fn find_active_snapshot(
    pool: &PgPool,
    bill_id: Uuid,
) -> Result<Option<ApTaxSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, ApTaxSnapshot>(
        "SELECT id, bill_id, tenant_id, provider, provider_quote_ref, provider_commit_ref, \
         quote_hash, total_tax_minor, tax_by_line, status, quoted_at, committed_at, \
         voided_at, void_reason, created_at, updated_at \
         FROM ap_tax_snapshots WHERE bill_id = $1 AND status != 'voided' LIMIT 1",
    )
    .bind(bill_id)
    .fetch_optional(pool)
    .await
}

// ============================================================================
// Quote
// ============================================================================

/// Quote tax for a vendor bill draft. Idempotent: returns existing snapshot
/// if an active snapshot with the same quote_hash already exists.
pub async fn quote_bill_tax(
    pool: &PgPool,
    provider: &(impl TaxProvider + ?Sized),
    provider_name: &str,
    tenant_id: &str,
    bill_id: Uuid,
    req: TaxQuoteRequest,
) -> Result<ApTaxSnapshot, ApTaxError> {
    let quote_hash = compute_quote_hash(&req);

    // Idempotency: existing active snapshot with same hash -> return it.
    // If hash differs (bill content changed), void the old snapshot first.
    if let Some(snap) = find_active_snapshot(pool, bill_id).await? {
        if snap.quote_hash == quote_hash {
            return Ok(snap);
        }
        // Content changed — void old snapshot before creating new one
        sqlx::query(
            "UPDATE ap_tax_snapshots \
             SET status = 'voided', voided_at = NOW(), void_reason = 'superseded by new quote', \
                 updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(snap.id)
        .execute(pool)
        .await?;
    }

    let response = provider.quote_tax(req).await?;

    let id = Uuid::new_v4();
    let tax_by_line_json =
        serde_json::to_value(&response.tax_by_line).unwrap_or_else(|_| serde_json::json!([]));

    sqlx::query(
        "INSERT INTO ap_tax_snapshots \
         (id, bill_id, tenant_id, provider, provider_quote_ref, quote_hash, \
          total_tax_minor, tax_by_line, status, quoted_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'quoted', $9)",
    )
    .bind(id)
    .bind(bill_id)
    .bind(tenant_id)
    .bind(provider_name)
    .bind(&response.provider_quote_ref)
    .bind(&quote_hash)
    .bind(response.total_tax_minor)
    .bind(&tax_by_line_json)
    .bind(response.quoted_at)
    .execute(pool)
    .await?;

    // Fetch back for the complete row with DB defaults
    let snap = sqlx::query_as::<_, ApTaxSnapshot>(
        "SELECT id, bill_id, tenant_id, provider, provider_quote_ref, provider_commit_ref, \
         quote_hash, total_tax_minor, tax_by_line, status, quoted_at, committed_at, \
         voided_at, void_reason, created_at, updated_at \
         FROM ap_tax_snapshots WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;

    Ok(snap)
}

// ============================================================================
// Commit
// ============================================================================

/// Commit a previously quoted tax for a bill. Called during bill approval.
/// Idempotent: if already committed, returns the existing snapshot.
pub async fn commit_bill_tax(
    pool: &PgPool,
    provider: &(impl TaxProvider + ?Sized),
    tenant_id: &str,
    bill_id: Uuid,
    correlation_id: &str,
) -> Result<ApTaxSnapshot, ApTaxError> {
    let snap = find_active_snapshot(pool, bill_id)
        .await?
        .ok_or(ApTaxError::NoQuoteFound(bill_id))?;

    // Idempotency: already committed -> return
    if snap.status == "committed" {
        return Ok(snap);
    }

    let commit_req = TaxCommitRequest {
        tenant_id: tenant_id.to_string(),
        invoice_id: bill_id.to_string(),
        provider_quote_ref: snap.provider_quote_ref.clone(),
        correlation_id: correlation_id.to_string(),
    };

    let commit_resp = provider.commit_tax(commit_req).await?;

    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'committed', provider_commit_ref = $1, committed_at = $2, \
             updated_at = NOW() \
         WHERE id = $3",
    )
    .bind(&commit_resp.provider_commit_ref)
    .bind(commit_resp.committed_at)
    .bind(snap.id)
    .execute(pool)
    .await?;

    let updated = sqlx::query_as::<_, ApTaxSnapshot>(
        "SELECT id, bill_id, tenant_id, provider, provider_quote_ref, provider_commit_ref, \
         quote_hash, total_tax_minor, tax_by_line, status, quoted_at, committed_at, \
         voided_at, void_reason, created_at, updated_at \
         FROM ap_tax_snapshots WHERE id = $1",
    )
    .bind(snap.id)
    .fetch_one(pool)
    .await?;

    Ok(updated)
}

// ============================================================================
// Void
// ============================================================================

/// Void tax for a bill. Called during bill void.
/// If snapshot is 'quoted' (never committed), marks voided without calling provider.
/// If snapshot is 'committed', calls provider.void_tax then marks voided.
/// Idempotent: no-op if no active snapshot exists.
pub async fn void_bill_tax(
    pool: &PgPool,
    provider: &(impl TaxProvider + ?Sized),
    tenant_id: &str,
    bill_id: Uuid,
    void_reason: &str,
    correlation_id: &str,
) -> Result<Option<ApTaxSnapshot>, ApTaxError> {
    let snap = match find_active_snapshot(pool, bill_id).await? {
        Some(s) => s,
        None => return Ok(None), // No tax snapshot -> non-taxable bill, nothing to void
    };

    // Quoted but never committed: just mark voided (no provider call needed)
    if snap.status == "quoted" {
        sqlx::query(
            "UPDATE ap_tax_snapshots \
             SET status = 'voided', voided_at = NOW(), void_reason = $1, updated_at = NOW() \
             WHERE id = $2",
        )
        .bind(void_reason)
        .bind(snap.id)
        .execute(pool)
        .await?;

        let updated = sqlx::query_as::<_, ApTaxSnapshot>(
            "SELECT id, bill_id, tenant_id, provider, provider_quote_ref, provider_commit_ref, \
             quote_hash, total_tax_minor, tax_by_line, status, quoted_at, committed_at, \
             voided_at, void_reason, created_at, updated_at \
             FROM ap_tax_snapshots WHERE id = $1",
        )
        .bind(snap.id)
        .fetch_one(pool)
        .await?;

        return Ok(Some(updated));
    }

    // Committed: void with provider
    let commit_ref = snap.provider_commit_ref.as_deref().unwrap_or("");

    let void_req = TaxVoidRequest {
        tenant_id: tenant_id.to_string(),
        invoice_id: bill_id.to_string(),
        provider_commit_ref: commit_ref.to_string(),
        void_reason: void_reason.to_string(),
        correlation_id: correlation_id.to_string(),
    };

    let void_resp = provider.void_tax(void_req).await?;

    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'voided', voided_at = $1, void_reason = $2, updated_at = NOW() \
         WHERE id = $3",
    )
    .bind(void_resp.voided_at)
    .bind(void_reason)
    .bind(snap.id)
    .execute(pool)
    .await?;

    let updated = sqlx::query_as::<_, ApTaxSnapshot>(
        "SELECT id, bill_id, tenant_id, provider, provider_quote_ref, provider_commit_ref, \
         quote_hash, total_tax_minor, tax_by_line, status, quoted_at, committed_at, \
         voided_at, void_reason, created_at, updated_at \
         FROM ap_tax_snapshots WHERE id = $1",
    )
    .bind(snap.id)
    .fetch_one(pool)
    .await?;

    Ok(Some(updated))
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::tax::ZeroTaxProvider;
    use chrono::Utc;
    use serial_test::serial;
    use tax_core::models::{TaxAddress, TaxLineItem, TaxQuoteRequest};

    const TEST_TENANT: &str = "test-tenant-ap-tax";

    fn db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    async fn pool() -> PgPool {
        PgPool::connect(&db_url()).await.expect("DB connect failed")
    }

    fn sample_address() -> TaxAddress {
        TaxAddress {
            line1: "123 Main St".to_string(),
            line2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94102".to_string(),
            country: "US".to_string(),
        }
    }

    /// Fixed invoice date for deterministic quote hashing in tests.
    fn fixed_invoice_date() -> chrono::DateTime<Utc> {
        "2026-01-15T12:00:00Z".parse().expect("valid datetime")
    }

    fn sample_quote_req(bill_id: Uuid) -> TaxQuoteRequest {
        TaxQuoteRequest {
            tenant_id: TEST_TENANT.to_string(),
            invoice_id: bill_id.to_string(),
            customer_id: "vendor-1".to_string(),
            ship_to: sample_address(),
            ship_from: sample_address(),
            line_items: vec![TaxLineItem {
                line_id: "line-1".to_string(),
                description: "Widget".to_string(),
                amount_minor: 50000,
                currency: "USD".to_string(),
                tax_code: None,
                quantity: 10.0,
            }],
            currency: "USD".to_string(),
            invoice_date: fixed_invoice_date(),
            correlation_id: "corr-tax-test".to_string(),
        }
    }

    async fn create_vendor(db: &PgPool) -> Uuid {
        let vendor_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days, \
             is_active, created_at, updated_at) VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
        )
        .bind(vendor_id)
        .bind(TEST_TENANT)
        .bind(format!("Vendor-{}", vendor_id))
        .execute(db)
        .await
        .expect("insert vendor");
        vendor_id
    }

    async fn create_bill(db: &PgPool, vendor_id: Uuid) -> Uuid {
        let bill_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendor_bills (bill_id, tenant_id, vendor_id, vendor_invoice_ref, \
             currency, total_minor, invoice_date, due_date, status, entered_by, entered_at) \
             VALUES ($1, $2, $3, $4, 'USD', 50000, NOW(), NOW() + interval '30 days', \
             'open', 'system', NOW())",
        )
        .bind(bill_id)
        .bind(TEST_TENANT)
        .bind(vendor_id)
        .bind(format!("INV-{}", &bill_id.to_string()[..8]))
        .execute(db)
        .await
        .expect("insert bill");
        bill_id
    }

    async fn cleanup(db: &PgPool) {
        for q in [
            "DELETE FROM ap_tax_snapshots WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id IN (SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM bill_lines WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM vendor_bills WHERE tenant_id = $1",
            "DELETE FROM vendors WHERE tenant_id = $1",
        ] {
            sqlx::query(q).bind(TEST_TENANT).execute(db).await.ok();
        }
    }

    #[test]
    fn quote_hash_is_deterministic() {
        let bill_id = Uuid::new_v4();
        let req = sample_quote_req(bill_id);
        let h1 = compute_quote_hash(&req);
        let h2 = compute_quote_hash(&req);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn quote_hash_changes_with_amount() {
        let bill_id = Uuid::new_v4();
        let mut req = sample_quote_req(bill_id);
        let h1 = compute_quote_hash(&req);
        req.line_items[0].amount_minor = 99999;
        let h2 = compute_quote_hash(&req);
        assert_ne!(h1, h2);
    }

    #[tokio::test]
    #[serial]
    async fn test_quote_bill_tax_persists_snapshot() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id).await;

        let provider = ZeroTaxProvider;
        let req = sample_quote_req(bill_id);
        let snap = quote_bill_tax(&db, &provider, "zero", TEST_TENANT, bill_id, req)
            .await
            .expect("quote_bill_tax failed");

        assert_eq!(snap.bill_id, bill_id);
        assert_eq!(snap.status, "quoted");
        assert_eq!(snap.total_tax_minor, 0); // ZeroTaxProvider
        assert!(snap.provider_quote_ref.starts_with("zero-quote-"));
        assert!(snap.provider_commit_ref.is_none());

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_quote_idempotent_same_hash() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id).await;

        let provider = ZeroTaxProvider;
        let req1 = sample_quote_req(bill_id);
        let req2 = sample_quote_req(bill_id);
        let snap1 = quote_bill_tax(&db, &provider, "zero", TEST_TENANT, bill_id, req1)
            .await
            .expect("first quote");
        let snap2 = quote_bill_tax(&db, &provider, "zero", TEST_TENANT, bill_id, req2)
            .await
            .expect("second quote");

        assert_eq!(
            snap1.id, snap2.id,
            "idempotent quote must return same snapshot"
        );

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_commit_bill_tax_updates_status() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id).await;

        let provider = ZeroTaxProvider;
        let req = sample_quote_req(bill_id);
        quote_bill_tax(&db, &provider, "zero", TEST_TENANT, bill_id, req)
            .await
            .expect("quote failed");

        let committed = commit_bill_tax(&db, &provider, TEST_TENANT, bill_id, "corr-commit")
            .await
            .expect("commit failed");

        assert_eq!(committed.status, "committed");
        assert!(committed.provider_commit_ref.is_some());
        assert!(committed.committed_at.is_some());

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_commit_idempotent() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id).await;

        let provider = ZeroTaxProvider;
        let req = sample_quote_req(bill_id);
        quote_bill_tax(&db, &provider, "zero", TEST_TENANT, bill_id, req)
            .await
            .expect("quote failed");

        let first = commit_bill_tax(&db, &provider, TEST_TENANT, bill_id, "corr-c1")
            .await
            .expect("first commit");
        let second = commit_bill_tax(&db, &provider, TEST_TENANT, bill_id, "corr-c2")
            .await
            .expect("second commit must succeed (idempotent)");

        assert_eq!(first.id, second.id);
        assert_eq!(second.status, "committed");

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_commit_without_quote_fails() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id).await;

        let provider = ZeroTaxProvider;
        let result = commit_bill_tax(&db, &provider, TEST_TENANT, bill_id, "corr-no-quote").await;

        assert!(
            matches!(result, Err(ApTaxError::NoQuoteFound(_))),
            "expected NoQuoteFound, got {:?}",
            result
        );

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_void_committed_tax() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id).await;

        let provider = ZeroTaxProvider;
        let req = sample_quote_req(bill_id);
        quote_bill_tax(&db, &provider, "zero", TEST_TENANT, bill_id, req)
            .await
            .expect("quote failed");
        commit_bill_tax(&db, &provider, TEST_TENANT, bill_id, "corr-v1")
            .await
            .expect("commit failed");

        let voided = void_bill_tax(
            &db,
            &provider,
            TEST_TENANT,
            bill_id,
            "bill voided",
            "corr-v2",
        )
        .await
        .expect("void failed");

        let snap = voided.expect("should have voided snapshot");
        assert_eq!(snap.status, "voided");
        assert!(snap.voided_at.is_some());
        assert_eq!(snap.void_reason.as_deref(), Some("bill voided"));

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_void_quoted_tax_skips_provider() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id).await;

        let provider = ZeroTaxProvider;
        let req = sample_quote_req(bill_id);
        quote_bill_tax(&db, &provider, "zero", TEST_TENANT, bill_id, req)
            .await
            .expect("quote failed");

        // Void without committing first — should mark voided without provider call
        let voided = void_bill_tax(
            &db,
            &provider,
            TEST_TENANT,
            bill_id,
            "draft cancelled",
            "corr-v3",
        )
        .await
        .expect("void failed");

        let snap = voided.expect("should have voided snapshot");
        assert_eq!(snap.status, "voided");

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_void_no_snapshot_returns_none() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id).await;

        let provider = ZeroTaxProvider;
        let result = void_bill_tax(&db, &provider, TEST_TENANT, bill_id, "no tax", "corr-v4")
            .await
            .expect("void should not fail");

        assert!(result.is_none(), "no snapshot -> None");

        cleanup(&db).await;
    }
}
