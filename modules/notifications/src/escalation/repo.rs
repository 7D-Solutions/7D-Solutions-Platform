use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::event_bus::{create_notifications_envelope, enqueue_event};
use super::models::{EscalationRule, InsertEscalationRule};

/// Result from one poll-and-escalate cycle.
#[derive(Debug, Default)]
pub struct EscalationCycleResult {
    /// Number of notifications evaluated.
    pub evaluated: usize,
    /// Number of escalation sends created.
    pub escalated: usize,
    /// Number of notifications skipped (already acknowledged or already escalated at this level).
    pub skipped: usize,
}

/// Insert a new escalation rule. Returns the generated id.
pub async fn create_escalation_rule(
    pool: &PgPool,
    rule: &InsertEscalationRule,
) -> Result<Uuid, sqlx::Error> {
    let row = sqlx::query_as::<_, (Uuid,)>(
        r#"
        INSERT INTO escalation_rules
            (tenant_id, source_notification_type, level, timeout_secs,
             target_channel, target_recipient, priority)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(&rule.tenant_id)
    .bind(&rule.source_notification_type)
    .bind(rule.level)
    .bind(rule.timeout_secs)
    .bind(&rule.target_channel)
    .bind(&rule.target_recipient)
    .bind(&rule.priority)
    .fetch_one(pool)
    .await?;

    Ok(row.0)
}

/// Fetch escalation rules for a given tenant + notification type, ordered by level.
pub async fn get_rules_for_type(
    pool: &PgPool,
    tenant_id: &str,
    source_notification_type: &str,
) -> Result<Vec<EscalationRule>, sqlx::Error> {
    sqlx::query_as::<_, EscalationRule>(
        r#"
        SELECT id, tenant_id, source_notification_type, level, timeout_secs,
               target_channel, target_recipient, priority, created_at
        FROM escalation_rules
        WHERE tenant_id = $1 AND source_notification_type = $2
        ORDER BY level ASC
        "#,
    )
    .bind(tenant_id)
    .bind(source_notification_type)
    .fetch_all(pool)
    .await
}

/// Mark a notification as acknowledged. Prevents future escalations.
pub async fn acknowledge_notification(
    pool: &PgPool,
    notification_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE scheduled_notifications
        SET acknowledged_at = NOW()
        WHERE id = $1 AND acknowledged_at IS NULL
        "#,
    )
    .bind(notification_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Poll for unacknowledged, sent notifications past their escalation timeout
/// and create escalation sends.
///
/// This is the core escalation execution loop. It:
/// 1. Finds sent notifications that are unacknowledged.
/// 2. For each, looks up matching escalation rules by tenant + template_key.
/// 3. For each rule whose timeout has expired, creates an escalation send
///    (idempotently — unique constraint prevents duplicates).
/// 4. Enqueues an outbox event for each escalation send.
///
/// All mutations use Guard → Mutation → Outbox in a single transaction.
pub async fn poll_and_escalate(pool: &PgPool) -> Result<EscalationCycleResult, sqlx::Error> {
    let mut result = EscalationCycleResult::default();
    let now = Utc::now();

    // Step 1: Find sent, unacknowledged notifications that have escalation rules.
    // We join against escalation_rules to only consider notifications that have
    // at least one rule defined.
    let candidates = sqlx::query_as::<_, CandidateNotification>(
        r#"
        SELECT DISTINCT
            sn.id AS notification_id,
            sn.tenant_id,
            sn.template_key,
            sn.sent_at,
            sn.acknowledged_at
        FROM scheduled_notifications sn
        INNER JOIN escalation_rules er
            ON er.tenant_id = sn.tenant_id
            AND er.source_notification_type = sn.template_key
        WHERE sn.status = 'sent'
          AND sn.acknowledged_at IS NULL
          AND sn.sent_at IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    result.evaluated = candidates.len();

    for candidate in &candidates {
        let sent_at = match candidate.sent_at {
            Some(t) => t,
            None => continue,
        };

        // Fetch rules for this notification type, ordered by level.
        let rules = get_rules_for_type(pool, &candidate.tenant_id, &candidate.template_key).await?;

        for rule in &rules {
            let elapsed_secs = (now - sent_at).num_seconds();
            if elapsed_secs < rule.timeout_secs as i64 {
                // Timeout not yet reached for this level — skip higher levels too.
                break;
            }

            // Guard: check if this escalation has already been sent (idempotency).
            let already_sent = sqlx::query_as::<_, (Uuid,)>(
                r#"
                SELECT id FROM escalation_sends
                WHERE source_notification_id = $1 AND escalation_rule_id = $2 AND tenant_id = $3
                "#,
            )
            .bind(candidate.notification_id)
            .bind(rule.id)
            .bind(&candidate.tenant_id)
            .fetch_optional(pool)
            .await?;

            if already_sent.is_some() {
                result.skipped += 1;
                continue;
            }

            // Guard → Mutation → Outbox in a single transaction.
            let mut tx = pool.begin().await?;

            // Re-check idempotency inside the transaction (race-safe).
            let dup_check = sqlx::query_as::<_, (Uuid,)>(
                r#"
                SELECT id FROM escalation_sends
                WHERE source_notification_id = $1 AND escalation_rule_id = $2 AND tenant_id = $3
                FOR UPDATE
                "#,
            )
            .bind(candidate.notification_id)
            .bind(rule.id)
            .bind(&candidate.tenant_id)
            .fetch_optional(&mut *tx)
            .await?;

            if dup_check.is_some() {
                tx.commit().await?;
                result.skipped += 1;
                continue;
            }

            // Also re-check that the notification is still unacknowledged.
            let still_unacked = sqlx::query_as::<_, (Uuid,)>(
                r#"
                SELECT id FROM scheduled_notifications
                WHERE id = $1 AND acknowledged_at IS NULL
                FOR UPDATE
                "#,
            )
            .bind(candidate.notification_id)
            .fetch_optional(&mut *tx)
            .await?;

            if still_unacked.is_none() {
                tx.commit().await?;
                result.skipped += 1;
                continue;
            }

            // Mutation: insert the escalation send.
            let send_id = sqlx::query_as::<_, (Uuid,)>(
                r#"
                INSERT INTO escalation_sends
                    (tenant_id, source_notification_id, escalation_rule_id,
                     level, target_channel, target_recipient)
                VALUES ($1, $2, $3, $4, $5, $6)
                RETURNING id
                "#,
            )
            .bind(&candidate.tenant_id)
            .bind(candidate.notification_id)
            .bind(rule.id)
            .bind(rule.level)
            .bind(&rule.target_channel)
            .bind(&rule.target_recipient)
            .fetch_one(&mut *tx)
            .await?;

            // Outbox: enqueue escalation event.
            let payload = serde_json::json!({
                "escalation_send_id": send_id.0,
                "source_notification_id": candidate.notification_id,
                "escalation_rule_id": rule.id,
                "level": rule.level,
                "target_channel": rule.target_channel,
                "target_recipient": rule.target_recipient,
                "priority": rule.priority,
                "tenant_id": candidate.tenant_id,
            });

            let envelope = create_notifications_envelope(
                Uuid::new_v4(),
                candidate.tenant_id.clone(),
                "notifications.escalation.fired".to_string(),
                None,
                Some(candidate.notification_id.to_string()),
                "SIDE_EFFECT".to_string(),
                payload,
            );
            enqueue_event(&mut tx, "notifications.escalation.fired", &envelope).await?;

            tx.commit().await?;
            result.escalated += 1;
        }
    }

    Ok(result)
}

/// Internal struct for the candidate query.
#[derive(Debug, sqlx::FromRow)]
struct CandidateNotification {
    notification_id: Uuid,
    tenant_id: String,
    template_key: String,
    sent_at: Option<chrono::DateTime<Utc>>,
    #[allow(dead_code)]
    acknowledged_at: Option<chrono::DateTime<Utc>>,
}
