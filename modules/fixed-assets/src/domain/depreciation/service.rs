//! Depreciation service: schedule generation + run execution.
//!
//! Guard → Mutation → Outbox atomicity for the run.
//! Schedule generation is idempotent via ON CONFLICT DO NOTHING.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::engine;
use super::gl_entries;
use super::models::*;
use crate::outbox;

/// Minimal projection of fa_assets needed for depreciation — avoids decoding
/// the fa_asset_status PostgreSQL ENUM (which requires a custom sqlx::Type impl).
#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct AssetProjection {
    id: Uuid,
    tenant_id: String,
    in_service_date: Option<NaiveDate>,
    acquisition_cost_minor: i64,
    salvage_value_minor: i64,
    useful_life_months: i32,
    depreciation_method: String,
    currency: String,
}

pub struct DepreciationService;

impl DepreciationService {
    /// Generate the straight-line schedule for a single asset.
    ///
    /// Inserts one row per period into fa_depreciation_schedules.
    /// Idempotent: ON CONFLICT (asset_id, period_number) DO NOTHING means
    /// re-running produces no duplicate rows.
    ///
    /// Returns the current complete schedule (existing + newly inserted).
    pub async fn generate_schedule(
        pool: &PgPool,
        req: &GenerateScheduleRequest,
    ) -> Result<Vec<DepreciationSchedule>, DepreciationError> {
        req.validate()?;

        let asset = sqlx::query_as::<_, AssetProjection>(
            r#"
            SELECT id, tenant_id, in_service_date, acquisition_cost_minor,
                   salvage_value_minor, useful_life_months, depreciation_method, currency
            FROM fa_assets
            WHERE id = $1 AND tenant_id = $2
            "#,
        )
        .bind(req.asset_id)
        .bind(&req.tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(DepreciationError::AssetNotFound(req.asset_id))?;

        let in_service_date = asset
            .in_service_date
            .ok_or(DepreciationError::AssetNotInService(asset.id))?;

        if asset.depreciation_method != "straight_line" {
            return Err(DepreciationError::UnsupportedMethod(
                asset.depreciation_method.clone(),
            ));
        }

        let periods = engine::compute_straight_line(
            in_service_date,
            asset.acquisition_cost_minor,
            asset.salvage_value_minor,
            asset.useful_life_months,
        );

        for p in &periods {
            sqlx::query(
                r#"
                INSERT INTO fa_depreciation_schedules
                    (id, tenant_id, asset_id, period_number,
                     period_start, period_end,
                     depreciation_amount_minor, currency,
                     cumulative_depreciation_minor, remaining_book_value_minor,
                     is_posted, created_at, updated_at)
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,FALSE,NOW(),NOW())
                ON CONFLICT (asset_id, period_number) DO NOTHING
                "#,
            )
            .bind(Uuid::new_v4())
            .bind(&req.tenant_id)
            .bind(asset.id)
            .bind(p.period_number)
            .bind(p.period_start)
            .bind(p.period_end)
            .bind(p.depreciation_amount_minor)
            .bind(&asset.currency)
            .bind(p.cumulative_depreciation_minor)
            .bind(p.remaining_book_value_minor)
            .execute(pool)
            .await?;
        }

        // Always return the full current schedule from the DB (may include pre-existing rows).
        let schedules = sqlx::query_as::<_, DepreciationSchedule>(
            r#"
            SELECT * FROM fa_depreciation_schedules
            WHERE asset_id = $1 AND tenant_id = $2
            ORDER BY period_number
            "#,
        )
        .bind(asset.id)
        .bind(&req.tenant_id)
        .fetch_all(pool)
        .await?;

        Ok(schedules)
    }

