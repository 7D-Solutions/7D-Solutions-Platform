//! Maintenance scheduler tick — evaluates due plan assignments and emits events.
//!
//! Design:
//! - Runs on a configurable interval (MAINTENANCE_SCHED_INTERVAL_SECS, default 60s).
//! - Finds active assignments where due_notified_at IS NULL and either:
//!   - next_due_date <= today (calendar trigger), OR
//!   - next_due_meter <= max reading for that asset+meter_type (meter trigger)
//! - For each due assignment, atomically: enqueue `maintenance.plan.due` event
//!   + set due_notified_at = NOW().
//! - If tenant has auto_create_on_due=true, a work order is created in the same tx.
//!   - approvals_required=true → WO starts as awaiting_approval; else scheduled.
//! - Idempotency: due_notified_at prevents re-emission until reset (after WO completion).
//! - FOR UPDATE SKIP LOCKED prevents concurrent scheduler instances from double-processing.

use chrono::{NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Result of a single scheduler tick.
#[derive(Debug, Default)]
pub struct TickResult {
    pub evaluated: usize,
    pub events_emitted: usize,
    pub work_orders_created: usize,
}

/// A due assignment row returned by the finder query.
#[derive(Debug, sqlx::FromRow)]
struct DueAssignment {
    id: Uuid,
    tenant_id: String,
    plan_id: Uuid,
    asset_id: Uuid,
    next_due_date: Option<NaiveDate>,
    next_due_meter: Option<i64>,
    // Joined from maintenance_plans
    plan_name: String,
    plan_priority: String,
    schedule_type: String,
    meter_type_id: Option<Uuid>,
    task_checklist: Option<serde_json::Value>,
}

/// Event payload for `maintenance.plan.due`.
#[derive(Debug, Serialize)]
struct PlanDueEvent {
    assignment_id: Uuid,
    plan_id: Uuid,
    asset_id: Uuid,
    tenant_id: String,
    trigger_type: String,
    next_due_date: Option<NaiveDate>,
    next_due_meter: Option<i64>,
    plan_name: String,
    plan_priority: String,
}

/// Determine which trigger fired for a due assignment.
fn classify_trigger(schedule_type: &str, date_due: bool, meter_due: bool) -> &'static str {
    match schedule_type {
        "both" if date_due && meter_due => "both",
        "both" if date_due => "calendar",
        "both" if meter_due => "meter",
        "calendar" => "calendar",
        "meter" => "meter",
        _ => "unknown",
    }
}

