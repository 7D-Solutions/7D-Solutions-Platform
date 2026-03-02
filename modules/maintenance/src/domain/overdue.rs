//! Work order overdue detection — finds overdue WOs and emits events with dedup.
//!
//! Design:
//! - WOs with status in (scheduled, in_progress, on_hold) and scheduled_date < today → overdue.
//! - Emits `maintenance.work_order.overdue` with a deterministic event_id per
//!   (tenant_id, wo_id, overdue_date) so each WO only gets one event per calendar day.
//! - Dedup uses ON CONFLICT DO NOTHING on the events_outbox unique event_id constraint.
//! - Includes days_overdue in the payload for downstream severity escalation.

use chrono::{NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Namespace UUID for deterministic overdue event IDs (UUID v5).
const OVERDUE_NS: Uuid = Uuid::from_bytes([
    0x9a, 0x3b, 0x7c, 0x4d, 0xe5, 0xf6, 0x47, 0x8a, 0xb1, 0xc2, 0xd3, 0xe4, 0xf5, 0x06, 0x17, 0x28,
]);

/// Result of an overdue detection tick.
#[derive(Debug, Default)]
pub struct OverdueResult {
    pub evaluated: usize,
    pub events_emitted: usize,
}

/// An overdue work order row from the finder query.
#[derive(Debug, sqlx::FromRow)]
struct OverdueWo {
    id: Uuid,
    tenant_id: String,
    wo_number: String,
    asset_id: Uuid,
    priority: String,
    status: String,
    scheduled_date: NaiveDate,
}

/// Event payload for `maintenance.work_order.overdue`.
#[derive(Debug, Serialize)]
struct WoOverdueEvent {
    work_order_id: Uuid,
    tenant_id: String,
    wo_number: String,
    asset_id: Uuid,
    priority: String,
    status: String,
    scheduled_date: NaiveDate,
    overdue_date: NaiveDate,
    days_overdue: i64,
}

/// Build a deterministic event_id for an overdue event on a given day.
/// Same (tenant, wo, date) always produces the same UUID — outbox UNIQUE prevents dupes.
fn overdue_event_id(tenant_id: &str, wo_id: Uuid, overdue_date: NaiveDate) -> Uuid {
    let key = format!("{}:{}:{}", tenant_id, wo_id, overdue_date);
    Uuid::new_v5(&OVERDUE_NS, key.as_bytes())
}

/// Evaluate all work orders for overdue status and emit one event per WO per day.
///
/// Returns the number of overdue WOs evaluated and new events emitted.
pub async fn evaluate_overdue(pool: &PgPool) -> Result<OverdueResult, sqlx::Error> {
    let today = Utc::now().date_naive();
    let mut result = OverdueResult::default();

    let overdue_wos = sqlx::query_as::<_, OverdueWo>(
        r#"
        SELECT id, tenant_id, wo_number, asset_id,
               priority, status, scheduled_date
        FROM work_orders
        WHERE status IN ('scheduled', 'in_progress', 'on_hold')
          AND scheduled_date IS NOT NULL
          AND scheduled_date < $1
        ORDER BY tenant_id, scheduled_date
        "#,
    )
    .bind(today)
    .fetch_all(pool)
    .await?;

    result.evaluated = overdue_wos.len();

    for wo in &overdue_wos {
        let days_overdue = (today - wo.scheduled_date).num_days();
        let event_id = overdue_event_id(&wo.tenant_id, wo.id, today);

        let event = WoOverdueEvent {
            work_order_id: wo.id,
            tenant_id: wo.tenant_id.clone(),
            wo_number: wo.wo_number.clone(),
            asset_id: wo.asset_id,
            priority: wo.priority.clone(),
            status: wo.status.clone(),
            scheduled_date: wo.scheduled_date,
            overdue_date: today,
            days_overdue,
        };

        let env = crate::events::envelope::create_envelope(
            event_id,
            wo.tenant_id.clone(),
            crate::events::subjects::WO_OVERDUE.to_string(),
            event,
        );
        let env_json = crate::events::envelope::validate_envelope(&env).map_err(|e| {
            sqlx::Error::Encode(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Envelope validation: {}", e),
            )))
        })?;

        // INSERT with ON CONFLICT DO NOTHING — idempotent dedup per (wo, day).
        let res = sqlx::query(
            r#"
            INSERT INTO events_outbox
                (event_id, event_type, aggregate_type, aggregate_id, payload)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (event_id) DO NOTHING
            "#,
        )
        .bind(event_id)
        .bind(crate::events::subjects::WO_OVERDUE)
        .bind("work_order")
        .bind(wo.id.to_string())
        .bind(env_json)
        .execute(pool)
        .await?;

        if res.rows_affected() > 0 {
            result.events_emitted += 1;
            tracing::info!(
                work_order_id = %wo.id,
                wo_number = %wo.wo_number,
                tenant_id = %wo.tenant_id,
                days_overdue = days_overdue,
                "maintenance.work_order.overdue event emitted"
            );
        }
    }

    Ok(result)
}
