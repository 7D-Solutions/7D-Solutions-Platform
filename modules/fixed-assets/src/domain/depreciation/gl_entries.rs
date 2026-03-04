//! Query helper: fetch per-entry GL posting data for a completed depreciation run.
//!
//! Called from within the run transaction so the data is consistent with the
//! just-posted schedule rows.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::models::DepreciationGlEntry;

/// Fetch GL entry data for all schedules posted in `run_id`.
///
/// Joins `fa_depreciation_schedules` → `fa_assets` → `fa_categories` to obtain
/// the category-configured account refs required by the GL consumer.
///
/// Called inside the same transaction that executed the depreciation run so
/// results are guaranteed to be consistent.
pub async fn query_for_run(
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

// ============================================================================
// Integrated tests — require running fixed-assets Postgres instance
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serial_test::serial;
    use sqlx::PgPool;
    use uuid::Uuid;

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db?sslmode=require"
                .to_string()
        })
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("connect to fixed-assets test DB")
    }

    const TEST_TENANT: &str = "test-gl-entries-query";

    async fn cleanup(pool: &PgPool) {
        for q in [
            "DELETE FROM fa_depreciation_schedules WHERE tenant_id = $1",
            "DELETE FROM fa_depreciation_runs     WHERE tenant_id = $1",
            "DELETE FROM fa_events_outbox          WHERE tenant_id = $1",
            "DELETE FROM fa_assets                 WHERE tenant_id = $1",
            "DELETE FROM fa_categories             WHERE tenant_id = $1",
        ] {
            sqlx::query(q).bind(TEST_TENANT).execute(pool).await.ok();
        }
    }

    async fn insert_category_and_asset(pool: &PgPool, in_service: NaiveDate) -> (Uuid, Uuid) {
        let cat_id = Uuid::new_v4();
        let tag = format!("GE-{}", &cat_id.to_string()[..8]);
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
        .expect("insert category");

        let asset_id = Uuid::new_v4();
        let atag = format!("FA-{}", &asset_id.to_string()[..8]);
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
        .bind(atag)
        .bind("Test Asset")
        .bind(in_service)
        .execute(pool)
        .await
        .expect("insert asset");

        (cat_id, asset_id)
    }

    #[tokio::test]
    #[serial]
    async fn query_returns_entries_with_account_refs() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let in_service =
            NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid fixed test date literal");
        let (_, asset_id) = insert_category_and_asset(&pool, in_service).await;

        // Insert one schedule row, mark it posted with a run_id
        let schedule_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO fa_depreciation_schedules
                (id, tenant_id, asset_id, period_number,
                 period_start, period_end,
                 depreciation_amount_minor, currency,
                 cumulative_depreciation_minor, remaining_book_value_minor,
                 is_posted, posted_at, posted_by_run_id,
                 created_at, updated_at)
            VALUES ($1,$2,$3,1,'2026-01-01','2026-01-31',10000,'usd',10000,110000,
                    TRUE,NOW(),$4,NOW(),NOW())
            "#,
        )
        .bind(schedule_id)
        .bind(TEST_TENANT)
        .bind(asset_id)
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("insert schedule");

        let mut tx = pool.begin().await.expect("begin tx");
        let entries = query_for_run(&mut tx, run_id, TEST_TENANT)
            .await
            .expect("query_for_run");
        tx.rollback().await.ok();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entry_id, schedule_id);
        assert_eq!(entries[0].depreciation_amount_minor, 10000);
        assert_eq!(entries[0].expense_account_ref, "6100");
        assert_eq!(entries[0].accum_depreciation_ref, "1510");

        cleanup(&pool).await;
    }
}
