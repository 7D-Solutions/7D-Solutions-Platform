use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use event_bus::{BusError, BusMessage, BusResult, EventBus, InMemoryBus};
use futures::{stream, StreamExt};
use integrations_rs::outbox::{publish_batch, DEFAULT_MAX_RETRIES};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

static TEST_POOL: OnceCell<sqlx::PgPool> = OnceCell::const_new();

async fn init_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

async fn setup_db() -> sqlx::PgPool {
    TEST_POOL.get_or_init(init_pool).await.clone()
}

fn unique_app() -> String {
    format!("relay-test-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM failed_events WHERE tenant_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn insert_outbox_row(
    pool: &sqlx::PgPool,
    app_id: &str,
    event_id: Uuid,
    event_type: &str,
    payload: serde_json::Value,
) {
    sqlx::query(
        r#"
        INSERT INTO integrations_outbox (
            event_id, event_type, aggregate_type, aggregate_id, app_id, payload, schema_version, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, '1.0.0', TIMESTAMPTZ '-infinity')
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind("webhook")
    .bind("relay-test")
    .bind(app_id)
    .bind(payload)
    .execute(pool)
    .await
    .expect("insert outbox row");
}

#[tokio::test]
#[serial]
async fn test_outbox_relay_publishes_and_marks_published() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let event_id = Uuid::new_v4();
    let event_type = "relay.test.published";
    let payload = serde_json::json!({ "hello": "world" });

    cleanup(&pool, &app_id).await;
    insert_outbox_row(&pool, &app_id, event_id, event_type, payload.clone()).await;

    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let mut stream = bus
        .subscribe(event_type)
        .await
        .expect("subscribe in-memory bus");

    let published = publish_batch(&pool, &bus, DEFAULT_MAX_RETRIES)
        .await
        .expect("publish batch");
    assert!(published >= 1);

    let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("timed out waiting for bus message")
        .expect("expected bus message");

    assert_eq!(msg.subject, event_type);
    let published_payload: serde_json::Value =
        serde_json::from_slice(&msg.payload).expect("deserialize published payload");
    assert_eq!(published_payload, payload);

    let row: (
        bool,
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
    ) = sqlx::query_as(
        r#"
        SELECT published_at IS NOT NULL, retry_count, failed_at, error_message
        FROM integrations_outbox
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("fetch published row");

    assert!(row.0, "published_at should be set");
    assert_eq!(row.1, 0, "retry_count should stay at 0 on success");
    assert!(row.2.is_none(), "failed_at should remain null");
    assert!(row.3.is_none(), "error_message should remain null");

    cleanup(&pool, &app_id).await;
}

struct FailingBus;

#[async_trait]
impl EventBus for FailingBus {
    async fn publish(&self, _subject: &str, _payload: Vec<u8>) -> BusResult<()> {
        Err(BusError::PublishError("boom".to_string()))
    }

    async fn subscribe(
        &self,
        _subject: &str,
    ) -> BusResult<futures::stream::BoxStream<'static, BusMessage>> {
        Ok(stream::empty::<BusMessage>().boxed())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

#[tokio::test]
#[serial]
async fn test_outbox_relay_moves_exhausted_rows_to_failed_events() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let event_id = Uuid::new_v4();
    let event_type = "relay.test.failed";
    let payload = serde_json::json!({ "status": "retry-me" });

    cleanup(&pool, &app_id).await;
    insert_outbox_row(&pool, &app_id, event_id, event_type, payload.clone()).await;

    let bus: Arc<dyn EventBus> = Arc::new(FailingBus);

    for _ in 0..DEFAULT_MAX_RETRIES {
        let published = publish_batch(&pool, &bus, DEFAULT_MAX_RETRIES)
            .await
            .expect("publish batch failure path");
        assert_eq!(published, 0);
    }

    let outbox_row: (i32, bool, bool, Option<String>) = sqlx::query_as(
        r#"
        SELECT retry_count, published_at IS NOT NULL, failed_at IS NOT NULL, error_message
        FROM integrations_outbox
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("fetch failed outbox row");

    assert_eq!(outbox_row.0, DEFAULT_MAX_RETRIES);
    assert!(!outbox_row.1, "failed row must not be marked published");
    assert!(outbox_row.2, "failed row must be marked failed");
    assert_eq!(
        outbox_row.3.as_deref(),
        Some("failed to publish message: boom")
    );

    let failed_row: (String, String, i32, serde_json::Value) = sqlx::query_as(
        r#"
        SELECT subject, tenant_id, retry_count, envelope_json
        FROM failed_events
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("fetch failed_events row");

    assert_eq!(failed_row.0, event_type);
    assert_eq!(failed_row.1, app_id);
    assert_eq!(failed_row.2, DEFAULT_MAX_RETRIES);
    assert_eq!(failed_row.3, payload);

    cleanup(&pool, &app_id).await;
}
