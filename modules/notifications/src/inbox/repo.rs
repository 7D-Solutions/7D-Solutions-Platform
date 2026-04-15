use sqlx::PgPool;
use uuid::Uuid;

use super::models::InboxMessage;
use crate::event_bus::{create_notifications_envelope, enqueue_event};

/// Parameters for listing inbox messages.
#[derive(Debug, Clone)]
pub struct InboxListParams {
    pub tenant_id: String,
    pub user_id: String,
    pub unread_only: bool,
    pub include_dismissed: bool,
    pub category: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

/// Idempotent insert of an inbox message for a specific user + notification.
///
/// Returns `Some(message)` if a new row was created, or `None` if the
/// dedupe constraint fired (same notification_id + user_id already exists).
///
/// Uses Guard → Mutation → Outbox atomicity: the insert and outbox event
/// are committed in a single transaction.
pub async fn create_inbox_message(
    pool: &PgPool,
    tenant_id: &str,
    user_id: &str,
    notification_id: Uuid,
    title: &str,
    body: Option<&str>,
    category: Option<&str>,
) -> Result<Option<InboxMessage>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Idempotent insert: ON CONFLICT DO NOTHING returns zero rows when a
    // duplicate (notification_id, user_id) already exists.
    let row = sqlx::query_as::<_, InboxMessage>(
        r#"
        INSERT INTO inbox_messages
            (tenant_id, user_id, notification_id, title, body, category)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (notification_id, user_id) DO NOTHING
        RETURNING
            id, tenant_id, user_id, notification_id, title, body, category,
            is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(notification_id)
    .bind(title)
    .bind(body)
    .bind(category)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(ref msg) = row {
        let envelope = create_notifications_envelope(
            Uuid::new_v4(),
            tenant_id.to_string(),
            "notifications.inbox.message_created".to_string(),
            None,
            None,
            "LIFECYCLE".to_string(),
            serde_json::json!({
                "inbox_message_id": msg.id,
                "user_id": user_id,
                "notification_id": notification_id,
                "title": title,
            }),
        );
        enqueue_event(
            &mut tx,
            "notifications.events.inbox.message_created",
            &envelope,
        )
        .await?;
    }

    tx.commit().await?;
    Ok(row)
}

/// List inbox messages for a user with pagination and optional filters.
pub async fn list_messages(
    pool: &PgPool,
    params: &InboxListParams,
) -> Result<(Vec<InboxMessage>, i64), sqlx::Error> {
    let templates = inbox_list_templates(params);

    // Count query
    let mut count_q = sqlx::query_as::<_, (i64,)>(templates.count_sql)
        .bind(&params.tenant_id)
        .bind(&params.user_id);
    if templates.bind_category {
        let cat = params
            .category
            .as_ref()
            .expect("category template requires category value");
        count_q = count_q.bind(cat);
    }
    let (total,) = count_q.fetch_one(pool).await?;

    // Data query
    let mut data_q = sqlx::query_as::<_, InboxMessage>(templates.data_sql)
        .bind(&params.tenant_id)
        .bind(&params.user_id);
    if templates.bind_category {
        let cat = params
            .category
            .as_ref()
            .expect("category template requires category value");
        data_q = data_q.bind(cat);
    }
    data_q = data_q.bind(params.limit).bind(params.offset);

    let rows = data_q.fetch_all(pool).await?;
    Ok((rows, total))
}

struct InboxListTemplates {
    count_sql: &'static str,
    data_sql: &'static str,
    bind_category: bool,
}

fn inbox_list_templates(params: &InboxListParams) -> InboxListTemplates {
    match (
        params.unread_only,
        params.include_dismissed,
        params.category.is_some(),
    ) {
        (false, false, false) => InboxListTemplates {
            count_sql: r#"
                SELECT COUNT(*)
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_dismissed = FALSE
            "#,
            data_sql: r#"
                SELECT id, tenant_id, user_id, notification_id, title, body, category,
                       is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_dismissed = FALSE
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
            "#,
            bind_category: false,
        },
        (true, false, false) => InboxListTemplates {
            count_sql: r#"
                SELECT COUNT(*)
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_read = FALSE
                  AND is_dismissed = FALSE
            "#,
            data_sql: r#"
                SELECT id, tenant_id, user_id, notification_id, title, body, category,
                       is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_read = FALSE
                  AND is_dismissed = FALSE
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
            "#,
            bind_category: false,
        },
        (false, true, false) => InboxListTemplates {
            count_sql: r#"
                SELECT COUNT(*)
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
            "#,
            data_sql: r#"
                SELECT id, tenant_id, user_id, notification_id, title, body, category,
                       is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
            "#,
            bind_category: false,
        },
        (true, true, false) => InboxListTemplates {
            count_sql: r#"
                SELECT COUNT(*)
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_read = FALSE
            "#,
            data_sql: r#"
                SELECT id, tenant_id, user_id, notification_id, title, body, category,
                       is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_read = FALSE
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
            "#,
            bind_category: false,
        },
        (false, false, true) => InboxListTemplates {
            count_sql: r#"
                SELECT COUNT(*)
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_dismissed = FALSE
                  AND category = $3
            "#,
            data_sql: r#"
                SELECT id, tenant_id, user_id, notification_id, title, body, category,
                       is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_dismissed = FALSE
                  AND category = $3
                ORDER BY created_at DESC
                LIMIT $4 OFFSET $5
            "#,
            bind_category: true,
        },
        (true, false, true) => InboxListTemplates {
            count_sql: r#"
                SELECT COUNT(*)
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_read = FALSE
                  AND is_dismissed = FALSE
                  AND category = $3
            "#,
            data_sql: r#"
                SELECT id, tenant_id, user_id, notification_id, title, body, category,
                       is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_read = FALSE
                  AND is_dismissed = FALSE
                  AND category = $3
                ORDER BY created_at DESC
                LIMIT $4 OFFSET $5
            "#,
            bind_category: true,
        },
        (false, true, true) => InboxListTemplates {
            count_sql: r#"
                SELECT COUNT(*)
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND category = $3
            "#,
            data_sql: r#"
                SELECT id, tenant_id, user_id, notification_id, title, body, category,
                       is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND category = $3
                ORDER BY created_at DESC
                LIMIT $4 OFFSET $5
            "#,
            bind_category: true,
        },
        (true, true, true) => InboxListTemplates {
            count_sql: r#"
                SELECT COUNT(*)
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_read = FALSE
                  AND category = $3
            "#,
            data_sql: r#"
                SELECT id, tenant_id, user_id, notification_id, title, body, category,
                       is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
                FROM inbox_messages
                WHERE tenant_id = $1
                  AND user_id = $2
                  AND is_read = FALSE
                  AND category = $3
                ORDER BY created_at DESC
                LIMIT $4 OFFSET $5
            "#,
            bind_category: true,
        },
    }
}

/// Fetch a single inbox message by id, scoped to tenant + user.
pub async fn get_message(
    pool: &PgPool,
    tenant_id: &str,
    user_id: &str,
    message_id: Uuid,
) -> Result<Option<InboxMessage>, sqlx::Error> {
    sqlx::query_as::<_, InboxMessage>(
        r#"
        SELECT id, tenant_id, user_id, notification_id, title, body, category,
               is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
        FROM inbox_messages
        WHERE id = $1 AND tenant_id = $2 AND user_id = $3
        "#,
    )
    .bind(message_id)
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Mark a message as read. Guard → Mutation → Outbox.
pub async fn mark_read(
    pool: &PgPool,
    tenant_id: &str,
    user_id: &str,
    message_id: Uuid,
) -> Result<Option<InboxMessage>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let msg = sqlx::query_as::<_, InboxMessage>(
        r#"
        UPDATE inbox_messages
        SET is_read = TRUE, read_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2 AND user_id = $3
        RETURNING id, tenant_id, user_id, notification_id, title, body, category,
                  is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
        "#,
    )
    .bind(message_id)
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(ref m) = msg {
        let envelope = create_notifications_envelope(
            Uuid::new_v4(),
            tenant_id.to_string(),
            "notifications.inbox.message_read".to_string(),
            None,
            None,
            "LIFECYCLE".to_string(),
            serde_json::json!({
                "inbox_message_id": m.id,
                "user_id": user_id,
            }),
        );
        enqueue_event(
            &mut tx,
            "notifications.events.inbox.message_read",
            &envelope,
        )
        .await?;
    }

    tx.commit().await?;
    Ok(msg)
}

/// Mark a message as unread. Guard → Mutation → Outbox.
pub async fn mark_unread(
    pool: &PgPool,
    tenant_id: &str,
    user_id: &str,
    message_id: Uuid,
) -> Result<Option<InboxMessage>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let msg = sqlx::query_as::<_, InboxMessage>(
        r#"
        UPDATE inbox_messages
        SET is_read = FALSE, read_at = NULL, updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2 AND user_id = $3
        RETURNING id, tenant_id, user_id, notification_id, title, body, category,
                  is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
        "#,
    )
    .bind(message_id)
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(ref m) = msg {
        let envelope = create_notifications_envelope(
            Uuid::new_v4(),
            tenant_id.to_string(),
            "notifications.inbox.message_unread".to_string(),
            None,
            None,
            "LIFECYCLE".to_string(),
            serde_json::json!({
                "inbox_message_id": m.id,
                "user_id": user_id,
            }),
        );
        enqueue_event(
            &mut tx,
            "notifications.events.inbox.message_unread",
            &envelope,
        )
        .await?;
    }

