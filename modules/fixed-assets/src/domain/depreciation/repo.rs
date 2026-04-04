//! Depreciation repository — SQL layer for schedules, runs, and GL entries.
//!
//! All raw SQL lives here. The service layer calls these functions
//! for persistence and delegates business logic (validation, engine
//! computation, outbox orchestration) to its own methods.

use chrono::NaiveDate;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::engine::PeriodEntry;
use super::models::*;

/// Minimal projection of fa_assets needed for depreciation — avoids decoding
/// the fa_asset_status PostgreSQL ENUM (which requires a custom sqlx::Type impl).
#[derive(sqlx::FromRow)]
#[allow(dead_code)]
pub struct AssetProjection {
    pub id: Uuid,
    pub tenant_id: String,
    pub in_service_date: Option<NaiveDate>,
    pub acquisition_cost_minor: i64,
    pub salvage_value_minor: i64,
    pub useful_life_months: i32,
    pub depreciation_method: String,
    pub currency: String,
}

// ============================================================================
// Reads
// ============================================================================

/// Fetch the asset projection needed for schedule generation.
pub async fn fetch_asset_for_schedule(
    pool: &PgPool,
    asset_id: Uuid,
    tenant_id: &str,
) -> Result<Option<AssetProjection>, sqlx::Error> {
    sqlx::query_as::<_, AssetProjection>(
        r#"
        SELECT id, tenant_id, in_service_date, acquisition_cost_minor,
               salvage_value_minor, useful_life_months, depreciation_method, currency
        FROM fa_assets
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(asset_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

/// Fetch the full depreciation schedule for an asset, ordered by period.
pub async fn fetch_schedules(
    pool: &PgPool,
    asset_id: Uuid,
    tenant_id: &str,
) -> Result<Vec<DepreciationSchedule>, sqlx::Error> {
    sqlx::query_as::<_, DepreciationSchedule>(
        r#"
        SELECT * FROM fa_depreciation_schedules
        WHERE asset_id = $1 AND tenant_id = $2
        ORDER BY period_number
        "#,
    )
    .bind(asset_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

/// List all runs for a tenant, newest first.
pub async fn list_runs(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<DepreciationRun>, sqlx::Error> {
    sqlx::query_as::<_, DepreciationRun>(
        "SELECT * FROM fa_depreciation_runs WHERE tenant_id = $1 ORDER BY as_of_date DESC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

/// Fetch a single run by id, tenant-scoped.
pub async fn get_run(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
) -> Result<Option<DepreciationRun>, sqlx::Error> {
    sqlx::query_as::<_, DepreciationRun>(
        "SELECT * FROM fa_depreciation_runs WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

// ============================================================================
// Writes (pool — schedule generation is not transactional)
// ============================================================================

/// Batch-insert depreciation schedule periods using UNNEST arrays.
///
/// Idempotent: ON CONFLICT (asset_id, period_number) DO NOTHING.
pub async fn insert_schedule_batch(
    pool: &PgPool,
    tenant_id: &str,
    asset_id: Uuid,
    currency: &str,
    periods: &[PeriodEntry],
) -> Result<(), sqlx::Error> {
    if periods.is_empty() {
        return Ok(());
    }

    let n = periods.len();
    let ids: Vec<Uuid> = (0..n).map(|_| Uuid::new_v4()).collect();
    let tenant_ids: Vec<&str> = vec![tenant_id; n];
    let asset_ids: Vec<Uuid> = vec![asset_id; n];
    let period_numbers: Vec<i32> = periods.iter().map(|p| p.period_number).collect();
    let period_starts: Vec<NaiveDate> = periods.iter().map(|p| p.period_start).collect();
    let period_ends: Vec<NaiveDate> = periods.iter().map(|p| p.period_end).collect();
    let amounts: Vec<i64> = periods.iter().map(|p| p.depreciation_amount_minor).collect();
    let currencies: Vec<&str> = vec![currency; n];
    let cumulatives: Vec<i64> = periods
        .iter()
        .map(|p| p.cumulative_depreciation_minor)
        .collect();
    let remainings: Vec<i64> = periods
        .iter()
        .map(|p| p.remaining_book_value_minor)
        .collect();

    sqlx::query(
        r#"
        INSERT INTO fa_depreciation_schedules
            (id, tenant_id, asset_id, period_number,
             period_start, period_end,
             depreciation_amount_minor, currency,
             cumulative_depreciation_minor, remaining_book_value_minor,
             is_posted, created_at, updated_at)
        SELECT * FROM UNNEST(
            $1::UUID[], $2::TEXT[], $3::UUID[], $4::INT[],
            $5::DATE[], $6::DATE[],
            $7::BIGINT[], $8::TEXT[],
            $9::BIGINT[], $10::BIGINT[],
            ARRAY_FILL(FALSE, ARRAY[$11]),
            ARRAY_FILL(NOW()::TIMESTAMPTZ, ARRAY[$11]),
            ARRAY_FILL(NOW()::TIMESTAMPTZ, ARRAY[$11])
        )
        ON CONFLICT (asset_id, period_number) DO NOTHING
        "#,
    )
    .bind(&ids)
    .bind(&tenant_ids)
    .bind(&asset_ids)
    .bind(&period_numbers)
    .bind(&period_starts)
    .bind(&period_ends)
    .bind(&amounts)
    .bind(&currencies)
    .bind(&cumulatives)
    .bind(&remainings)
    .bind(n as i32)
    .execute(pool)
    .await?;

    Ok(())
}

// ============================================================================
// Writes (transactional — depreciation run)
// ============================================================================

/// Insert a new depreciation run in 'running' state.
pub async fn insert_run(
    conn: &mut PgConnection,
    run_id: Uuid,
    tenant_id: &str,
    as_of_date: NaiveDate,
    currency: &str,
    created_by: Option<&str>,
) -> Result<DepreciationRun, sqlx::Error> {
    sqlx::query_as(
        r#"
        INSERT INTO fa_depreciation_runs
            (id, tenant_id, as_of_date, status,
             assets_processed, periods_posted, total_depreciation_minor, currency,
             idempotency_key, started_at, created_at, updated_at, created_by)
        VALUES ($1,$2,$3,'running',0,0,0,$4,gen_random_uuid(),NOW(),NOW(),NOW(),$5)
        RETURNING *
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .bind(as_of_date)
    .bind(currency)
    .bind(created_by)
    .fetch_one(conn)
    .await
}

/// Post all unposted schedule periods up to as_of_date for non-disposed assets.
pub async fn post_unposted_periods(
    conn: &mut PgConnection,
    run_id: Uuid,
    tenant_id: &str,
    as_of_date: NaiveDate,
) -> Result<Vec<DepreciationSchedule>, sqlx::Error> {
    sqlx::query_as(
        r#"
        UPDATE fa_depreciation_schedules
        SET
            is_posted        = TRUE,
            posted_at        = NOW(),
            posted_by_run_id = $1,
            updated_at       = NOW()
        WHERE tenant_id  = $2
          AND period_end <= $3
          AND is_posted   = FALSE
          AND asset_id IN (
              SELECT id FROM fa_assets
              WHERE tenant_id = $2
                AND status NOT IN ('disposed', 'impaired')
          )
        RETURNING *
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .bind(as_of_date)
    .fetch_all(conn)
    .await
}

/// Finalize a depreciation run with computed stats.
pub async fn finalize_run(
    conn: &mut PgConnection,
    run_id: Uuid,
    assets_processed: i32,
    periods_posted: i32,
    total_minor: i64,
) -> Result<DepreciationRun, sqlx::Error> {
    sqlx::query_as(
        r#"
        UPDATE fa_depreciation_runs
        SET
            status                   = 'completed',
            assets_processed         = $2,
            periods_posted           = $3,
            total_depreciation_minor = $4,
            completed_at             = NOW(),
            updated_at               = NOW()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(run_id)
    .bind(assets_processed)
    .bind(periods_posted)
    .bind(total_minor)
    .fetch_one(conn)
    .await
}

/// Fetch per-entry GL posting data for a completed depreciation run.
///
/// Joins schedules → assets → categories to obtain account refs.
/// Called inside the same transaction for consistency.
pub async fn query_gl_entries_for_run(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    tenant_id: &str,
) -> Result<Vec<DepreciationGlEntry>, sqlx::Error> {
    sqlx::query_as::<_, DepreciationGlEntry>(
        r#"
        SELECT
            s.id                          AS entry_id,
            s.asset_id,
            s.period_end,
            s.depreciation_amount_minor,
            s.currency,
            c.depreciation_expense_ref    AS expense_account_ref,
            c.accum_depreciation_ref
        FROM fa_depreciation_schedules s
        JOIN fa_assets     a ON a.id          = s.asset_id      AND a.tenant_id = s.tenant_id
        JOIN fa_categories c ON c.id          = a.category_id   AND c.tenant_id = a.tenant_id
        WHERE s.posted_by_run_id = $1
          AND s.tenant_id        = $2
        ORDER BY s.asset_id, s.period_number
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .fetch_all(&mut **tx)
    .await
}
