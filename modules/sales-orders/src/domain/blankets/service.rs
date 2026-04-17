//! Blanket order service — business logic, state transitions, release with over-draw protection.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    repo, ActivateBlanketRequest, BlanketError, BlanketOrder, BlanketOrderLine,
    BlanketOrderRelease, BlanketOrderWithLines, BlanketStatus, CreateBlanketLineRequest,
    CreateBlanketRequest, CreateReleaseRequest, ListBlanketsQuery, UpdateBlanketRequest,
};

pub async fn create_blanket(
    pool: &PgPool,
    tenant_id: &str,
    created_by: &str,
    req: CreateBlanketRequest,
) -> Result<BlanketOrder, BlanketError> {
    let id = Uuid::new_v4();
    let blanket_number = generate_blanket_number();
    let effective_date = req
        .effective_date
        .unwrap_or_else(|| Utc::now().date_naive());

    Ok(repo::insert_blanket(
        pool,
        id,
        tenant_id,
        &blanket_number,
        req.customer_id,
        req.party_id,
        &req.currency,
        effective_date,
        req.expiry_date,
        req.notes.as_deref(),
        created_by,
    )
    .await?)
}

pub async fn get_blanket_with_lines(
    pool: &PgPool,
    tenant_id: &str,
    blanket_id: Uuid,
) -> Result<BlanketOrderWithLines, BlanketError> {
    let order = repo::fetch_blanket(pool, blanket_id, tenant_id)
        .await?
        .ok_or(BlanketError::NotFound(blanket_id))?;
    let lines = repo::fetch_blanket_lines(pool, blanket_id, tenant_id).await?;
    Ok(BlanketOrderWithLines { order, lines })
}

pub async fn list_blankets(
    pool: &PgPool,
    tenant_id: &str,
    query: &ListBlanketsQuery,
) -> Result<Vec<BlanketOrder>, BlanketError> {
    Ok(repo::list_blankets(
        pool,
        tenant_id,
        query.customer_id,
        query.status.as_deref(),
        query.limit,
        query.offset,
    )
    .await?)
}

pub async fn update_blanket(
    pool: &PgPool,
    tenant_id: &str,
    blanket_id: Uuid,
    req: UpdateBlanketRequest,
) -> Result<BlanketOrder, BlanketError> {
    let blanket = repo::fetch_blanket(pool, blanket_id, tenant_id)
        .await?
        .ok_or(BlanketError::NotFound(blanket_id))?;

    let status = BlanketStatus::from_str(&blanket.status).unwrap_or(BlanketStatus::Draft);
    if status != BlanketStatus::Draft && status != BlanketStatus::Active {
        return Err(BlanketError::NotEditable(blanket.status));
    }

    Ok(repo::update_blanket_header(
        pool,
        blanket_id,
        tenant_id,
        req.customer_id,
        req.party_id,
        req.expiry_date,
        req.notes.as_deref(),
    )
    .await?)
}

pub async fn activate_blanket(
    pool: &PgPool,
    tenant_id: &str,
    blanket_id: Uuid,
    _req: ActivateBlanketRequest,
) -> Result<BlanketOrder, BlanketError> {
    let blanket = repo::fetch_blanket(pool, blanket_id, tenant_id)
        .await?
        .ok_or(BlanketError::NotFound(blanket_id))?;

    let current = BlanketStatus::from_str(&blanket.status).unwrap_or(BlanketStatus::Draft);
    if !current.can_transition_to(BlanketStatus::Active) {
        return Err(BlanketError::InvalidTransition {
            from: blanket.status,
            to: "active".to_string(),
        });
    }

    repo::update_blanket_status(pool, blanket_id, tenant_id, BlanketStatus::Active.as_str())
        .await?;

    repo::fetch_blanket(pool, blanket_id, tenant_id)
        .await?
        .ok_or(BlanketError::NotFound(blanket_id))
}

pub async fn add_blanket_line(
    pool: &PgPool,
    tenant_id: &str,
    blanket_id: Uuid,
    req: CreateBlanketLineRequest,
) -> Result<BlanketOrderLine, BlanketError> {
    let blanket = repo::fetch_blanket(pool, blanket_id, tenant_id)
        .await?
        .ok_or(BlanketError::NotFound(blanket_id))?;

    let status = BlanketStatus::from_str(&blanket.status).unwrap_or(BlanketStatus::Draft);
    if status != BlanketStatus::Draft {
        return Err(BlanketError::NotEditable(blanket.status));
    }

    if req.committed_qty <= 0.0 {
        return Err(BlanketError::Validation(
            "committed_qty must be positive".to_string(),
        ));
    }

    let uom = req.uom.as_deref().unwrap_or("EA");
    Ok(repo::insert_blanket_line(
        pool,
        Uuid::new_v4(),
        tenant_id,
        blanket_id,
        req.item_id,
        req.part_number.as_deref(),
        &req.description,
        uom,
        req.committed_qty,
        req.unit_price_cents,
        req.notes.as_deref(),
    )
    .await?)
}

/// Create a release with over-draw protection via SELECT FOR UPDATE.
pub async fn create_release(
    pool: &PgPool,
    tenant_id: &str,
    blanket_id: Uuid,
    req: CreateReleaseRequest,
) -> Result<BlanketOrderRelease, BlanketError> {
    // Verify blanket is active
    let blanket = repo::fetch_blanket(pool, blanket_id, tenant_id)
        .await?
        .ok_or(BlanketError::NotFound(blanket_id))?;

    if BlanketStatus::from_str(&blanket.status) != Some(BlanketStatus::Active) {
        return Err(BlanketError::NotEditable(blanket.status));
    }

    if req.release_qty <= 0.0 {
        return Err(BlanketError::Validation(
            "release_qty must be positive".to_string(),
        ));
    }

    let release_date = req
        .release_date
        .unwrap_or_else(|| Utc::now().date_naive());

    // Over-draw check with row-level lock
    let mut tx = pool.begin().await?;

    let line = repo::fetch_blanket_line_for_update(pool, req.blanket_line_id, tenant_id)
        .await?
        .ok_or(BlanketError::NotFound(req.blanket_line_id))?;

    let remaining = line.committed_qty - line.released_qty;
    if req.release_qty > remaining {
        return Err(BlanketError::OverDraw {
            requested: req.release_qty,
            remaining,
        });
    }

    let release = repo::insert_release_and_update_line(
        &mut *tx,
        Uuid::new_v4(),
        tenant_id,
        blanket_id,
        req.blanket_line_id,
        req.release_qty,
        release_date,
        req.notes.as_deref(),
    )
    .await?;

    repo::increment_line_released_qty(&mut *tx, req.blanket_line_id, tenant_id, req.release_qty)
        .await?;

    tx.commit().await?;

    Ok(release)
}

pub async fn get_releases_for_blanket(
    pool: &PgPool,
    tenant_id: &str,
    blanket_id: Uuid,
    line_id: Uuid,
) -> Result<Vec<BlanketOrderRelease>, BlanketError> {
    // Verify blanket exists and belongs to tenant
    repo::fetch_blanket(pool, blanket_id, tenant_id)
        .await?
        .ok_or(BlanketError::NotFound(blanket_id))?;

    Ok(repo::fetch_releases_for_line(pool, line_id, tenant_id).await?)
}

fn generate_blanket_number() -> String {
    let now = chrono::Utc::now();
    format!(
        "BL-{}-{:06}",
        now.format("%Y%m%d"),
        fastrand::u32(0..1_000_000)
    )
}
