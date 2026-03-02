//! Integration tests for outbox_bridge audit event emission.
//!
//! Verifies that the bridge correctly classifies outbox events, checks
//! audit completeness, and backfills missing records against a real DB.

mod helpers;

use audit::{
    outbox_bridge::{
        backfill_missing_audit_records, check_module_audit_completeness, classify_event_type,
        OutboxEventMeta,
    },
    schema::{MutationClass, WriteAuditRequest},
    writer::AuditWriter,
};
use uuid::Uuid;

// ── classify_event_type tests ─────────────────────────────────────────

#[test]
fn classify_creates() {
    assert_eq!(
        classify_event_type("ar.credit_note_issued"),
        MutationClass::Create
    );
    assert_eq!(
        classify_event_type("inventory.item.created"),
        MutationClass::Create
    );
}

#[test]
fn classify_updates() {
    assert_eq!(
        classify_event_type("fx.rate_updated"),
        MutationClass::Update
    );
    assert_eq!(
        classify_event_type("asset.revaluation"),
        MutationClass::Update
    );
}

#[test]
fn classify_state_transitions() {
    assert_eq!(
        classify_event_type("ar.invoice.finalizing"),
        MutationClass::StateTransition
    );
    assert_eq!(
        classify_event_type("order.status_changed"),
        MutationClass::StateTransition
    );
    assert_eq!(
        classify_event_type("invoice.approved"),
        MutationClass::StateTransition
    );
}

#[test]
fn classify_reversals() {
    assert_eq!(
        classify_event_type("gl.events.entry.reversed"),
        MutationClass::Reversal
    );
    assert_eq!(
        classify_event_type("ar.invoice_written_off"),
        MutationClass::Reversal
    );
    assert_eq!(
        classify_event_type("payment.voided"),
        MutationClass::Reversal
    );
}

#[test]
fn classify_deletes() {
    assert_eq!(classify_event_type("item.deleted"), MutationClass::Delete);
    assert_eq!(classify_event_type("record.removed"), MutationClass::Delete);
}

// ── completeness check tests ──────────────────────────────────────────

#[tokio::test]
async fn completeness_reports_gaps_for_uncovered_events() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    // Create fake outbox events (no corresponding audit records)
    let events: Vec<OutboxEventMeta> = (0..3)
        .map(|i| OutboxEventMeta {
            event_id: Uuid::new_v4(),
            event_type: format!("test.event_{}", i),
            aggregate_type: "TestEntity".to_string(),
            aggregate_id: format!("te_{}", i),
        })
        .collect();

    let result = check_module_audit_completeness(&pool, &events, "test_module")
        .await
        .expect("completeness check failed");

    assert_eq!(result.module, "test_module");
    assert_eq!(result.total_outbox_events, 3);
    assert_eq!(result.covered, 0);
    assert_eq!(result.gaps.len(), 3);
    assert!(result.duplicates.is_empty());
    assert!(!result.is_clean());
}

#[tokio::test]
async fn completeness_reports_clean_when_all_covered() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());

    // Create outbox events and matching audit records
    let mut events = Vec::new();
    for i in 0..3 {
        let event_id = Uuid::new_v4();
        events.push(OutboxEventMeta {
            event_id,
            event_type: format!("covered.event_{}", i),
            aggregate_type: "CoveredEntity".to_string(),
            aggregate_id: format!("ce_{}", i),
        });

        let request = WriteAuditRequest::new(
            Uuid::nil(),
            "System".to_string(),
            format!("covered.event_{}", i),
            MutationClass::Create,
            "CoveredEntity".to_string(),
            format!("ce_{}", i),
        )
        .with_correlation(Some(event_id), None, None);

        writer.write(request).await.expect("write failed");
    }

    let result = check_module_audit_completeness(&pool, &events, "covered_module")
        .await
        .expect("completeness check failed");

    assert_eq!(result.total_outbox_events, 3);
    assert_eq!(result.covered, 3);
    assert!(result.gaps.is_empty());
    assert!(result.is_clean());
}

// ── backfill tests ────────────────────────────────────────────────────

#[tokio::test]
async fn backfill_creates_missing_audit_records() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    // Gaps to backfill
    let gaps: Vec<OutboxEventMeta> = (0..2)
        .map(|i| OutboxEventMeta {
            event_id: Uuid::new_v4(),
            event_type: format!("backfill.created_{}", i),
            aggregate_type: "BackfillEntity".to_string(),
            aggregate_id: format!("bf_{}", Uuid::new_v4()),
        })
        .collect();

    let written = backfill_missing_audit_records(&pool, &gaps, "backfill_module")
        .await
        .expect("backfill failed");

    assert_eq!(written, 2);

    // Verify the backfilled records exist via causation_id
    for gap in &gaps {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*)::bigint FROM audit_events WHERE causation_id = $1")
                .bind(gap.event_id)
                .fetch_one(&pool)
                .await
                .expect("count query failed");

        assert_eq!(count, 1, "backfilled record missing for {:?}", gap.event_id);
    }
}

#[tokio::test]
async fn backfill_records_have_correct_metadata() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    let event_id = Uuid::new_v4();
    let gap = OutboxEventMeta {
        event_id,
        event_type: "order.status_transition".to_string(),
        aggregate_type: "Order".to_string(),
        aggregate_id: format!("ord_{}", Uuid::new_v4()),
    };

    backfill_missing_audit_records(&pool, &[gap.clone()], "orders_module")
        .await
        .expect("backfill failed");

    let row = sqlx::query_as::<_, audit::schema::AuditEvent>(
        "SELECT audit_id, occurred_at, actor_id, actor_type, action, mutation_class, \
         entity_type, entity_id, before_snapshot, after_snapshot, before_hash, after_hash, \
         causation_id, correlation_id, trace_id, metadata \
         FROM audit_events WHERE causation_id = $1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("fetch failed");

    assert_eq!(row.actor_id, Uuid::nil());
    assert_eq!(row.actor_type, "System");
    assert_eq!(row.action, "order.status_transition");
    assert_eq!(row.mutation_class, MutationClass::StateTransition);
    assert_eq!(row.entity_type, "Order");
    assert_eq!(row.causation_id, Some(event_id));

    let meta = row.metadata.expect("metadata should exist");
    assert_eq!(meta["source"], "outbox_bridge_backfill");
    assert_eq!(meta["module"], "orders_module");
}
