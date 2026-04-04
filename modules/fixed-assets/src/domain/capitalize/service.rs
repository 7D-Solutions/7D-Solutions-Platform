//! Capitalization service: create fixed assets from AP bill lines.
//!
//! Triggered by `ap.vendor_bill_approved` events. For each bill line whose
//! `gl_account_code` maps to an active `fa_category.asset_account_ref`, an
//! asset is created and a linkage row written to `fa_ap_capitalizations`.
//!
//! ## Idempotency
//! `fa_ap_capitalizations` has a `UNIQUE (tenant_id, bill_id, line_id)` constraint.
//! If a row already exists for `(bill_id, line_id)`, processing is skipped and
//! `Ok(None)` is returned — safe under event replay.
//!
//! ## No AP DB writes
//! This service reads only the fixed-assets DB (fa_categories, fa_assets,
//! fa_ap_capitalizations). It never touches the AP database.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::assets::models::DepreciationMethod;
use crate::outbox;

use super::repo;

/// Outcome of a single capitalization attempt.
#[derive(Debug)]
pub struct CapitalizationResult {
    pub asset_id: Uuid,
    pub capitalization_id: i64,
}

/// Input for one bill line capitalization.
#[derive(Debug)]
pub struct CapitalizeFromApLineRequest {
    pub tenant_id: String,
    pub bill_id: Uuid,
    pub line_id: Uuid,
    pub gl_account_code: String,
    pub amount_minor: i64,
    pub currency: String,
    /// Date the bill was approved (used as acquisition_date).
    pub acquisition_date: NaiveDate,
    /// Human-readable source reference for the asset (vendor invoice ref).
    pub vendor_invoice_ref: String,
}

/// Attempt to capitalize one AP bill line.
///
/// Guard:    Check idempotency table; look up active category by asset_account_ref.
/// Mutation: Insert asset + capitalization linkage atomically.
/// Outbox:   Emits asset_created event.
///
/// Returns `Ok(Some(_))` if a new asset was created, `Ok(None)` if:
///   - already processed (idempotent replay), or
///   - no category maps to this gl_account_code (non-capex line).
pub async fn capitalize_from_ap_line(
    pool: &PgPool,
    req: &CapitalizeFromApLineRequest,
) -> Result<Option<CapitalizationResult>, sqlx::Error> {
    if req.amount_minor <= 0 {
        tracing::debug!(
            bill_id = %req.bill_id,
            line_id = %req.line_id,
            "capitalize: skipping zero/negative amount line"
        );
        return Ok(None);
    }

    let mut tx = pool.begin().await?;

    // Guard: idempotency check
    if let Some(cap_id) =
        repo::check_capitalization_exists(&mut *tx, &req.tenant_id, req.bill_id, req.line_id)
            .await?
    {
        tracing::debug!(
            bill_id = %req.bill_id,
            line_id = %req.line_id,
            cap_id = cap_id,
            "capitalize: already processed (idempotent replay)"
        );
        tx.commit().await?;
        return Ok(None);
    }

    // Guard: look up active category by asset_account_ref
    let category = match repo::find_category_by_asset_account(
        &mut *tx,
        &req.tenant_id,
        &req.gl_account_code,
    )
    .await?
    {
        Some(c) => c,
        None => {
            tracing::debug!(
                tenant_id = %req.tenant_id,
                gl_account_code = %req.gl_account_code,
                "capitalize: no active category for gl_account_code; non-capex line"
            );
            tx.commit().await?;
            return Ok(None);
        }
    };

    // Compute asset parameters from category defaults
    let method = DepreciationMethod::try_from(category.default_method.clone())
        .unwrap_or(DepreciationMethod::StraightLine);
    let life_months = category.default_useful_life_months;
    let salvage = 0i64; // Salvage derived at disposal time; zero at acquisition
    let nbv = req.amount_minor - salvage;

    // Asset tag: AP-{line_id} — globally unique since line_id is UUID
    let asset_tag = format!("AP-{}", req.line_id);
    let asset_id = Uuid::new_v4();

    // Mutation: insert asset
    repo::insert_capitalized_asset(
        &mut *tx,
        asset_id,
        &req.tenant_id,
        category.id,
        &asset_tag,
        &req.vendor_invoice_ref,
        &format!("Capitalized from AP bill {}", req.bill_id),
        req.acquisition_date,
        req.amount_minor,
        &req.currency.to_lowercase(),
        method.as_str(),
        life_months,
        nbv,
        &req.vendor_invoice_ref,
        &req.bill_id.to_string(),
        &format!("Source: AP bill {} line {}", req.bill_id, req.line_id),
    )
    .await?;

    // Mutation: insert capitalization linkage
    let cap_id = repo::insert_capitalization(
        &mut *tx,
        &req.tenant_id,
        req.bill_id,
        req.line_id,
        asset_id,
        &req.gl_account_code,
        req.amount_minor,
        &req.currency.to_lowercase(),
        &format!("{}:{}", req.bill_id, req.line_id),
    )
    .await?;

    // Outbox: emit asset_created
    use crate::domain::assets::models::AssetCreatedEvent;
    let event = AssetCreatedEvent {
        asset_id,
        tenant_id: req.tenant_id.clone(),
        asset_tag: asset_tag.clone(),
        category_id: category.id,
        acquisition_cost_minor: req.amount_minor,
        currency: req.currency.to_lowercase(),
    };
    outbox::enqueue_event_tx(
        &mut tx,
        &req.tenant_id,
        Uuid::new_v4(),
        "asset_created",
        "fa_asset",
        &asset_id.to_string(),
        &event,
    )
    .await?;

    tx.commit().await?;

    tracing::info!(
        tenant_id = %req.tenant_id,
        bill_id = %req.bill_id,
        line_id = %req.line_id,
        asset_id = %asset_id,
        category_code = %category.code,
        amount_minor = req.amount_minor,
        "capitalize: created asset from AP bill line"
    );

    Ok(Some(CapitalizationResult {
        asset_id,
        capitalization_id: cap_id,
    }))
}

// Tests in service_tests.rs
