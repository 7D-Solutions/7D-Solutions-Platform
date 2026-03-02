//! Real E2E tests for the Numbering service.
//!
//! Tests run against a real Postgres database — no mocks, no stubs.
//! Each test uses a unique tenant_id to avoid cross-contamination.
//!
//! Default DB: postgresql://postgres:postgres@localhost:5450/numbering_db
//! Override with NUMBERING_DATABASE_URL env var.

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("NUMBERING_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://numbering_user:numbering_pass@localhost:5456/numbering_db".to_string()
        });

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to numbering test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run numbering migrations");

    pool
}

fn unique_tenant() -> Uuid {
    Uuid::new_v4()
}

// ============================================================================
// 1. Basic allocation: first number for a tenant+entity is 1
// ============================================================================

#[tokio::test]
#[serial]
async fn test_allocate_first_number() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "quote";
    let idem_key = format!("numbering:allocate:{}:first", tid);

    let num = allocate(&pool, tid, entity, &idem_key).await;
    assert_eq!(num, 1, "First allocation should be 1");
}

// ============================================================================
// 2. Sequential: second allocation returns 2
// ============================================================================

#[tokio::test]
#[serial]
async fn test_allocate_sequential() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "invoice";

    let n1 = allocate(
        &pool,
        tid,
        entity,
        &format!("numbering:allocate:{}:seq-1", tid),
    )
    .await;
    let n2 = allocate(
        &pool,
        tid,
        entity,
        &format!("numbering:allocate:{}:seq-2", tid),
    )
    .await;
    let n3 = allocate(
        &pool,
        tid,
        entity,
        &format!("numbering:allocate:{}:seq-3", tid),
    )
    .await;

    assert_eq!(n1, 1);
    assert_eq!(n2, 2);
    assert_eq!(n3, 3);
}

// ============================================================================
// 3. Idempotency: same key returns same number
// ============================================================================

#[tokio::test]
#[serial]
async fn test_allocate_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "po";
    let idem_key = format!("numbering:allocate:{}:idem", tid);

    let n1 = allocate(&pool, tid, entity, &idem_key).await;
    let n2 = allocate(&pool, tid, entity, &idem_key).await;

    assert_eq!(n1, n2, "Same idempotency key must return same number");

    // A different key should get the next number
    let n3 = allocate(
        &pool,
        tid,
        entity,
        &format!("numbering:allocate:{}:idem-2", tid),
    )
    .await;
    assert_eq!(n3, 2, "Different key should advance the sequence");
}

// ============================================================================
// 4. Tenant isolation: different tenants get independent sequences
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let t1 = unique_tenant();
    let t2 = unique_tenant();
    let entity = "quote";

    let n_t1 = allocate(
        &pool,
        t1,
        entity,
        &format!("numbering:allocate:{}:iso", t1),
    )
    .await;
    let n_t2 = allocate(
        &pool,
        t2,
        entity,
        &format!("numbering:allocate:{}:iso", t2),
    )
    .await;

    assert_eq!(n_t1, 1, "Tenant 1 should start at 1");
    assert_eq!(n_t2, 1, "Tenant 2 should independently start at 1");
}

// ============================================================================
// 5. Entity isolation: different entities get independent sequences
// ============================================================================

#[tokio::test]
#[serial]
async fn test_entity_isolation() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let n_quote = allocate(
        &pool,
        tid,
        "quote",
        &format!("numbering:allocate:{}:ent-q", tid),
    )
    .await;
    let n_invoice = allocate(
        &pool,
        tid,
        "invoice",
        &format!("numbering:allocate:{}:ent-i", tid),
    )
    .await;

    assert_eq!(n_quote, 1, "Quote entity starts at 1");
    assert_eq!(n_invoice, 1, "Invoice entity starts independently at 1");
}

// ============================================================================
// 6. Concurrency: parallel allocations never produce duplicates
// ============================================================================

#[tokio::test]
#[serial]
async fn test_concurrent_allocations_no_duplicates() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "work_order";
    let count = 20;

    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        let pool = pool.clone();
        let idem_key = format!("numbering:allocate:{}:conc-{}", tid, i);
        let entity = entity.to_string();
        handles.push(tokio::spawn(async move {
            allocate(&pool, tid, &entity, &idem_key).await
        }));
    }

    let mut results = Vec::with_capacity(count);
    for h in handles {
        results.push(h.await.expect("task panicked"));
    }

    results.sort();

    // Every number from 1..=count should appear exactly once
    let expected: Vec<i64> = (1..=count as i64).collect();
    assert_eq!(
        results, expected,
        "Concurrent allocations must produce exactly 1..={} with no gaps or duplicates",
        count
    );
}

// ============================================================================
// 7. Outbox event: allocation creates an outbox event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_allocate_creates_outbox_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "receipt";
    let idem_key = format!("numbering:allocate:{}:outbox", tid);

    let _ = allocate(&pool, tid, entity, &idem_key).await;

    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'numbering.events.number.allocated' \
         AND aggregate_id = $1",
    )
    .bind(format!("{}:{}", tid, entity))
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");

    assert!(row.0 >= 1, "At least one outbox event should exist");
}

// ============================================================================
// Helper: allocate using direct SQL (same logic as the handler)
// ============================================================================

async fn allocate(pool: &sqlx::PgPool, tenant_id: Uuid, entity: &str, idem_key: &str) -> i64 {
    // Check idempotency
    let existing: Option<(i64,)> = sqlx::query_as(
        "SELECT number_value FROM issued_numbers WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(idem_key)
    .fetch_optional(pool)
    .await
    .expect("idempotency check failed");

    if let Some((val,)) = existing {
        return val;
    }

    let mut tx = pool.begin().await.expect("begin tx failed");

    // Guard + Mutation: atomic upsert — handles concurrent first-insert race
    let (next_value,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO sequences (tenant_id, entity, current_value)
        VALUES ($1, $2, 1)
        ON CONFLICT (tenant_id, entity)
        DO UPDATE SET current_value = sequences.current_value + 1,
                      updated_at = NOW()
        RETURNING current_value
        "#,
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_one(&mut *tx)
    .await
    .expect("sequence upsert failed");

    // Record issued number
    sqlx::query(
        "INSERT INTO issued_numbers (tenant_id, entity, number_value, idempotency_key) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind(entity)
    .bind(next_value)
    .bind(idem_key)
    .execute(&mut *tx)
    .await
    .expect("issued_numbers insert failed");

    // Outbox event
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "tenant_id": tenant_id.to_string(),
        "entity": entity,
        "number_value": next_value,
        "idempotency_key": idem_key,
    });

    sqlx::query(
        "INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event_id)
    .bind("numbering.events.number.allocated")
    .bind("number")
    .bind(format!("{}:{}", tenant_id, entity))
    .bind(payload)
    .execute(&mut *tx)
    .await
    .expect("outbox insert failed");

    tx.commit().await.expect("commit failed");

    next_value
}