    /// Execute a depreciation run: post all unposted periods up to as_of_date.
    ///
    /// Guard → Mutation → Outbox in a single transaction.
    /// Idempotent: periods already marked is_posted=TRUE are skipped.
    pub async fn run(
        pool: &PgPool,
        req: &CreateRunRequest,
    ) -> Result<DepreciationRun, DepreciationError> {
        req.validate()?;

        let currency = req.currency.as_deref().unwrap_or("usd");
        let run_id = Uuid::new_v4();

        let mut tx = pool.begin().await?;

        // Insert run in 'running' state (skips pending → running transition for simplicity).
        let run: DepreciationRun = sqlx::query_as(
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
        .bind(&req.tenant_id)
        .bind(req.as_of_date)
        .bind(currency)
        .bind(req.created_by.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        // Post all unposted periods up to as_of_date in one UPDATE.
        // Guard: skip schedules for disposed or impaired assets — depreciation stops at disposal.
        let posted: Vec<DepreciationSchedule> = sqlx::query_as(
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
        .bind(run.id)
        .bind(&req.tenant_id)
        .bind(req.as_of_date)
        .fetch_all(&mut *tx)
        .await?;

        let periods_posted = posted.len() as i32;
        let total_minor: i64 = posted.iter().map(|s| s.depreciation_amount_minor).sum();
        let assets_processed: i32 = {
            let mut ids: Vec<Uuid> = posted.iter().map(|s| s.asset_id).collect();
            ids.sort();
            ids.dedup();
            ids.len() as i32
        };

        // Finalise run stats.
        let completed: DepreciationRun = sqlx::query_as(
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
        .bind(run.id)
        .bind(assets_processed)
        .bind(periods_posted)
        .bind(total_minor)
        .fetch_one(&mut *tx)
        .await?;

        let gl_entry_data =
            gl_entries::query_for_run(&mut tx, completed.id, &req.tenant_id).await?;
        let event = DepreciationRunCompletedEvent {
            run_id: completed.id,
            tenant_id: req.tenant_id.clone(),
            as_of_date: req.as_of_date,
            periods_posted,
            total_depreciation_minor: total_minor,
            gl_entries: gl_entry_data,
        };
        outbox::enqueue_event_tx(
            &mut tx,
            &req.tenant_id,
            Uuid::new_v4(),
            "depreciation_run_completed",
            "fa_depreciation_run",
            &completed.id.to_string(),
            &event,
        )
        .await?;

        tx.commit().await?;
        Ok(completed)
    }

    /// List all runs for a tenant, newest first.
    pub async fn list_runs(
        pool: &PgPool,
        tenant_id: &str,
    ) -> Result<Vec<DepreciationRun>, DepreciationError> {
        let runs = sqlx::query_as::<_, DepreciationRun>(
            "SELECT * FROM fa_depreciation_runs WHERE tenant_id = $1 ORDER BY as_of_date DESC",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;
        Ok(runs)
    }

    /// Fetch a single run by id, tenant-scoped.
    pub async fn get_run(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<DepreciationRun>, DepreciationError> {
        let run = sqlx::query_as::<_, DepreciationRun>(
            "SELECT * FROM fa_depreciation_runs WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;
        Ok(run)
    }
}

// ============================================================================
// Integrated tests — require a running fixed-assets Postgres instance
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serial_test::serial;

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db"
                .to_string()
        })
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("connect to fixed-assets test DB")
    }

    const TEST_TENANT: &str = "test-depr-svc";

    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM fa_depreciation_schedules WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
        sqlx::query(
            "DELETE FROM fa_depreciation_runs WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
        sqlx::query(
            "DELETE FROM fa_events_outbox WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
        sqlx::query("DELETE FROM fa_assets WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM fa_categories WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();
    }

    /// Insert category + asset directly (avoiding RETURNING * with the fa_asset_status ENUM).
    async fn create_test_asset(pool: &PgPool, in_service_date: NaiveDate) -> Uuid {
        let cat_id = Uuid::new_v4();
        let tag = format!("CAT-{}", &cat_id.to_string()[..8]);
        sqlx::query(
            r#"
            INSERT INTO fa_categories
                (id, tenant_id, code, name,
                 default_method, default_useful_life_months, default_salvage_pct_bp,
                 asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
                 is_active, created_at, updated_at)
            VALUES ($1,$2,$3,$4,'straight_line',12,0,'1500','6100','1510',TRUE,NOW(),NOW())
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
        let asset_tag = format!("FA-{}", &asset_id.to_string()[..8]);
        sqlx::query(
            r#"
            INSERT INTO fa_assets
                (id, tenant_id, category_id, asset_tag, name,
                 acquisition_date, in_service_date,
                 acquisition_cost_minor, currency,
                 depreciation_method, useful_life_months, salvage_value_minor,
                 accum_depreciation_minor, net_book_value_minor,
                 created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$6,120000,'usd','straight_line',12,0,0,120000,NOW(),NOW())
            "#,
        )
        .bind(asset_id)
        .bind(TEST_TENANT)
        .bind(cat_id)
        .bind(asset_tag)
        .bind("Test Asset")
        .bind(in_service_date)
        .execute(pool)
        .await
        .expect("insert test asset");

        asset_id
    }

    #[tokio::test]
    #[serial]
    async fn generate_schedule_creates_12_periods() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let asset_id = create_test_asset(&pool, in_service).await;

        let req = GenerateScheduleRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
        };
        let schedules = DepreciationService::generate_schedule(&pool, &req)
            .await
            .unwrap();

        assert_eq!(schedules.len(), 12);
        let total: i64 = schedules
            .iter()
            .map(|s| s.depreciation_amount_minor)
            .sum();
        assert_eq!(total, 120_000);
        assert_eq!(schedules[0].period_number, 1);
        assert_eq!(schedules[11].period_number, 12);
        assert_eq!(schedules[11].remaining_book_value_minor, 0);
    }

    #[tokio::test]
    #[serial]
    async fn generate_schedule_is_idempotent() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let asset_id = create_test_asset(&pool, in_service).await;

        let req = GenerateScheduleRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
        };
        let first = DepreciationService::generate_schedule(&pool, &req)
            .await
            .unwrap();
        let second = DepreciationService::generate_schedule(&pool, &req)
            .await
            .unwrap();

        assert_eq!(first.len(), second.len(), "no duplicate rows on rerun");
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.id, b.id, "same row ids — no new inserts");
        }
    }

