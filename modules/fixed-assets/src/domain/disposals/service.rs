//! Disposal service: Guard → Mutation → Outbox atomicity.
//!
//! Disposes or impairs an asset, recording the financial impact.
//! Idempotent: if the asset is already disposed/impaired, returns existing disposal.

use sqlx::PgPool;
use uuid::Uuid;

use super::*;
use crate::outbox;

/// Minimal projection for disposal — avoids decoding custom PG types.
#[derive(sqlx::FromRow)]
struct AssetForDisposal {
    #[allow(dead_code)]
    id: Uuid,
    category_id: Uuid,
    status: String,
    acquisition_cost_minor: i64,
    accum_depreciation_minor: i64,
    net_book_value_minor: i64,
    currency: String,
    asset_account_ref: Option<String>,
    accum_depreciation_ref: Option<String>,
}

/// Category account refs needed for GL posting.
#[derive(sqlx::FromRow)]
struct CategoryAccounts {
    asset_account_ref: String,
    accum_depreciation_ref: String,
    gain_loss_account_ref: Option<String>,
}

/// Select columns for Disposal, casting disposal_type to TEXT.
const DISPOSAL_COLUMNS: &str = r#"
    id, tenant_id, asset_id, disposal_type::TEXT as disposal_type,
    disposal_date, net_book_value_at_disposal_minor,
    proceeds_minor, gain_loss_minor, currency,
    reason, buyer, reference, journal_entry_ref,
    is_posted, posted_at, created_by, approved_by,
    created_at, updated_at
"#;

pub struct DisposalService;

