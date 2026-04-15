//! Disposal service: Guard → Mutation → Outbox atomicity.
//!
//! Disposes or impairs an asset, recording the financial impact.
//! Idempotent: if the asset is already disposed/impaired, returns existing disposal.

use sqlx::PgPool;
use uuid::Uuid;

use super::repo;
use super::*;
use crate::outbox;

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
        let asset = repo::fetch_asset_for_disposal(&mut *tx, req.asset_id, &req.tenant_id)
            .await?
            .ok_or(DisposalError::AssetNotFound(req.asset_id))?;

        // Idempotent: if already disposed/impaired, return existing disposal
        if asset.status == "disposed" || asset.status == "impaired" {
            let existing =
                repo::fetch_existing_disposal(&mut *tx, req.asset_id, &req.tenant_id).await?;

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
        let cat = repo::fetch_category_accounts(&mut *tx, asset.category_id, &req.tenant_id)
            .await?
            .ok_or(DisposalError::CategoryNotFound(asset.category_id))?;

        // Compute financials
        let proceeds = req.proceeds_minor.unwrap_or(0);
        let nbv = asset.net_book_value_minor;
        let gain_loss = proceeds - nbv;

        let disposal_id = Uuid::new_v4();
        let target_status = req.disposal_type.target_status();

        // Mutation: insert disposal
        let disposal = repo::insert_disposal(
            &mut *tx,
            disposal_id,
            &req.tenant_id,
            req.asset_id,
            req.disposal_type.as_str(),
            req.disposal_date,
            nbv,
            proceeds,
            gain_loss,
            &asset.currency,
            req.reason.as_deref(),
            req.buyer.as_deref(),
            req.reference.as_deref(),
            req.created_by.as_deref(),
        )
        .await?;

        // Mutation: update asset status and zero NBV
        repo::update_asset_status_disposed(&mut *tx, req.asset_id, &req.tenant_id, target_status)
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
        let disposals = repo::list_disposals(pool, tenant_id).await?;
        Ok(disposals)
    }

    /// Fetch a single disposal by id.
    pub async fn get(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<Disposal>, DisposalError> {
        let disposal = repo::get_disposal(pool, id, tenant_id).await?;
        Ok(disposal)
    }
}

// Tests in service_tests.rs
