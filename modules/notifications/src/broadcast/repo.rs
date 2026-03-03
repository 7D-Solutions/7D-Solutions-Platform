use sqlx::PgPool;
use uuid::Uuid;

use super::models::{Broadcast, BroadcastRecipient, BroadcastResult, CreateBroadcast};
use crate::event_bus::{create_notifications_envelope, enqueue_event};

/// Create a broadcast and fan out to individual recipients.
///
/// Uses Guard → Mutation → Outbox atomicity in a single transaction:
/// 1. Guard: check idempotency_key — if duplicate, return existing broadcast.
/// 2. Mutation: insert broadcast + fan-out recipient records.
/// 3. Outbox: enqueue broadcast.created + individual delivery events.
///
/// The `user_ids` slice represents the resolved audience — the caller is
/// responsible for resolving "all_tenant" or "role" to concrete user IDs.
pub async fn create_broadcast_and_fan_out(
    pool: &PgPool,
    req: &CreateBroadcast,
    user_ids: &[String],
) -> Result<BroadcastResult, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // ── Guard: idempotency check ──────────────────────────────────────
    let existing = sqlx::query_as::<_, Broadcast>(
        r#"
        SELECT id, tenant_id, idempotency_key, audience_type, audience_filter,
               title, body, channel, status, recipient_count, created_at, updated_at
        FROM broadcasts
        WHERE tenant_id = $1 AND idempotency_key = $2
        FOR UPDATE
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(broadcast) = existing {
        tx.commit().await?;
        return Ok(BroadcastResult {
            broadcast,
            recipients_created: 0,
            was_duplicate: true,
        });
    }

    // ── Mutation: insert broadcast ────────────────────────────────────
    let broadcast = sqlx::query_as::<_, Broadcast>(
        r#"
        INSERT INTO broadcasts
            (tenant_id, idempotency_key, audience_type, audience_filter,
             title, body, channel, status, recipient_count)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'fan_out_complete', $8)
        RETURNING id, tenant_id, idempotency_key, audience_type, audience_filter,
                  title, body, channel, status, recipient_count, created_at, updated_at
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(req.audience_type.as_str())
    .bind(&req.audience_filter)
    .bind(&req.title)
    .bind(&req.body)
    .bind(&req.channel)
    .bind(user_ids.len() as i32)
    .fetch_one(&mut *tx)
    .await?;

    // ── Mutation: fan-out recipient records ────────────────────────────
    let mut recipients_created: usize = 0;
    for user_id in user_ids {
        let inserted = sqlx::query_as::<_, (Uuid,)>(
            r#"
            INSERT INTO broadcast_recipients (broadcast_id, tenant_id, user_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (broadcast_id, user_id) DO NOTHING
            RETURNING id
            "#,
        )
        .bind(broadcast.id)
        .bind(&req.tenant_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;

        if inserted.is_some() {
            recipients_created += 1;
        }
    }

    // ── Outbox: broadcast.created event ───────────────────────────────
    let broadcast_envelope = create_notifications_envelope(
        Uuid::new_v4(),
        req.tenant_id.clone(),
        "notifications.broadcast.created".to_string(),
        None,
        None,
        "LIFECYCLE".to_string(),
        serde_json::json!({
            "broadcast_id": broadcast.id,
            "audience_type": req.audience_type.as_str(),
            "audience_filter": req.audience_filter,
            "title": req.title,
            "recipient_count": recipients_created,
        }),
    );
    enqueue_event(
        &mut tx,
        "notifications.events.broadcast.created",
        &broadcast_envelope,
    )
    .await?;

    // ── Outbox: individual delivery events ────────────────────────────
    for user_id in user_ids {
        let delivery_envelope = create_notifications_envelope(
            Uuid::new_v4(),
            req.tenant_id.clone(),
            "notifications.broadcast.delivered".to_string(),
            Some(broadcast.id.to_string()),
            None,
            "SIDE_EFFECT".to_string(),
            serde_json::json!({
                "broadcast_id": broadcast.id,
                "user_id": user_id,
                "title": req.title,
                "channel": req.channel,
            }),
        );
        enqueue_event(
            &mut tx,
            "notifications.events.broadcast.delivered",
            &delivery_envelope,
        )
        .await?;
    }

    tx.commit().await?;

    Ok(BroadcastResult {
        broadcast,
        recipients_created,
        was_duplicate: false,
    })
}

/// Get a broadcast by ID, scoped to tenant.
pub async fn get_broadcast(
    pool: &PgPool,
    tenant_id: &str,
    broadcast_id: Uuid,
) -> Result<Option<Broadcast>, sqlx::Error> {
    sqlx::query_as::<_, Broadcast>(
        r#"
        SELECT id, tenant_id, idempotency_key, audience_type, audience_filter,
               title, body, channel, status, recipient_count, created_at, updated_at
        FROM broadcasts
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(broadcast_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

/// List recipients for a broadcast, scoped to tenant.
pub async fn list_recipients(
    pool: &PgPool,
    tenant_id: &str,
    broadcast_id: Uuid,
) -> Result<Vec<BroadcastRecipient>, sqlx::Error> {
    sqlx::query_as::<_, BroadcastRecipient>(
        r#"
        SELECT id, broadcast_id, tenant_id, user_id, created_at
        FROM broadcast_recipients
        WHERE broadcast_id = $1 AND tenant_id = $2
        ORDER BY created_at ASC
        "#,
    )
    .bind(broadcast_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

/// List broadcasts for a tenant with pagination.
pub async fn list_broadcasts(
    pool: &PgPool,
    tenant_id: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<Broadcast>, i64), sqlx::Error> {
    let (total,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM broadcasts WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    let rows = sqlx::query_as::<_, Broadcast>(
        r#"
        SELECT id, tenant_id, idempotency_key, audience_type, audience_filter,
               title, body, channel, status, recipient_count, created_at, updated_at
        FROM broadcasts
        WHERE tenant_id = $1
        ORDER BY created_at DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(tenant_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok((rows, total))
}
