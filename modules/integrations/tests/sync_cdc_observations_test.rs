//! Integration tests: CDC and full-resync write canonical observation rows.
//!
//! These tests exercise `process_cdc_entities` and the full-resync path against
//! a real Postgres database.  No mocks, no stubs — the DB is the authority.
//!
//! Run: ./scripts/cargo-slot.sh test -p integrations-rs -- sync_cdc_observations --nocapture

use std::time::Duration;

use chrono::Utc;
use integrations_rs::domain::qbo::cdc;
use integrations_rs::domain::sync::observations;
use serial_test::serial;
use serde_json::json;
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
    format!("cdc-obs-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_sync_observations WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

// ── CDC observation tests ─────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn cdc_writes_observation_rows_with_cdc_source_channel() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let response = json!({
        "CDCResponse": [{
            "QueryResponse": [{
                "Customer": [{
                    "Id": "cust-100",
                    "DisplayName": "Acme Corp",
                    "SyncToken": "3",
                    "MetaData": {
                        "LastUpdatedTime": "2024-06-01T10:00:00Z",
                        "CreateTime": "2024-01-01T00:00:00Z"
                    }
                }]
            }]
        }]
    });

    let (count, max_lut) = cdc::process_cdc_entities(&pool, &response, &app_id, "realm-1")
        .await
        .expect("process_cdc_entities");

    assert_eq!(count, 1, "one entity in CDC response must produce one observation");
    assert!(max_lut.is_some(), "max_lut must be set when entities are present");

    let rows = sqlx::query_as::<_, (String, bool, String)>(
        "SELECT source_channel, is_tombstone, entity_id \
         FROM integrations_sync_observations WHERE app_id = $1",
    )
    .bind(&app_id)
    .fetch_all(&pool)
    .await
    .expect("fetch");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "cdc", "source_channel must be 'cdc'");
    assert!(!rows[0].1, "is_tombstone must be false for live entity");
    assert_eq!(rows[0].2, "cust-100");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn cdc_marks_deleted_entities_as_tombstone() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let response = json!({
        "CDCResponse": [{
            "QueryResponse": [{
                "Customer": [
                    {
                        "Id": "cust-live",
                        "DisplayName": "Active Corp",
                        "SyncToken": "1",
                        "MetaData": {"LastUpdatedTime": "2024-06-01T08:00:00Z"}
                    },
                    {
                        "Id": "cust-dead",
                        "status": "Deleted",
                        "SyncToken": "2",
                        "MetaData": {"LastUpdatedTime": "2024-06-01T09:00:00Z"}
                    }
                ]
            }]
        }]
    });

    let (count, _) = cdc::process_cdc_entities(&pool, &response, &app_id, "realm-1")
        .await
        .expect("process_cdc_entities");

    assert_eq!(count, 2);

    let tombstones: Vec<(String, bool)> = sqlx::query_as(
        "SELECT entity_id, is_tombstone \
         FROM integrations_sync_observations \
         WHERE app_id = $1 \
         ORDER BY entity_id",
    )
    .bind(&app_id)
    .fetch_all(&pool)
    .await
    .expect("fetch");

    assert_eq!(tombstones.len(), 2);
    let dead = tombstones.iter().find(|(id, _)| id == "cust-dead").expect("cust-dead");
    let live = tombstones.iter().find(|(id, _)| id == "cust-live").expect("cust-live");

    assert!(dead.1, "deleted entity must be marked as tombstone");
    assert!(!live.1, "live entity must not be marked as tombstone");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn cdc_watermark_returned_is_max_last_updated_time() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let response = json!({
        "CDCResponse": [{
            "QueryResponse": [{
                "Customer": [
                    {
                        "Id": "c1",
                        "SyncToken": "1",
                        "MetaData": {"LastUpdatedTime": "2024-06-01T10:00:00Z"}
                    },
                    {
                        "Id": "c2",
                        "SyncToken": "2",
                        "MetaData": {"LastUpdatedTime": "2024-06-01T12:00:00Z"}
                    }
                ],
                "Invoice": [
                    {
                        "Id": "inv-1",
                        "SyncToken": "5",
                        "MetaData": {"LastUpdatedTime": "2024-06-01T11:30:00Z"}
                    }
                ]
            }]
        }]
    });

    let (count, max_lut) = cdc::process_cdc_entities(&pool, &response, &app_id, "realm-1")
        .await
        .expect("process_cdc_entities");

    assert_eq!(count, 3);

    let max = max_lut.expect("max_lut must be Some");
    // 2024-06-01T12:00:00Z is the latest of the three timestamps
    assert_eq!(
        max.timestamp(),
        "2024-06-01T12:00:00Z".parse::<chrono::DateTime<Utc>>().unwrap().timestamp(),
        "watermark must be the maximum LastUpdatedTime across all entities"
    );

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn cdc_empty_response_returns_zero_and_no_watermark() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let response = json!({});
    let (count, max_lut) = cdc::process_cdc_entities(&pool, &response, &app_id, "realm-1")
        .await
        .expect("process_cdc_entities");

    assert_eq!(count, 0);
    assert!(max_lut.is_none(), "empty response must return None watermark");
}

