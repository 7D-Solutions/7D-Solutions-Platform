/// Bridge between module outbox tables and the central audit log
///
/// Provides utilities to ensure every outbox event has a corresponding
/// audit record (linked via causation_id). Used by the audit oracle
/// test and can be called at module startup for backfill.

use crate::schema::{MutationClass, WriteAuditRequest};
use sqlx::PgPool;
use uuid::Uuid;

/// Metadata extracted from a module's outbox event
#[derive(Debug, Clone)]
pub struct OutboxEventMeta {
    pub event_id: Uuid,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
}

/// Result of an audit completeness check for one module
#[derive(Debug, Default)]
pub struct ModuleAuditResult {
    pub module: String,
    pub total_outbox_events: u64,
    pub covered: u64,
    pub gaps: Vec<OutboxEventMeta>,
    pub duplicates: Vec<(Uuid, i64)>,
}

impl ModuleAuditResult {
    pub fn is_clean(&self) -> bool {
        self.gaps.is_empty() && self.duplicates.is_empty()
    }
}

/// Query all outbox event metadata from a module's outbox table.
///
/// Handles the standard schema (event_id, event_type, aggregate_type, aggregate_id)
/// used by most modules.
pub async fn query_outbox_events(
    module_pool: &PgPool,
    table_name: &str,
) -> Result<Vec<OutboxEventMeta>, sqlx::Error> {
    let query = format!(
        "SELECT event_id, event_type, aggregate_type, aggregate_id FROM {} ORDER BY created_at",
        table_name
    );

    let rows = sqlx::query_as::<_, (Uuid, String, String, String)>(&query)
        .fetch_all(module_pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(event_id, event_type, aggregate_type, aggregate_id)| OutboxEventMeta {
            event_id,
            event_type,
            aggregate_type,
            aggregate_id,
        })
        .collect())
}

/// Query outbox events from the payments module (different schema — no aggregate columns).
pub async fn query_payments_outbox_events(
    module_pool: &PgPool,
) -> Result<Vec<OutboxEventMeta>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT event_id, event_type FROM payments_events_outbox ORDER BY created_at",
    )
    .fetch_all(module_pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(event_id, event_type)| OutboxEventMeta {
            event_id,
            event_type,
            aggregate_type: "PaymentAttempt".to_string(),
            aggregate_id: event_id.to_string(),
        })
        .collect())
}

/// Query outbox events from the subscriptions module.
///
/// Subscriptions outbox has event_id/event_type (Phase 16) but no aggregate columns.
/// Rows with NULL event_id (pre-Phase 16) are skipped.
pub async fn query_subscriptions_outbox_events(
    module_pool: &PgPool,
) -> Result<Vec<OutboxEventMeta>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (Uuid, Option<String>, String)>(
        "SELECT event_id, event_type, subject FROM events_outbox \
         WHERE event_id IS NOT NULL ORDER BY created_at",
    )
    .fetch_all(module_pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(event_id, event_type, subject)| {
            let etype = event_type.unwrap_or_else(|| subject.clone());
            OutboxEventMeta {
                event_id,
                event_type: etype,
                aggregate_type: "Subscription".to_string(),
                aggregate_id: event_id.to_string(),
            }
        })
        .collect())
}

/// Query outbox events from the notifications module.
///
/// Notifications outbox has event_id (base) and event_type (Phase 16, nullable)
/// but no aggregate columns.
pub async fn query_notifications_outbox_events(
    module_pool: &PgPool,
) -> Result<Vec<OutboxEventMeta>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (Uuid, Option<String>, String)>(
        "SELECT event_id, event_type, subject FROM events_outbox ORDER BY created_at",
    )
    .fetch_all(module_pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(event_id, event_type, subject)| {
            let etype = event_type.unwrap_or_else(|| subject.clone());
            OutboxEventMeta {
                event_id,
                event_type: etype,
                aggregate_type: "Notification".to_string(),
                aggregate_id: event_id.to_string(),
            }
        })
        .collect())
}

