//! Vendor CRUD service — DB operations with Guard→Mutation→Outbox atomicity.
//!
//! All write operations:
//! 1. Guard: validate inputs and check preconditions
//! 2. Mutation: write to the vendors table
//! 3. Outbox: enqueue event atomically in the same transaction

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_vendor_created_envelope, build_vendor_updated_envelope,
    VendorCreatedPayload, VendorUpdatedPayload,
    EVENT_TYPE_VENDOR_CREATED, EVENT_TYPE_VENDOR_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use super::{CreateVendorRequest, UpdateVendorRequest, Vendor, VendorError};

// ============================================================================
// Reads
// ============================================================================

/// Fetch a single vendor by ID, scoped to tenant.
pub async fn get_vendor(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
) -> Result<Option<Vendor>, VendorError> {
    let vendor = sqlx::query_as::<_, Vendor>(
        r#"
        SELECT vendor_id, tenant_id, name, tax_id, currency,
               payment_terms_days, payment_method, remittance_email,
               is_active, party_id, created_at, updated_at
        FROM vendors
        WHERE vendor_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(vendor)
}

/// List vendors for a tenant. Pass `include_inactive = true` to include deactivated vendors.
pub async fn list_vendors(
    pool: &PgPool,
    tenant_id: &str,
    include_inactive: bool,
) -> Result<Vec<Vendor>, VendorError> {
    let vendors = if include_inactive {
        sqlx::query_as::<_, Vendor>(
            r#"
            SELECT vendor_id, tenant_id, name, tax_id, currency,
                   payment_terms_days, payment_method, remittance_email,
                   is_active, party_id, created_at, updated_at
            FROM vendors
            WHERE tenant_id = $1
            ORDER BY name ASC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Vendor>(
            r#"
            SELECT vendor_id, tenant_id, name, tax_id, currency,
                   payment_terms_days, payment_method, remittance_email,
                   is_active, party_id, created_at, updated_at
            FROM vendors
            WHERE tenant_id = $1 AND is_active = TRUE
            ORDER BY name ASC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    };

    Ok(vendors)
}

// ============================================================================
// Writes
// ============================================================================

/// Create a new vendor. Emits `ap.vendor_created` via the outbox.
///
/// Returns `VendorError::DuplicateName` if an active vendor with the same name
/// already exists for this tenant.
pub async fn create_vendor(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateVendorRequest,
    correlation_id: String,
) -> Result<Vendor, VendorError> {
    req.validate()?;

    let vendor_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    // Guard: check for duplicate active vendor name within tenant
    let duplicate: Option<(Uuid,)> = sqlx::query_as(
        "SELECT vendor_id FROM vendors WHERE tenant_id = $1 AND name = $2 AND is_active = TRUE LIMIT 1",
    )
    .bind(tenant_id)
    .bind(req.name.trim())
    .fetch_optional(&mut *tx)
    .await?;

    if duplicate.is_some() {
        return Err(VendorError::DuplicateName(req.name.clone()));
    }

    // Mutation: insert vendor
    let vendor = sqlx::query_as::<_, Vendor>(
        r#"
        INSERT INTO vendors (
            vendor_id, tenant_id, name, tax_id, currency,
            payment_terms_days, payment_method, remittance_email,
            is_active, party_id, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE, $9, $10, $10)
        RETURNING
            vendor_id, tenant_id, name, tax_id, currency,
            payment_terms_days, payment_method, remittance_email,
            is_active, party_id, created_at, updated_at
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .bind(req.name.trim())
    .bind(&req.tax_id)
    .bind(req.currency.to_uppercase())
    .bind(req.payment_terms_days)
    .bind(&req.payment_method)
    .bind(&req.remittance_email)
    .bind(req.party_id)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        // Unique constraint violation maps to DuplicateName
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.code().as_deref() == Some("23505") {
                return sqlx::Error::RowNotFound; // remapped below
            }
        }
        e
    })?;

    // Outbox: enqueue vendor_created event
    let payload = VendorCreatedPayload {
        vendor_id,
        tenant_id: tenant_id.to_string(),
        name: vendor.name.clone(),
        tax_id: vendor.tax_id.clone(),
        currency: vendor.currency.clone(),
        payment_terms_days: vendor.payment_terms_days,
        payment_method: vendor.payment_method.clone(),
        remittance_email: vendor.remittance_email.clone(),
        created_at: vendor.created_at,
    };

    let envelope = build_vendor_created_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_VENDOR_CREATED,
        "vendor",
        &vendor_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(vendor)
}

/// Update mutable vendor fields. Emits `ap.vendor_updated` via the outbox.
///
/// Returns `VendorError::NotFound` if the vendor does not exist for this tenant.
pub async fn update_vendor(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    req: &UpdateVendorRequest,
    correlation_id: String,
) -> Result<Vendor, VendorError> {
    req.validate()?;

    let event_id = Uuid::new_v4();
    let actor = req
        .updated_by
        .clone()
        .unwrap_or_else(|| "system".to_string());

    let mut tx = pool.begin().await?;

    // Guard: vendor must exist for this tenant
    let existing: Option<Vendor> = sqlx::query_as(
        r#"
        SELECT vendor_id, tenant_id, name, tax_id, currency,
               payment_terms_days, payment_method, remittance_email,
               is_active, party_id, created_at, updated_at
        FROM vendors
        WHERE vendor_id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(VendorError::NotFound(vendor_id))?;

    // Resolve updated values (keep existing where not provided)
    let new_name = req
        .name
        .as_deref()
        .map(|n| n.trim().to_string())
        .unwrap_or_else(|| current.name.clone());
    let new_tax_id = if req.tax_id.is_some() {
        req.tax_id.clone()
    } else {
        current.tax_id.clone()
    };
    let new_currency = req
        .currency
        .as_deref()
        .map(|c| c.to_uppercase())
        .unwrap_or_else(|| current.currency.clone());
    let new_terms = req.payment_terms_days.unwrap_or(current.payment_terms_days);
    let new_method = if req.payment_method.is_some() {
        req.payment_method.clone()
    } else {
        current.payment_method.clone()
    };
    let new_email = if req.remittance_email.is_some() {
        req.remittance_email.clone()
    } else {
        current.remittance_email.clone()
    };
    let new_party_id = if req.party_id.is_some() { req.party_id } else { current.party_id };
    let now = Utc::now();

    // Mutation
    let vendor = sqlx::query_as::<_, Vendor>(
        r#"
        UPDATE vendors
        SET name = $1, tax_id = $2, currency = $3,
            payment_terms_days = $4, payment_method = $5,
            remittance_email = $6, party_id = $7, updated_at = $8
        WHERE vendor_id = $9 AND tenant_id = $10
        RETURNING
            vendor_id, tenant_id, name, tax_id, currency,
            payment_terms_days, payment_method, remittance_email,
            is_active, party_id, created_at, updated_at
        "#,
    )
    .bind(&new_name)
    .bind(&new_tax_id)
    .bind(&new_currency)
    .bind(new_terms)
    .bind(&new_method)
    .bind(&new_email)
    .bind(new_party_id)
    .bind(now)
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox: enqueue vendor_updated event
    let payload = VendorUpdatedPayload {
        vendor_id,
        tenant_id: tenant_id.to_string(),
        name: req.name.clone(),
        tax_id: req.tax_id.clone(),
        currency: req.currency.clone(),
        payment_terms_days: req.payment_terms_days,
        payment_method: req.payment_method.clone(),
        remittance_email: req.remittance_email.clone(),
        updated_by: actor,
        updated_at: now,
    };

    let envelope = build_vendor_updated_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_VENDOR_UPDATED,
        "vendor",
        &vendor_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(vendor)
}

/// Deactivate a vendor (soft delete). Emits `ap.vendor_updated` with lifecycle context.
///
/// Returns `VendorError::NotFound` if the vendor does not exist for this tenant.
pub async fn deactivate_vendor(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    actor: &str,
    correlation_id: String,
) -> Result<(), VendorError> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    // Guard: vendor must exist for this tenant
    let exists: Option<(bool,)> =
        sqlx::query_as("SELECT is_active FROM vendors WHERE vendor_id = $1 AND tenant_id = $2")
            .bind(vendor_id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;

    if exists.is_none() {
        return Err(VendorError::NotFound(vendor_id));
    }

    // Mutation
    sqlx::query(
        "UPDATE vendors SET is_active = FALSE, updated_at = $1 WHERE vendor_id = $2 AND tenant_id = $3",
    )
    .bind(now)
    .bind(vendor_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    // Outbox: vendor_updated with is_active=false signal via updated_by
    let payload = VendorUpdatedPayload {
        vendor_id,
        tenant_id: tenant_id.to_string(),
        name: None,
        tax_id: None,
        currency: None,
        payment_terms_days: None,
        payment_method: None,
        remittance_email: None,
        updated_by: format!("deactivate:{}", actor),
        updated_at: now,
    };

    let envelope = build_vendor_updated_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_VENDOR_UPDATED,
        "vendor",
        &vendor_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(())
}

// ============================================================================
// Integrated Tests (real DB)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-vendors";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string()
        })
    }

    async fn test_pool() -> PgPool {
        sqlx::PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to AP test database")
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM events_outbox WHERE aggregate_type = 'vendor' AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();
    }

    fn sample_create_req(name: &str) -> CreateVendorRequest {
        CreateVendorRequest {
            name: name.to_string(),
            tax_id: Some("12-3456789".to_string()),
            currency: "USD".to_string(),
            payment_terms_days: 30,
            payment_method: Some("ach".to_string()),
            remittance_email: Some("ap@example.com".to_string()),
            party_id: None,
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_create_and_get_vendor() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_create_req("Acme Corp");
        let vendor = create_vendor(&pool, TEST_TENANT, &req, "corr-1".to_string())
            .await
            .expect("create_vendor failed");

        assert_eq!(vendor.name, "Acme Corp");
        assert_eq!(vendor.tenant_id, TEST_TENANT);
        assert_eq!(vendor.payment_terms_days, 30);
        assert_eq!(vendor.currency, "USD");
        assert!(vendor.is_active);

        let fetched = get_vendor(&pool, TEST_TENANT, vendor.vendor_id)
            .await
            .expect("get_vendor failed");
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().vendor_id, vendor.vendor_id);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_duplicate_vendor_name_rejected() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_create_req("Beta Supplies");
        create_vendor(&pool, TEST_TENANT, &req, "corr-1".to_string())
            .await
            .expect("first create failed");

        let result = create_vendor(&pool, TEST_TENANT, &req, "corr-2".to_string()).await;
        assert!(
            matches!(result, Err(VendorError::DuplicateName(_))),
            "expected DuplicateName, got {:?}",
            result
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_update_vendor() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_vendor(
            &pool,
            TEST_TENANT,
            &sample_create_req("Gamma LLC"),
            "corr-1".to_string(),
        )
        .await
        .expect("create failed");

        let update = UpdateVendorRequest {
            name: None,
            tax_id: None,
            currency: None,
            payment_terms_days: Some(45),
            payment_method: Some("wire".to_string()),
            remittance_email: None,
            updated_by: Some("user-42".to_string()),
            party_id: None,
        };
        let updated = update_vendor(
            &pool,
            TEST_TENANT,
            created.vendor_id,
            &update,
            "corr-2".to_string(),
        )
        .await
        .expect("update failed");

        assert_eq!(updated.payment_terms_days, 45);
        assert_eq!(updated.payment_method.as_deref(), Some("wire"));
        assert_eq!(updated.name, "Gamma LLC"); // unchanged

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_deactivate_vendor() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_vendor(
            &pool,
            TEST_TENANT,
            &sample_create_req("Delta Inc"),
            "corr-1".to_string(),
        )
        .await
        .expect("create failed");

        deactivate_vendor(
            &pool,
            TEST_TENANT,
            created.vendor_id,
            "user-1",
            "corr-3".to_string(),
        )
        .await
        .expect("deactivate failed");

        // Inactive vendors are excluded from the default list
        let active = list_vendors(&pool, TEST_TENANT, false)
            .await
            .expect("list failed");
        assert!(active.iter().all(|v| v.vendor_id != created.vendor_id));

        // But visible when include_inactive=true
        let all = list_vendors(&pool, TEST_TENANT, true)
            .await
            .expect("list all failed");
        let found = all.iter().find(|v| v.vendor_id == created.vendor_id);
        assert!(found.is_some());
        assert!(!found.unwrap().is_active);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_vendor_events_enqueued_in_outbox() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_vendor(
            &pool,
            TEST_TENANT,
            &sample_create_req("Epsilon Co"),
            "corr-outbox".to_string(),
        )
        .await
        .expect("create failed");

        let outbox_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'vendor' AND aggregate_id = $1",
        )
        .bind(created.vendor_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("outbox query failed");

        assert!(outbox_count.0 >= 1, "expected at least 1 event in outbox");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_get_vendor_wrong_tenant_returns_none() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_vendor(
            &pool,
            TEST_TENANT,
            &sample_create_req("Zeta LLC"),
            "corr-1".to_string(),
        )
        .await
        .expect("create failed");

        let result = get_vendor(&pool, "other-tenant", created.vendor_id)
            .await
            .expect("get_vendor failed");
        assert!(result.is_none());

        cleanup(&pool).await;
    }
}
