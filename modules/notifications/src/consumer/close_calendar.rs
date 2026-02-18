//! Close Calendar Reminder Emitter
//!
//! Evaluates close_calendar entries against the current date and emits
//! reminder notifications idempotently. Reads GL close state from
//! accounting_periods (read-only) to skip already-closed periods.
//!
//! Idempotency: Each reminder is keyed by (calendar_entry_id, reminder_key).
//! The reminder_key encodes the type and trigger date so the same reminder
//! is never emitted twice.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// A pending reminder action computed by the evaluator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderAction {
    pub calendar_entry_id: Uuid,
    pub tenant_id: String,
    pub period_id: Uuid,
    pub owner_role: String,
    pub reminder_type: ReminderType,
    pub reminder_key: String,
    pub expected_close_date: NaiveDate,
    pub days_offset: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReminderType {
    Upcoming,
    Overdue,
}

impl ReminderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReminderType::Upcoming => "upcoming",
            ReminderType::Overdue => "overdue",
        }
    }
}

/// Row from the calendar + period join for evaluation
#[derive(Debug, sqlx::FromRow)]
struct CalendarRow {
    id: Uuid,
    tenant_id: String,
    period_id: Uuid,
    expected_close_date: NaiveDate,
    owner_role: String,
    reminder_offset_days: Vec<i32>,
    overdue_reminder_interval_days: i32,
    // closed_at is filtered in SQL (WHERE closed_at IS NULL) so not needed on struct
}

/// Evaluate which reminders are due for all tenants.
///
/// Joins close_calendar with accounting_periods to get GL close state,
/// then computes which reminders should fire based on the current date.
/// Returns only reminders that haven't been sent yet (idempotent).
pub async fn evaluate_due_reminders(
    gl_pool: &PgPool,
    now: NaiveDate,
) -> Result<Vec<ReminderAction>, Box<dyn std::error::Error>> {
    let rows = sqlx::query_as::<_, CalendarRow>(
        r#"
        SELECT cc.id, cc.tenant_id, cc.period_id,
               cc.expected_close_date, cc.owner_role,
               cc.reminder_offset_days, cc.overdue_reminder_interval_days
        FROM close_calendar cc
        JOIN accounting_periods ap ON ap.id = cc.period_id AND ap.tenant_id = cc.tenant_id
        WHERE ap.closed_at IS NULL
        ORDER BY cc.expected_close_date ASC
        "#,
    )
    .fetch_all(gl_pool)
    .await?;

    let mut actions = Vec::new();

    for row in &rows {
        // Upcoming reminders: fire on (expected_close_date - offset_days)
        for &offset in &row.reminder_offset_days {
            let trigger_date = row.expected_close_date - chrono::Duration::days(offset as i64);
            if now >= trigger_date && now < row.expected_close_date {
                let key = format!("upcoming:{}d:{}", offset, row.expected_close_date);
                actions.push(ReminderAction {
                    calendar_entry_id: row.id,
                    tenant_id: row.tenant_id.clone(),
                    period_id: row.period_id,
                    owner_role: row.owner_role.clone(),
                    reminder_type: ReminderType::Upcoming,
                    reminder_key: key,
                    expected_close_date: row.expected_close_date,
                    days_offset: offset,
                });
            }
        }

        // Overdue reminders: fire every N days after expected_close_date
        if now >= row.expected_close_date {
            let days_overdue = (now - row.expected_close_date).num_days();
            let interval = row.overdue_reminder_interval_days.max(1) as i64;
            // Emit one reminder per interval that has elapsed
            let mut day = 0i64;
            while day <= days_overdue {
                let key = format!("overdue:day{}:{}", day, row.expected_close_date);
                actions.push(ReminderAction {
                    calendar_entry_id: row.id,
                    tenant_id: row.tenant_id.clone(),
                    period_id: row.period_id,
                    owner_role: row.owner_role.clone(),
                    reminder_type: ReminderType::Overdue,
                    reminder_key: key,
                    expected_close_date: row.expected_close_date,
                    days_offset: -(day as i32),
                });
                day += interval;
            }
        }
    }

    // Filter out already-sent reminders
    filter_unsent(gl_pool, actions).await
}

/// Filter out reminders that have already been sent (idempotency check)
async fn filter_unsent(
    gl_pool: &PgPool,
    actions: Vec<ReminderAction>,
) -> Result<Vec<ReminderAction>, Box<dyn std::error::Error>> {
    if actions.is_empty() {
        return Ok(actions);
    }

    let mut unsent = Vec::new();

    for action in actions {
        let exists: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT 1 FROM close_calendar_reminders_sent
            WHERE calendar_entry_id = $1 AND reminder_key = $2
            "#,
        )
        .bind(action.calendar_entry_id)
        .bind(&action.reminder_key)
        .fetch_optional(gl_pool)
        .await?;

        if exists.is_none() {
            unsent.push(action);
        }
    }

    Ok(unsent)
}

/// Mark a reminder as sent in the idempotency table.
///
/// Uses ON CONFLICT DO NOTHING so concurrent calls are safe.
pub async fn mark_reminder_sent(
    gl_pool: &PgPool,
    action: &ReminderAction,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO close_calendar_reminders_sent
            (tenant_id, calendar_entry_id, reminder_type, reminder_key)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (tenant_id, calendar_entry_id, reminder_key) DO NOTHING
        "#,
    )
    .bind(&action.tenant_id)
    .bind(action.calendar_entry_id)
    .bind(action.reminder_type.as_str())
    .bind(&action.reminder_key)
    .execute(gl_pool)
    .await?;

    Ok(())
}