    #[tokio::test]
    #[serial]
    async fn run_posts_periods_and_is_idempotent() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let asset_id = create_test_asset(&pool, in_service).await;

        // Generate schedule first
        let sched_req = GenerateScheduleRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
        };
        DepreciationService::generate_schedule(&pool, &sched_req)
            .await
            .unwrap();

        // Run through 2026-06-30 → should post periods 1-6
        let run_req = CreateRunRequest {
            tenant_id: TEST_TENANT.into(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
            currency: None,
            created_by: None,
        };
        let run1 = DepreciationService::run(&pool, &run_req).await.unwrap();
        assert_eq!(run1.status, "completed");
        assert_eq!(run1.periods_posted, 6);
        assert_eq!(run1.assets_processed, 1);
        assert_eq!(run1.total_depreciation_minor, 60_000);

        // Rerun same as_of_date → no new periods posted
        let run2 = DepreciationService::run(&pool, &run_req).await.unwrap();
        assert_eq!(run2.status, "completed");
        assert_eq!(run2.periods_posted, 0, "already posted — idempotent");
        assert_eq!(run2.total_depreciation_minor, 0);
    }

    #[tokio::test]
    #[serial]
    async fn run_respects_as_of_date_boundary() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let asset_id = create_test_asset(&pool, in_service).await;

        DepreciationService::generate_schedule(
            &pool,
            &GenerateScheduleRequest {
                tenant_id: TEST_TENANT.into(),
                asset_id,
            },
        )
        .await
        .unwrap();

        // Only one period ending 2026-01-31 should be posted
        let run = DepreciationService::run(
            &pool,
            &CreateRunRequest {
                tenant_id: TEST_TENANT.into(),
                as_of_date: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
                currency: None,
                created_by: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(run.periods_posted, 1);
        assert_eq!(run.total_depreciation_minor, 10_000); // 120_000 / 12
    }
}