    tx.commit().await?;
    Ok(msg)
}

/// Dismiss a message. Guard → Mutation → Outbox.
pub async fn dismiss_message(
    pool: &PgPool,
    tenant_id: &str,
    user_id: &str,
    message_id: Uuid,
) -> Result<Option<InboxMessage>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let msg = sqlx::query_as::<_, InboxMessage>(
        r#"
        UPDATE inbox_messages
        SET is_dismissed = TRUE, dismissed_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2 AND user_id = $3
        RETURNING id, tenant_id, user_id, notification_id, title, body, category,
                  is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
        "#,
    )
    .bind(message_id)
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(ref m) = msg {
        let envelope = create_notifications_envelope(
            Uuid::new_v4(),
            tenant_id.to_string(),
            "notifications.inbox.message_dismissed".to_string(),
            None,
            None,
            "LIFECYCLE".to_string(),
            serde_json::json!({
                "inbox_message_id": m.id,
                "user_id": user_id,
            }),
        );
        enqueue_event(
            &mut tx,
            "notifications.events.inbox.message_dismissed",
            &envelope,
        )
        .await?;
    }

    tx.commit().await?;
    Ok(msg)
}

/// Undismiss a message. Guard → Mutation → Outbox.
pub async fn undismiss_message(
    pool: &PgPool,
    tenant_id: &str,
    user_id: &str,
    message_id: Uuid,
) -> Result<Option<InboxMessage>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let msg = sqlx::query_as::<_, InboxMessage>(
        r#"
        UPDATE inbox_messages
        SET is_dismissed = FALSE, dismissed_at = NULL, updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2 AND user_id = $3
        RETURNING id, tenant_id, user_id, notification_id, title, body, category,
                  is_read, is_dismissed, read_at, dismissed_at, created_at, updated_at
        "#,
    )
    .bind(message_id)
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(ref m) = msg {
        let envelope = create_notifications_envelope(
            Uuid::new_v4(),
            tenant_id.to_string(),
            "notifications.inbox.message_undismissed".to_string(),
            None,
            None,
            "LIFECYCLE".to_string(),
            serde_json::json!({
                "inbox_message_id": m.id,
                "user_id": user_id,
            }),
        );
        enqueue_event(
            &mut tx,
            "notifications.events.inbox.message_undismissed",
            &envelope,
        )
        .await?;
    }

    tx.commit().await?;
    Ok(msg)
}