#[tokio::test]
#[serial]
async fn cdc_observation_fingerprint_uses_sync_token() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let response = json!({
        "CDCResponse": [{
            "QueryResponse": [{
                "Customer": [{
                    "Id": "cust-fp",
                    "DisplayName": "FP Test",
                    "SyncToken": "st-42",
                    "MetaData": {"LastUpdatedTime": "2024-06-01T10:00:00Z"}
                }]
            }]
        }]
    });

    cdc::process_cdc_entities(&pool, &response, &app_id, "realm-1")
        .await
        .expect("process_cdc_entities");

    let fp: (String,) = sqlx::query_as(
        "SELECT fingerprint FROM integrations_sync_observations WHERE app_id = $1",
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("fetch");

    assert_eq!(fp.0, "st:st-42", "fingerprint must use SyncToken when present");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn cdc_deduplicates_same_sync_token_on_replay() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let entity = json!({
        "Id": "cust-dedup",
        "DisplayName": "Dedup Corp",
        "SyncToken": "v9",
        "MetaData": {"LastUpdatedTime": "2024-06-15T08:00:00Z"}
    });

    let response = json!({
        "CDCResponse": [{"QueryResponse": [{"Customer": [entity.clone()]}]}]
    });

    // First observation
    let (c1, _) = cdc::process_cdc_entities(&pool, &response, &app_id, "realm-1")
        .await
        .expect("first");

    // Replay the same response (simulated CDC re-delivery)
    let (c2, _) = cdc::process_cdc_entities(&pool, &response, &app_id, "realm-1")
        .await
        .expect("second");

    assert_eq!(c1, 1);
    assert_eq!(c2, 1);

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_sync_observations WHERE app_id = $1",
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("count");

    assert_eq!(count.0, 1, "replayed CDC observation must not create duplicate rows");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn cdc_multiple_entity_types_all_written_as_observations() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let response = json!({
        "CDCResponse": [{
            "QueryResponse": [{
                "Customer": [{"Id": "c1", "SyncToken": "1", "MetaData": {"LastUpdatedTime": "2024-06-01T10:00:00Z"}}],
                "Invoice":  [{"Id": "i1", "SyncToken": "2", "MetaData": {"LastUpdatedTime": "2024-06-01T10:01:00Z"}}],
                "Payment":  [{"Id": "p1", "SyncToken": "3", "MetaData": {"LastUpdatedTime": "2024-06-01T10:02:00Z"}}],
                "Item":     [{"Id": "t1", "SyncToken": "4", "MetaData": {"LastUpdatedTime": "2024-06-01T10:03:00Z"}}]
            }]
        }]
    });

    let (count, _) = cdc::process_cdc_entities(&pool, &response, &app_id, "realm-1")
        .await
        .expect("process_cdc_entities");

    assert_eq!(count, 4, "one entity per type must produce four observation rows");

    let types: Vec<(String,)> = sqlx::query_as(
        "SELECT entity_type FROM integrations_sync_observations \
         WHERE app_id = $1 ORDER BY entity_type",
    )
    .bind(&app_id)
    .fetch_all(&pool)
    .await
    .expect("fetch");

    let type_set: std::collections::HashSet<&str> =
        types.iter().map(|(t,)| t.as_str()).collect();

    assert!(type_set.contains("customer"), "customer must be present");
    assert!(type_set.contains("invoice"),  "invoice must be present");
    assert!(type_set.contains("payment"),  "payment must be present");
    assert!(type_set.contains("item"),     "item must be present");

    cleanup(&pool, &app_id).await;
}

// ── Observation field integrity ───────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn cdc_observation_has_millisecond_normalized_last_updated_time() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    // QBO returns timestamps with sub-second precision sometimes
    let response = json!({
        "CDCResponse": [{
            "QueryResponse": [{
                "Customer": [{
                    "Id": "cust-ms",
                    "SyncToken": "1",
                    "MetaData": {"LastUpdatedTime": "2024-06-01T10:00:00.123456789Z"}
                }]
            }]
        }]
    });

    cdc::process_cdc_entities(&pool, &response, &app_id, "realm-1")
        .await
        .expect("process_cdc_entities");

    let row = observations::get_latest_for_entity(&pool, &app_id, "quickbooks", "customer", "cust-ms")
        .await
        .expect("get")
        .expect("must exist");

    assert_eq!(
        row.last_updated_time.timestamp_subsec_micros() % 1000,
        0,
        "stored last_updated_time must have zero sub-millisecond component"
    );

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn cdc_observation_source_channel_and_tombstone_fields_present() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let response = json!({
        "CDCResponse": [{
            "QueryResponse": [{
                "Payment": [{
                    "Id": "pay-sc",
                    "TotalAmt": 250.0,
                    "SyncToken": "7",
                    "MetaData": {"LastUpdatedTime": "2024-07-04T14:00:00Z"}
                }]
            }]
        }]
    });

    cdc::process_cdc_entities(&pool, &response, &app_id, "realm-1")
        .await
        .expect("process_cdc_entities");

    let row = observations::get_latest_for_entity(&pool, &app_id, "quickbooks", "payment", "pay-sc")
        .await
        .expect("get")
        .expect("must exist");

    assert_eq!(row.source_channel, "cdc");
    assert!(!row.is_tombstone);

    cleanup(&pool, &app_id).await;
}