/// Check audit completeness for a module: for each outbox event, verify
/// exactly one audit record exists with causation_id = event_id.
///
/// Returns a result with gaps (missing) and duplicates (>1 record).
pub async fn check_module_audit_completeness(
    audit_pool: &PgPool,
    events: &[OutboxEventMeta],
    module_name: &str,
) -> Result<ModuleAuditResult, sqlx::Error> {
    let mut result = ModuleAuditResult {
        module: module_name.to_string(),
        total_outbox_events: events.len() as u64,
        ..Default::default()
    };

    for event in events {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::bigint FROM audit_events WHERE causation_id = $1",
        )
        .bind(event.event_id)
        .fetch_one(audit_pool)
        .await?;

        match count {
            0 => result.gaps.push(event.clone()),
            1 => result.covered += 1,
            n => {
                result.duplicates.push((event.event_id, n));
                result.covered += 1;
            }
        }
    }

    Ok(result)
}

/// Backfill missing audit records for outbox events that have no coverage.
///
/// For each gap, writes a single audit record with:
/// - causation_id = outbox event_id
/// - actor = System (backfill)
/// - mutation_class inferred from event_type
///
/// Returns the number of records written.
pub async fn backfill_missing_audit_records(
    audit_pool: &PgPool,
    gaps: &[OutboxEventMeta],
    module_name: &str,
) -> Result<u64, sqlx::Error> {
    let mut written = 0u64;

    for event in gaps {
        let mutation_class = classify_event_type(&event.event_type);

        let request = WriteAuditRequest::new(
            Uuid::nil(),
            "System".to_string(),
            event.event_type.clone(),
            mutation_class,
            event.aggregate_type.clone(),
            event.aggregate_id.clone(),
        )
        .with_correlation(Some(event.event_id), Some(Uuid::new_v4()), None)
        .with_metadata(serde_json::json!({
            "source": "outbox_bridge_backfill",
            "module": module_name,
        }));

        sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO audit_events (
                actor_id, actor_type, action, mutation_class,
                entity_type, entity_id,
                causation_id, correlation_id, trace_id,
                metadata
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING audit_id
            "#,
        )
        .bind(request.actor_id)
        .bind(&request.actor_type)
        .bind(&request.action)
        .bind(request.mutation_class)
        .bind(&request.entity_type)
        .bind(&request.entity_id)
        .bind(request.causation_id)
        .bind(request.correlation_id)
        .bind(&request.trace_id)
        .bind(&request.metadata)
        .fetch_one(audit_pool)
        .await?;

        written += 1;
    }

    Ok(written)
}

/// Classify an event type into a MutationClass.
///
/// Uses naming conventions across modules to infer the class.
pub fn classify_event_type(event_type: &str) -> MutationClass {
    let lower = event_type.to_lowercase();

    if lower.contains("reversed") || lower.contains("written_off") || lower.contains("voided") {
        MutationClass::Reversal
    } else if lower.contains("transition")
        || lower.contains("finaliz")
        || lower.contains("suspended")
        || lower.contains("status")
        || lower.contains("approved")
        || lower.contains("submitted")
    {
        MutationClass::StateTransition
    } else if lower.contains("updated") || lower.contains("revaluation") || lower.contains("aging")
    {
        MutationClass::Update
    } else if lower.contains("deleted") || lower.contains("removed") {
        MutationClass::Delete
    } else {
        MutationClass::Create
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_event_types() {
        assert_eq!(
            classify_event_type("ar.invoice.finalizing"),
            MutationClass::StateTransition
        );
        assert_eq!(
            classify_event_type("gl.events.entry.reversed"),
            MutationClass::Reversal
        );
        assert_eq!(
            classify_event_type("ar.credit_note_issued"),
            MutationClass::Create
        );
        assert_eq!(
            classify_event_type("fx.rate_updated"),
            MutationClass::Update
        );
        assert_eq!(
            classify_event_type("ar.invoice_written_off"),
            MutationClass::Reversal
        );
        assert_eq!(
            classify_event_type("inventory.item.status_transition"),
            MutationClass::StateTransition
        );
    }

    #[test]
    fn test_module_audit_result_clean() {
        let result = ModuleAuditResult {
            module: "test".to_string(),
            total_outbox_events: 5,
            covered: 5,
            gaps: vec![],
            duplicates: vec![],
        };
        assert!(result.is_clean());
    }

    #[test]
    fn test_module_audit_result_with_gaps() {
        let result = ModuleAuditResult {
            module: "test".to_string(),
            total_outbox_events: 5,
            covered: 3,
            gaps: vec![OutboxEventMeta {
                event_id: Uuid::new_v4(),
                event_type: "test.created".to_string(),
                aggregate_type: "Test".to_string(),
                aggregate_id: "t1".to_string(),
            }],
            duplicates: vec![],
        };
        assert!(!result.is_clean());
    }
}