/// Run one scheduler evaluation tick.
///
/// Returns the number of assignments evaluated and events emitted.
/// Each due assignment is processed in its own transaction for isolation.
pub async fn evaluate_due(pool: &PgPool) -> Result<TickResult, sqlx::Error> {
    let today = Utc::now().date_naive();
    let mut result = TickResult::default();

    // Find all due assignments in a single query.
    // Calendar trigger: next_due_date <= today
    // Meter trigger: next_due_meter <= MAX(reading_value) for that asset+meter_type
    let due_assignments = sqlx::query_as::<_, DueAssignment>(
        r#"
        SELECT
            a.id,
            a.tenant_id,
            a.plan_id,
            a.asset_id,
            a.next_due_date,
            a.next_due_meter,
            p.name AS plan_name,
            p.priority AS plan_priority,
            p.schedule_type,
            p.meter_type_id,
            p.task_checklist
        FROM maintenance_plan_assignments a
        JOIN maintenance_plans p ON a.plan_id = p.id AND p.tenant_id = a.tenant_id
        WHERE a.state = 'active'
          AND a.due_notified_at IS NULL
          AND p.is_active = true
          AND (
              (a.next_due_date IS NOT NULL AND a.next_due_date <= $1)
              OR
              (a.next_due_meter IS NOT NULL AND p.meter_type_id IS NOT NULL AND EXISTS (
                  SELECT 1 FROM meter_readings mr
                  WHERE mr.tenant_id = a.tenant_id
                    AND mr.asset_id = a.asset_id
                    AND mr.meter_type_id = p.meter_type_id
                    AND mr.reading_value >= a.next_due_meter
              ))
          )
        ORDER BY a.tenant_id, a.created_at
        "#,
    )
    .bind(today)
    .fetch_all(pool)
    .await?;

    result.evaluated = due_assignments.len();

    for assignment in &due_assignments {
        let date_due = assignment
            .next_due_date
            .map(|d| d <= today)
            .unwrap_or(false);

        // Determine if meter threshold is exceeded.
        // For meter-only schedules, the EXISTS sub-query in the finder guarantees it.
        // For "both" schedules, re-check the actual reading to classify the trigger.
        let meter_due = if assignment.next_due_meter.is_some() && assignment.meter_type_id.is_some()
        {
            if assignment.schedule_type == "meter" {
                true // EXISTS in finder query guarantees it
            } else {
                let reading: Option<i64> = sqlx::query_scalar(
                    r#"
                    SELECT MAX(reading_value) FROM meter_readings
                    WHERE tenant_id = $1 AND asset_id = $2 AND meter_type_id = $3
                    "#,
                )
                .bind(&assignment.tenant_id)
                .bind(assignment.asset_id)
                .bind(assignment.meter_type_id)
                .fetch_one(pool)
                .await?;

                reading.unwrap_or(0) >= assignment.next_due_meter.unwrap_or(i64::MAX)
            }
        } else {
            false
        };

        let trigger = classify_trigger(&assignment.schedule_type, date_due, meter_due);

        let event = PlanDueEvent {
            assignment_id: assignment.id,
            plan_id: assignment.plan_id,
            asset_id: assignment.asset_id,
            tenant_id: assignment.tenant_id.clone(),
            trigger_type: trigger.to_string(),
            next_due_date: assignment.next_due_date,
            next_due_meter: assignment.next_due_meter,
            plan_name: assignment.plan_name.clone(),
            plan_priority: assignment.plan_priority.clone(),
        };

        // Atomic: enqueue event + mark notified in same transaction
        let mut tx = pool.begin().await?;

        // Lock this specific assignment row to prevent concurrent processing
        let locked: Option<(Uuid,)> = sqlx::query_as(
            r#"
            SELECT id FROM maintenance_plan_assignments
            WHERE id = $1 AND due_notified_at IS NULL
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .bind(assignment.id)
        .fetch_optional(&mut *tx)
        .await?;

        if locked.is_none() {
            // Another scheduler instance already processed this
            tx.rollback().await?;
            continue;
        }

        let sched_event_id = Uuid::new_v4();
        let env = crate::events::envelope::create_envelope(
            sched_event_id,
            assignment.tenant_id.clone(),
            crate::events::subjects::PLAN_DUE.to_string(),
            &event,
        );
        let env_json = crate::events::envelope::validate_envelope(&env).map_err(|e| {
            sqlx::Error::Encode(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Envelope validation: {}", e),
            )))
        })?;
        crate::outbox::enqueue_event_tx(
            &mut tx,
            sched_event_id,
            crate::events::subjects::PLAN_DUE,
            "plan_assignment",
            &assignment.id.to_string(),
            &env_json,
        )
        .await?;

        sqlx::query(
            r#"
            UPDATE maintenance_plan_assignments
            SET due_notified_at = $2, updated_at = $2
            WHERE id = $1
            "#,
        )
        .bind(assignment.id)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;

        // Auto-create work order if tenant config enables it
        let config = super::tenant_config::TenantConfigRepo::get_or_default_tx(
            &mut tx,
            &assignment.tenant_id,
        )
        .await?;

        if config.auto_create_on_due {
            let initial_status = if config.approvals_required {
                "awaiting_approval"
            } else {
                "scheduled"
            };

            let title = format!("[Auto] {}", assignment.plan_name);
            super::work_orders::WorkOrderRepo::create_from_due_tx(
                &mut tx,
                &assignment.tenant_id,
                assignment.asset_id,
                assignment.id,
                &title,
                &assignment.plan_priority,
                initial_status,
                assignment.task_checklist.as_ref(),
            )
            .await
            .map_err(|e| match e {
                super::work_orders::WoError::Database(db_err) => db_err,
                other => sqlx::Error::Protocol(other.to_string()),
            })?;

            result.work_orders_created += 1;

            tracing::info!(
                assignment_id = %assignment.id,
                initial_status = initial_status,
                "auto-created work order from due plan"
            );
        }

        tx.commit().await?;
        result.events_emitted += 1;

        tracing::info!(
            assignment_id = %assignment.id,
            plan_id = %assignment.plan_id,
            asset_id = %assignment.asset_id,
            tenant_id = %assignment.tenant_id,
            trigger = trigger,
            "maintenance.plan.due event emitted"
        );
    }

    Ok(result)
}

/// Background scheduler loop — calls evaluate_due on each tick.
pub async fn run_scheduler_task(pool: PgPool, interval_secs: u64) {
    tracing::info!(
        interval_secs = interval_secs,
        "Maintenance: starting scheduler task"
    );

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    let mut tick_count: u64 = 0;

    loop {
        interval.tick().await;
        tick_count += 1;

        match evaluate_due(&pool).await {
            Ok(result) if result.events_emitted > 0 => {
                tracing::info!(
                    tick = tick_count,
                    evaluated = result.evaluated,
                    emitted = result.events_emitted,
                    wos_created = result.work_orders_created,
                    "Maintenance scheduler: due events emitted"
                );
            }
            Ok(result) => {
                if tick_count <= 3 || tick_count.is_multiple_of(60) {
                    tracing::debug!(
                        tick = tick_count,
                        evaluated = result.evaluated,
                        "Maintenance scheduler: no due assignments"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    tick = tick_count,
                    error = %e,
                    "Maintenance scheduler: tick failed"
                );
            }
        }

        // Overdue detection: emit events for WOs past their scheduled date.
        match super::overdue::evaluate_overdue(&pool).await {
            Ok(r) if r.events_emitted > 0 => {
                tracing::info!(
                    tick = tick_count,
                    evaluated = r.evaluated,
                    emitted = r.events_emitted,
                    "Maintenance scheduler: overdue events emitted"
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!(
                    tick = tick_count,
                    error = %e,
                    "Maintenance scheduler: overdue detection failed"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_calendar_trigger() {
        assert_eq!(classify_trigger("calendar", true, false), "calendar");
    }

    #[test]
    fn classify_meter_trigger() {
        assert_eq!(classify_trigger("meter", false, true), "meter");
    }

    #[test]
    fn classify_both_date_only() {
        assert_eq!(classify_trigger("both", true, false), "calendar");
    }

    #[test]
    fn classify_both_meter_only() {
        assert_eq!(classify_trigger("both", false, true), "meter");
    }

    #[test]
    fn classify_both_triggers() {
        assert_eq!(classify_trigger("both", true, true), "both");
    }
}
