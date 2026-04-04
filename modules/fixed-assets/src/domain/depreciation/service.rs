//! Depreciation service: schedule generation + run execution.
//!
//! Guard → Mutation → Outbox atomicity for the run.
//! Schedule generation is idempotent via ON CONFLICT DO NOTHING.

use sqlx::PgPool;
use uuid::Uuid;

use super::engine;
use super::models::*;
use super::repo;
use crate::outbox;

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

        let asset = repo::fetch_asset_for_schedule(pool, req.asset_id, &req.tenant_id)
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

        repo::insert_schedule_batch(
            pool,
            &req.tenant_id,
            asset.id,
            &asset.currency,
            &periods,
        )
        .await?;

        // Always return the full current schedule from the DB (may include pre-existing rows).
        let schedules = repo::fetch_schedules(pool, asset.id, &req.tenant_id).await?;

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

        let run = repo::insert_run(
            &mut *tx,
            run_id,
            &req.tenant_id,
            req.as_of_date,
            currency,
            req.created_by.as_deref(),
        )
        .await?;

        let posted =
            repo::post_unposted_periods(&mut *tx, run.id, &req.tenant_id, req.as_of_date).await?;

        let periods_posted = posted.len() as i32;
        let total_minor: i64 = posted.iter().map(|s| s.depreciation_amount_minor).sum();
        let assets_processed: i32 = {
            let mut ids: Vec<Uuid> = posted.iter().map(|s| s.asset_id).collect();
            ids.sort();
            ids.dedup();
            ids.len() as i32
        };

        let completed =
            repo::finalize_run(&mut *tx, run.id, assets_processed, periods_posted, total_minor)
                .await?;

        let gl_entry_data =
            repo::query_gl_entries_for_run(&mut tx, completed.id, &req.tenant_id).await?;
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
        let runs = repo::list_runs(pool, tenant_id).await?;
        Ok(runs)
    }

    /// Fetch a single run by id, tenant-scoped.
    pub async fn get_run(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<DepreciationRun>, DepreciationError> {
        let run = repo::get_run(pool, id, tenant_id).await?;
        Ok(run)
    }
}

// Tests in service_tests.rs