/// Payload emitted as a notification event for close calendar reminders
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseCalendarReminderPayload {
    pub calendar_entry_id: String,
    pub tenant_id: String,
    pub period_id: String,
    pub owner_role: String,
    pub reminder_type: String,
    pub expected_close_date: String,
    pub days_offset: i32,
    pub message: String,
}

/// Build a notification payload from a reminder action
pub fn build_reminder_payload(action: &ReminderAction) -> CloseCalendarReminderPayload {
    let message = match action.reminder_type {
        ReminderType::Upcoming => format!(
            "Period close due in {} day(s) on {}. Owner: {}",
            action.days_offset, action.expected_close_date, action.owner_role
        ),
        ReminderType::Overdue => format!(
            "Period close is overdue (expected {}). Owner: {}",
            action.expected_close_date, action.owner_role
        ),
    };

    CloseCalendarReminderPayload {
        calendar_entry_id: action.calendar_entry_id.to_string(),
        tenant_id: action.tenant_id.clone(),
        period_id: action.period_id.to_string(),
        owner_role: action.owner_role.clone(),
        reminder_type: action.reminder_type.as_str().to_string(),
        expected_close_date: action.expected_close_date.to_string(),
        days_offset: action.days_offset,
        message,
    }
}

/// Emit a single reminder notification via the notifications outbox.
///
/// Enqueues a `notifications.close_calendar.reminder` event and marks
/// the reminder as sent in the GL idempotency table. Both operations
/// are separate — the outbox insert is in the notifications DB and the
/// idempotency record is in the GL DB.
pub async fn emit_reminder(
    notif_pool: &PgPool,
    gl_pool: &PgPool,
    action: &ReminderAction,
) -> Result<(), Box<dyn std::error::Error>> {
    let payload = build_reminder_payload(action);
    let event_id = Uuid::new_v4();
    let subject = "notifications.close_calendar.reminder";

    let envelope = crate::event_bus::create_notifications_envelope(
        event_id,
        action.tenant_id.clone(),
        subject.to_string(),
        None,
        None,
        "SIDE_EFFECT".to_string(),
        serde_json::to_value(&payload)?,
    );

    let mut tx = notif_pool.begin().await?;
    crate::event_bus::enqueue_event(&mut tx, subject, &envelope).await?;
    tx.commit().await?;

    mark_reminder_sent(gl_pool, action).await?;

    tracing::info!(
        event_id = %event_id,
        tenant_id = %action.tenant_id,
        period_id = %action.period_id,
        reminder_type = %action.reminder_type.as_str(),
        reminder_key = %action.reminder_key,
        "Close calendar reminder emitted"
    );

    Ok(())
}

/// Top-level tick: evaluate all due reminders and emit notifications.
///
/// Call this periodically (e.g. every minute or every hour) to process
/// the close calendar. Idempotent — safe to call at any frequency.
pub async fn tick(
    notif_pool: &PgPool,
    gl_pool: &PgPool,
    now: NaiveDate,
) -> Result<usize, Box<dyn std::error::Error>> {
    let actions = evaluate_due_reminders(gl_pool, now).await?;
    let count = actions.len();

    for action in &actions {
        if let Err(e) = emit_reminder(notif_pool, gl_pool, action).await {
            tracing::error!(
                tenant_id = %action.tenant_id,
                reminder_key = %action.reminder_key,
                error = %e,
                "Failed to emit close calendar reminder"
            );
        }
    }

    if count > 0 {
        tracing::info!(count, "Close calendar reminders emitted");
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_upcoming_reminder_payload() {
        let action = ReminderAction {
            calendar_entry_id: Uuid::new_v4(),
            tenant_id: "t-1".to_string(),
            period_id: Uuid::new_v4(),
            owner_role: "controller".to_string(),
            reminder_type: ReminderType::Upcoming,
            reminder_key: "upcoming:7d:2026-03-01".to_string(),
            expected_close_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            days_offset: 7,
        };

        let payload = build_reminder_payload(&action);
        assert_eq!(payload.reminder_type, "upcoming");
        assert!(payload.message.contains("7 day(s)"));
        assert!(payload.message.contains("controller"));
    }

    #[test]
    fn test_build_overdue_reminder_payload() {
        let action = ReminderAction {
            calendar_entry_id: Uuid::new_v4(),
            tenant_id: "t-1".to_string(),
            period_id: Uuid::new_v4(),
            owner_role: "accounting_manager".to_string(),
            reminder_type: ReminderType::Overdue,
            reminder_key: "overdue:day1:2026-02-28".to_string(),
            expected_close_date: NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
            days_offset: -1,
        };

        let payload = build_reminder_payload(&action);
        assert_eq!(payload.reminder_type, "overdue");
        assert!(payload.message.contains("overdue"));
        assert!(payload.message.contains("2026-02-28"));
    }

    #[test]
    fn test_reminder_type_as_str() {
        assert_eq!(ReminderType::Upcoming.as_str(), "upcoming");
        assert_eq!(ReminderType::Overdue.as_str(), "overdue");
    }
}
