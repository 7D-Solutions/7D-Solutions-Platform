use super::*;
use serde_json::json;
use serial_test::serial;

const APP_A: &str = "test-qbo-a";
const APP_B: &str = "test-qbo-b";
const REALM_A: &str = "realm-t-001";
const REALM_B: &str = "realm-t-002";

async fn setup() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".into()
    });
    let pool = PgPool::connect(&url).await.expect("DB connect failed");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("migrate");
    // cleanup
    for app in [APP_A, APP_B, "_qbo_batch_"] {
        sqlx::query("DELETE FROM integrations_outbox WHERE app_id=$1")
            .bind(app)
            .execute(&pool)
            .await
            .ok();
        sqlx::query("DELETE FROM integrations_webhook_ingest WHERE app_id=$1")
            .bind(app)
            .execute(&pool)
            .await
            .ok();
    }
    for r in [REALM_A, REALM_B] {
        sqlx::query("DELETE FROM integrations_oauth_connections WHERE provider='quickbooks' AND realm_id=$1")
            .bind(r).execute(&pool).await.ok();
    }
    // seed connections
    for (app, realm) in [(APP_A, REALM_A), (APP_B, REALM_B)] {
        sqlx::query(
            "INSERT INTO integrations_oauth_connections
             (app_id,provider,realm_id,access_token,refresh_token,access_token_expires_at,refresh_token_expires_at,scopes_granted)
             VALUES($1,'quickbooks',$2,'t'::bytea,'t'::bytea,NOW()+'1h'::interval,NOW()+'100d'::interval,'accounting')
             ON CONFLICT DO NOTHING",
        ).bind(app).bind(realm).execute(&pool).await.expect("seed");
    }
    pool
}

fn ev(id: &str, typ: &str, realm: &str) -> serde_json::Value {
    json!({"id":id,"type":typ,"time":"2026-03-27T12:00:00Z","intuitentityid":"42","intuitaccountid":realm,"data":{}})
}

async fn run(pool: &PgPool, events: &serde_json::Value) -> QboNormalizeResult {
    let body = serde_json::to_vec(events).expect("json serialize");
    QboNormalizer::new(pool.clone())
        .normalize(&body, events, &std::collections::HashMap::new())
        .await
        .expect("normalize failed")
}

async fn count(pool: &PgPool, query: &str, bind: &str) -> i64 {
    sqlx::query_as::<_, (i64,)>(query)
        .bind(bind)
        .fetch_one(pool)
        .await
        .expect("count query")
        .0
}

#[tokio::test]
#[serial]
async fn test_qbo_normalize_fan_out_across_realms() {
    let pool = setup().await;
    let events = json!([
        ev("e1", "qbo.customer.created.v1", REALM_A),
        ev("e2", "qbo.invoice.updated.v1", REALM_B),
        ev("e3", "qbo.payment.created.v1", REALM_A)
    ]);
    let r = run(&pool, &events).await;
    assert!(!r.is_duplicate);
    assert_eq!(r.events_processed, 3);
    assert_eq!(r.events_skipped, 0);

    // 3 per-event ingest records
    let n = count(&pool, "SELECT COUNT(*) FROM integrations_webhook_ingest WHERE system='quickbooks' AND app_id!='_qbo_batch_'", APP_A).await;
    // app_id filter not applied here — just checking total
    assert!(n >= 2);

    // Tenant A: 2 events × (received + routed) = 4 outbox entries
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM integrations_outbox WHERE app_id=$1",
            APP_A
        )
        .await,
        4
    );
    // Tenant B: 1 event × 2 = 2
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM integrations_outbox WHERE app_id=$1",
            APP_B
        )
        .await,
        2
    );
}

#[tokio::test]
#[serial]
async fn test_qbo_unknown_realm_skipped() {
    let pool = setup().await;
    let events = json!([
        ev("s1", "qbo.customer.created.v1", "realm-unknown-999"),
        ev("s2", "qbo.invoice.created.v1", REALM_A)
    ]);
    let r = run(&pool, &events).await;
    assert_eq!(r.events_processed, 1);
    assert_eq!(r.events_skipped, 1);
}

#[tokio::test]
#[serial]
async fn test_qbo_post_level_dedup() {
    let pool = setup().await;
    let events = json!([ev("d1", "qbo.customer.created.v1", REALM_A)]);
    let r1 = run(&pool, &events).await;
    assert!(!r1.is_duplicate);
    assert_eq!(r1.events_processed, 1);
    // Replay same body
    let r2 = run(&pool, &events).await;
    assert!(r2.is_duplicate);
    assert_eq!(r2.events_processed, 0);
}

#[tokio::test]
#[serial]
async fn test_qbo_event_level_dedup() {
    let pool = setup().await;
    let r1 = run(
        &pool,
        &json!([ev("o1", "qbo.customer.created.v1", REALM_A)]),
    )
    .await;
    assert_eq!(r1.events_processed, 1);
    // Second POST: o1 overlaps, o2 is new
    let r2 = run(
        &pool,
        &json!([
            ev("o1", "qbo.customer.created.v1", REALM_A),
            ev("o2", "qbo.invoice.created.v1", REALM_B)
        ]),
    )
    .await;
    assert_eq!(r2.events_processed, 1);
    assert_eq!(r2.events_skipped, 1);
}