impl DisposalService {
    /// Dispose or impair an asset.
    ///
    /// Guard: validates input, checks asset exists and is in a disposable state.
    /// Mutation: inserts disposal record, updates asset status and NBV.
    /// Outbox: emits asset_disposed event with GL entry data.
    ///
    /// Idempotent: if the asset is already disposed/impaired, returns the
    /// existing disposal record without creating a duplicate.
    pub async fn dispose(
        pool: &PgPool,
        req: &DisposeAssetRequest,
    ) -> Result<Disposal, DisposalError> {
        req.validate()?;

        let mut tx = pool.begin().await?;

        // Guard: fetch asset with row lock
        let asset = sqlx::query_as::<_, AssetForDisposal>(
            r#"
            SELECT id, category_id, status,
                   acquisition_cost_minor, accum_depreciation_minor,
                   net_book_value_minor, currency,
                   asset_account_ref, accum_depreciation_ref
            FROM fa_assets
            WHERE id = $1 AND tenant_id = $2
            FOR UPDATE
            "#,
        )
        .bind(req.asset_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DisposalError::AssetNotFound(req.asset_id))?;

        // Idempotent: if already disposed/impaired, return existing disposal
        if asset.status == "disposed" || asset.status == "impaired" {
            let existing = sqlx::query_as::<_, Disposal>(&format!(
                "SELECT {} FROM fa_disposals WHERE asset_id = $1 AND tenant_id = $2 \
                 ORDER BY created_at DESC LIMIT 1",
                DISPOSAL_COLUMNS
            ))
            .bind(req.asset_id)
            .bind(&req.tenant_id)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(d) = existing {
                tx.commit().await?;
                return Ok(d);
            }
            return Err(DisposalError::InvalidState(format!(
                "Asset {} is {} but no disposal record found",
                req.asset_id, asset.status
            )));
        }

        // Guard: only active, fully_depreciated, or draft can be disposed
        if asset.status != "active"
            && asset.status != "fully_depreciated"
            && asset.status != "draft"
        {
            return Err(DisposalError::InvalidState(format!(
                "Cannot dispose asset in '{}' status",
                asset.status
            )));
        }

        // Fetch category for GL account refs
        let cat = sqlx::query_as::<_, CategoryAccounts>(
            "SELECT asset_account_ref, accum_depreciation_ref, gain_loss_account_ref \
             FROM fa_categories WHERE id = $1 AND tenant_id = $2",
        )
        .bind(asset.category_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DisposalError::CategoryNotFound(asset.category_id))?;

        // Compute financials
        let proceeds = req.proceeds_minor.unwrap_or(0);
        let nbv = asset.net_book_value_minor;
        let gain_loss = proceeds - nbv;

        let disposal_id = Uuid::new_v4();
        let target_status = req.disposal_type.target_status();

        // Mutation: insert disposal
        let disposal = sqlx::query_as::<_, Disposal>(&format!(
            r#"
            INSERT INTO fa_disposals
                (id, tenant_id, asset_id, disposal_type, disposal_date,
                 net_book_value_at_disposal_minor, proceeds_minor, gain_loss_minor,
                 currency, reason, buyer, reference, created_by,
                 created_at, updated_at)
            VALUES ($1, $2, $3, $4::fa_disposal_type, $5, $6, $7, $8, $9,
                    $10, $11, $12, $13, NOW(), NOW())
            RETURNING {}
            "#,
            DISPOSAL_COLUMNS
        ))
        .bind(disposal_id)
        .bind(&req.tenant_id)
        .bind(req.asset_id)
        .bind(req.disposal_type.as_str())
        .bind(req.disposal_date)
        .bind(nbv)
        .bind(proceeds)
        .bind(gain_loss)
        .bind(&asset.currency)
        .bind(req.reason.as_deref())
        .bind(req.buyer.as_deref())
        .bind(req.reference.as_deref())
        .bind(req.created_by.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        // Mutation: update asset status and zero NBV
        sqlx::query(
            "UPDATE fa_assets SET status = $3, net_book_value_minor = 0, updated_at = NOW() \
             WHERE id = $1 AND tenant_id = $2",
        )
        .bind(req.asset_id)
        .bind(&req.tenant_id)
        .bind(target_status)
        .execute(&mut *tx)
        .await?;

        // Resolve account refs (asset override > category default)
        let asset_acct = asset
            .asset_account_ref
            .unwrap_or_else(|| cat.asset_account_ref.clone());
        let accum_acct = asset
            .accum_depreciation_ref
            .unwrap_or_else(|| cat.accum_depreciation_ref.clone());

        // Outbox event
        let gl_data = DisposalGlData {
            disposal_id,
            asset_id: req.asset_id,
            disposal_type: req.disposal_type.as_str().to_string(),
            disposal_date: req.disposal_date,
            acquisition_cost_minor: asset.acquisition_cost_minor,
            accum_depreciation_minor: asset.accum_depreciation_minor,
            net_book_value_minor: nbv,
            proceeds_minor: proceeds,
            gain_loss_minor: gain_loss,
            currency: asset.currency.clone(),
            asset_account_ref: asset_acct,
            accum_depreciation_ref: accum_acct,
            gain_loss_account_ref: cat.gain_loss_account_ref,
        };
        let event = AssetDisposedEvent {
            disposal_id,
            asset_id: req.asset_id,
            tenant_id: req.tenant_id.clone(),
            disposal_type: req.disposal_type.as_str().to_string(),
            disposal_date: req.disposal_date,
            gl_data,
        };
        outbox::enqueue_event_tx(
            &mut tx,
            &req.tenant_id,
            Uuid::new_v4(),
            "asset_disposed",
            "fa_disposal",
            &disposal_id.to_string(),
            &event,
        )
        .await?;

        tx.commit().await?;
        Ok(disposal)
    }

    /// List all disposals for a tenant.
    pub async fn list(pool: &PgPool, tenant_id: &str) -> Result<Vec<Disposal>, DisposalError> {
        let disposals = sqlx::query_as::<_, Disposal>(&format!(
            "SELECT {} FROM fa_disposals WHERE tenant_id = $1 ORDER BY disposal_date DESC",
            DISPOSAL_COLUMNS
        ))
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;
        Ok(disposals)
    }

    /// Fetch a single disposal by id.
    pub async fn get(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<Disposal>, DisposalError> {
        let disposal = sqlx::query_as::<_, Disposal>(&format!(
            "SELECT {} FROM fa_disposals WHERE id = $1 AND tenant_id = $2",
            DISPOSAL_COLUMNS
        ))
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;
        Ok(disposal)
    }
}

// ============================================================================
// Integrated tests — require running fixed-assets Postgres instance
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serial_test::serial;

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db?sslmode=disable"
                .to_string()
        })
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("connect to fixed-assets test DB")
    }

    const TEST_TENANT: &str = "test-disposal-svc";

    async fn cleanup(pool: &PgPool) {
        for q in [
            "DELETE FROM fa_disposals WHERE tenant_id = $1",
            "DELETE FROM fa_depreciation_schedules WHERE tenant_id = $1",
            "DELETE FROM fa_depreciation_runs WHERE tenant_id = $1",
            "DELETE FROM fa_events_outbox WHERE tenant_id = $1",
            "DELETE FROM fa_assets WHERE tenant_id = $1",
            "DELETE FROM fa_categories WHERE tenant_id = $1",
        ] {
            sqlx::query(q).bind(TEST_TENANT).execute(pool).await.ok();
        }
    }

    /// Insert category + active asset (cost=120000, accum=60000, NBV=60000).
    async fn setup_active_asset(pool: &PgPool) -> (Uuid, Uuid) {
        let cat_id = Uuid::new_v4();
        let tag = format!("DSP-{}", &cat_id.to_string()[..8]);
        sqlx::query(
            r#"
            INSERT INTO fa_categories
                (id, tenant_id, code, name,
                 default_method, default_useful_life_months, default_salvage_pct_bp,
                 asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
                 gain_loss_account_ref, is_active, created_at, updated_at)
            VALUES ($1,$2,$3,$4,'straight_line',12,0,'1500','6100','1510',
                    '7000',TRUE,NOW(),NOW())
            "#,
        )
        .bind(cat_id)
        .bind(TEST_TENANT)
        .bind(tag.clone())
        .bind(format!("Category {}", tag))
        .execute(pool)
        .await
        .expect("insert test category");

        let asset_id = Uuid::new_v4();
        let atag = format!("FA-{}", &asset_id.to_string()[..8]);
        sqlx::query(
            r#"
            INSERT INTO fa_assets
                (id, tenant_id, category_id, asset_tag, name,
                 status, acquisition_date, in_service_date,
                 acquisition_cost_minor, currency,
                 depreciation_method, useful_life_months, salvage_value_minor,
                 accum_depreciation_minor, net_book_value_minor,
                 created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,'active','2026-01-01','2026-01-01',
                    120000,'usd','straight_line',12,0,60000,60000,NOW(),NOW())
            "#,
        )
        .bind(asset_id)
        .bind(TEST_TENANT)
        .bind(cat_id)
        .bind(atag)
        .bind("Test Disposal Asset")
        .execute(pool)
        .await
        .expect("insert test asset");

        (cat_id, asset_id)
    }

    #[tokio::test]
    #[serial]
    async fn dispose_sale_computes_gain() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (_, asset_id) = setup_active_asset(&pool).await;

        let req = DisposeAssetRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
            disposal_type: DisposalType::Sale,
            disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid test date"),
            proceeds_minor: Some(80000),
            reason: Some("Upgrading".into()),
            buyer: Some("Acme Corp".into()),
            reference: None,
            created_by: None,
        };

        let d = DisposalService::dispose(&pool, &req).await.expect("dispose failed");
        assert_eq!(d.disposal_type, "sale");
        assert_eq!(d.net_book_value_at_disposal_minor, 60000);
        assert_eq!(d.proceeds_minor, 80000);
        assert_eq!(d.gain_loss_minor, 20000);

        let (status,): (String,) = sqlx::query_as("SELECT status FROM fa_assets WHERE id = $1")
            .bind(asset_id)
            .fetch_one(&pool)
            .await
            .expect("status query failed");
        assert_eq!(status, "disposed");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn dispose_scrap_computes_loss() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (_, asset_id) = setup_active_asset(&pool).await;

        let req = DisposeAssetRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
            disposal_type: DisposalType::Scrap,
            disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid test date"),
            proceeds_minor: None,
            reason: Some("Broken".into()),
            buyer: None,
            reference: None,
            created_by: None,
        };

        let d = DisposalService::dispose(&pool, &req).await.expect("dispose failed");
        assert_eq!(d.disposal_type, "scrap");
        assert_eq!(d.gain_loss_minor, -60000);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn dispose_is_idempotent() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (_, asset_id) = setup_active_asset(&pool).await;

        let req = DisposeAssetRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
            disposal_type: DisposalType::Sale,
            disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid test date"),
            proceeds_minor: Some(50000),
            reason: None,
            buyer: None,
            reference: None,
            created_by: None,
        };

        let d1 = DisposalService::dispose(&pool, &req).await.expect("dispose d1 failed");
        let d2 = DisposalService::dispose(&pool, &req).await.expect("dispose d2 failed");
        assert_eq!(d1.id, d2.id, "idempotent — same disposal returned");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn impairment_sets_impaired_status() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (_, asset_id) = setup_active_asset(&pool).await;

        let req = DisposeAssetRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
            disposal_type: DisposalType::Impairment,
            disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid test date"),
            proceeds_minor: None,
            reason: Some("Market value decline".into()),
            buyer: None,
            reference: None,
            created_by: None,
        };

        let d = DisposalService::dispose(&pool, &req).await.expect("dispose failed");
        assert_eq!(d.disposal_type, "impairment");
        assert_eq!(d.gain_loss_minor, -60000);

        let (status,): (String,) = sqlx::query_as("SELECT status FROM fa_assets WHERE id = $1")
            .bind(asset_id)
            .fetch_one(&pool)
            .await
            .expect("status query failed");
        assert_eq!(status, "impaired");

        cleanup(&pool).await;
    }
}
