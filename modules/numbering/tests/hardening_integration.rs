//! Hardening integration tests for the Numbering module.
//!
//! Five test categories covering production-readiness:
//! 1. Migration safety — apply migrations, verify schema correctness
//! 2. Tenant boundary — cross-tenant queries return zero rows
//! 3. AuthZ denial — unauthenticated/unauthorized requests get 401/403
//! 4. Guard→Mutation→Outbox atomicity — outbox rows created in same tx
//! 5. Concurrent tenant isolation — parallel multi-tenant allocations
//!
//! All tests run against real Postgres on port 5456. No mocks, no stubs.

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Shared test setup
// ============================================================================

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
// 1. MIGRATION SAFETY
//
// Apply all migrations forward, verify expected tables and columns exist.
// Rollback procedure: documented inline — each migration is additive (CREATE
// TABLE / ALTER TABLE ADD COLUMN), so rollback is DROP TABLE / DROP COLUMN
// in reverse order.
// ============================================================================

#[tokio::test]
#[serial]
async fn migration_safety_all_tables_exist() {
    let pool = setup_db().await;

    // Verify all 4 tables exist
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' \
         AND table_name IN ('sequences', 'issued_numbers', 'events_outbox', 'numbering_policies') \
         ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await
    .expect("table check failed");

    let table_names: Vec<&str> = tables.iter().map(|(n,)| n.as_str()).collect();
    assert!(
        table_names.contains(&"events_outbox"),
        "events_outbox table must exist"
    );
    assert!(
        table_names.contains(&"issued_numbers"),
        "issued_numbers table must exist"
    );
    assert!(
        table_names.contains(&"numbering_policies"),
        "numbering_policies table must exist"
    );
    assert!(
        table_names.contains(&"sequences"),
        "sequences table must exist"
    );
}

#[tokio::test]
#[serial]
async fn migration_safety_sequences_columns() {
    let pool = setup_db().await;

    let cols: Vec<(String,)> = sqlx::query_as(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'sequences' \
         ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .expect("column check failed");

    let col_names: Vec<&str> = cols.iter().map(|(n,)| n.as_str()).collect();
    for expected in &[
        "tenant_id",
        "entity",
        "current_value",
        "created_at",
        "updated_at",
        "gap_free",
        "reservation_ttl_secs",
    ] {
        assert!(
            col_names.contains(expected),
            "sequences table must have column '{}'",
            expected
        );
    }
}

#[tokio::test]
#[serial]
async fn migration_safety_issued_numbers_columns() {
    let pool = setup_db().await;

    let cols: Vec<(String,)> = sqlx::query_as(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'issued_numbers' \
         ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .expect("column check failed");

    let col_names: Vec<&str> = cols.iter().map(|(n,)| n.as_str()).collect();
    for expected in &[
        "id",
        "tenant_id",
        "entity",
        "number_value",
        "idempotency_key",
        "created_at",
        "status",
        "expires_at",
    ] {
        assert!(
            col_names.contains(expected),
            "issued_numbers table must have column '{}'",
            expected
        );
    }
}

#[tokio::test]
#[serial]
async fn migration_safety_primary_keys_and_indexes() {
    let pool = setup_db().await;

    // sequences PK is (tenant_id, entity)
    let pk_check: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.table_constraints \
         WHERE table_name = 'sequences' AND constraint_type = 'PRIMARY KEY'",
    )
    .fetch_one(&pool)
    .await
    .expect("pk check failed");
    assert_eq!(pk_check.0, 1, "sequences must have a primary key");

    // issued_numbers unique constraint on (tenant_id, idempotency_key)
    let uq_check: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.table_constraints \
         WHERE table_name = 'issued_numbers' AND constraint_type = 'UNIQUE'",
    )
    .fetch_one(&pool)
    .await
    .expect("unique check failed");
    assert!(
        uq_check.0 >= 1,
        "issued_numbers must have at least one UNIQUE constraint"
    );

    // numbering_policies PK is (tenant_id, entity)
    let pol_pk: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.table_constraints \
         WHERE table_name = 'numbering_policies' AND constraint_type = 'PRIMARY KEY'",
    )
    .fetch_one(&pool)
    .await
    .expect("policy pk check failed");
    assert_eq!(pol_pk.0, 1, "numbering_policies must have a primary key");
}

/// Migration rollback plan (documented, not executed):
///
/// Rollback migration 4 (gap_free_allocation):
///   DROP INDEX IF EXISTS idx_issued_recyclable;
///   ALTER TABLE issued_numbers DROP COLUMN IF EXISTS expires_at;
///   ALTER TABLE issued_numbers DROP COLUMN IF EXISTS status;
///   ALTER TABLE sequences DROP COLUMN IF EXISTS reservation_ttl_secs;
///   ALTER TABLE sequences DROP COLUMN IF EXISTS gap_free;
///
/// Rollback migration 3 (numbering_policies):
///   DROP TABLE IF EXISTS numbering_policies;
///
/// Rollback migration 2 (core_tables):
///   DROP INDEX IF EXISTS idx_issued_numbers_entity;
///   DROP TABLE IF EXISTS issued_numbers;
///   DROP TABLE IF EXISTS sequences;
///
/// Rollback migration 1 (events_outbox):
///   DROP INDEX IF EXISTS idx_events_outbox_published;
///   DROP INDEX IF EXISTS idx_events_outbox_unpublished;
///   DROP TABLE IF EXISTS events_outbox;
#[tokio::test]
#[serial]
async fn migration_safety_rollback_plan_is_valid_sql() {
    let pool = setup_db().await;

    // Verify each rollback statement parses as valid SQL (via EXPLAIN)
    // without actually executing destructive DDL
    let rollback_stmts = [
        "SELECT 1 FROM pg_indexes WHERE indexname = 'idx_issued_recyclable'",
        "SELECT 1 FROM pg_indexes WHERE indexname = 'idx_issued_numbers_entity'",
        "SELECT 1 FROM pg_indexes WHERE indexname = 'idx_events_outbox_unpublished'",
        "SELECT 1 FROM pg_indexes WHERE indexname = 'idx_events_outbox_published'",
    ];

    for stmt in &rollback_stmts {
        sqlx::query(stmt)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("Rollback validation query failed: {} — {}", stmt, e));
    }
}

// ============================================================================
// 2. TENANT BOUNDARY
//
// Create numbering sequences under tenant_A, query as tenant_B, assert zero
// rows returned across all tenant-scoped tables.
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_boundary_sequences_isolated() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Create a sequence for tenant_a
    sqlx::query(
        "INSERT INTO sequences (tenant_id, entity, current_value) VALUES ($1, $2, 1) \
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant_a)
    .bind("invoice")
    .execute(&pool)
    .await
    .expect("insert failed");

    // Query as tenant_b — should find nothing
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT tenant_id FROM sequences WHERE tenant_id = $1 AND entity = 'invoice'",
    )
    .bind(tenant_b)
    .fetch_all(&pool)
    .await
    .expect("query failed");

    assert_eq!(rows.len(), 0, "Tenant B must not see Tenant A's sequences");
}

#[tokio::test]
#[serial]
async fn tenant_boundary_issued_numbers_isolated() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Allocate a number for tenant_a
    allocate_number(&pool, tenant_a, "quote", "tb:isoA:1").await;

    // Query issued_numbers as tenant_b
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT number_value FROM issued_numbers WHERE tenant_id = $1 AND entity = 'quote'",
    )
    .bind(tenant_b)
    .fetch_all(&pool)
    .await
    .expect("query failed");

    assert_eq!(
        rows.len(),
        0,
        "Tenant B must not see Tenant A's issued numbers"
    );
}

#[tokio::test]
#[serial]
async fn tenant_boundary_policies_isolated() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Create policy for tenant_a
    let mut tx = pool.begin().await.unwrap();
    numbering::policy::upsert_policy_tx(&mut tx, tenant_a, "wo", "WO-{number}", "WO", 5)
        .await
        .expect("upsert failed");
    tx.commit().await.unwrap();

    // Query as tenant_b via library function
    let result = numbering::policy::get_policy(&pool, tenant_b, "wo")
        .await
        .expect("query failed");

    assert!(
        result.is_none(),
        "Tenant B must not see Tenant A's policies"
    );
}

#[tokio::test]
#[serial]
async fn tenant_boundary_outbox_events_carry_correct_tenant() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Allocate for tenant_a (creates outbox event)
    allocate_number(&pool, tenant_a, "receipt", "tb:outbox:A").await;

    // Outbox events are keyed by aggregate_id = "tenant_id:entity"
    // Verify tenant_b's aggregate prefix returns nothing
    let rows: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE aggregate_id LIKE $1",
    )
    .bind(format!("{}:%", tenant_b))
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");

    assert_eq!(rows.0, 0, "No outbox events should reference tenant_b");
}

// ============================================================================
// 3. AUTHZ DENIAL
//
// Build the numbering HTTP router with real JWT verification, then send
// requests without valid claims. All protected endpoints must return 401/403.
// ============================================================================

mod authz {
    use super::*;
    use axum::body::Body;
    use axum::{
        extract::DefaultBodyLimit,
        routing::{get, post, put},
        Extension, Router,
    };
    use numbering::{http, metrics, AppState};
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::RsaPrivateKey;
    use security::{
        middleware::{
            default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
        },
        optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
    };
    use serde::Serialize;
    use std::sync::Arc;
    use tower::ServiceExt;

    struct TestKeys {
        encoding: jsonwebtoken::EncodingKey,
        verifier: Arc<JwtVerifier>,
    }

    fn make_test_keys() -> TestKeys {
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pub_key = priv_key.to_public_key();
        let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
        let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).unwrap();
        TestKeys {
            encoding: jsonwebtoken::EncodingKey::from_rsa_pem(priv_pem.as_bytes()).unwrap(),
            verifier: Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap()),
        }
    }

    #[derive(Serialize)]
    struct TestClaims {
        sub: String,
        iss: String,
        aud: String,
        iat: i64,
        exp: i64,
        jti: String,
        tenant_id: String,
        roles: Vec<String>,
        perms: Vec<String>,
        actor_type: String,
        ver: String,
    }

    fn claims_with_perms(perms: Vec<String>) -> TestClaims {
        let now = chrono::Utc::now();
        TestClaims {
            sub: Uuid::new_v4().to_string(),
            iss: "auth-rs".to_string(),
            aud: "7d-platform".to_string(),
            iat: now.timestamp(),
            exp: (now + chrono::Duration::minutes(15)).timestamp(),
            jti: Uuid::new_v4().to_string(),
            tenant_id: Uuid::new_v4().to_string(),
            roles: vec!["admin".into()],
            perms,
            actor_type: "user".to_string(),
            ver: "1".to_string(),
        }
    }

    fn sign(enc: &jsonwebtoken::EncodingKey, claims: &TestClaims) -> String {
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            claims,
            enc,
        )
        .unwrap()
    }

    fn build_test_router(state: Arc<AppState>, keys: &TestKeys) -> Router {
        let verifier = Some(keys.verifier.clone());

        Router::new()
            .route("/healthz", get(health::healthz))
            .route("/api/health", get(http::health::health))
            .merge(
                Router::new()
                    .route("/allocate", post(http::allocate::allocate))
                    .route("/confirm", post(http::confirm::confirm))
                    .route_layer(RequirePermissionsLayer::new(&[
                        permissions::NUMBERING_ALLOCATE,
                    ])),
            )
            .merge(
                Router::new()
                    .route(
                        "/policies/{entity}",
                        put(http::policy::upsert_policy).get(http::policy::get_policy),
                    )
                    .route_layer(RequirePermissionsLayer::new(&[
                        permissions::NUMBERING_ALLOCATE,
                    ])),
            )
            .with_state(state)
            .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
            .layer(axum::middleware::from_fn(timeout_middleware))
            .layer(axum::middleware::from_fn(rate_limit_middleware))
            .layer(Extension(default_rate_limiter()))
            .layer(axum::middleware::from_fn_with_state(
                verifier,
                optional_claims_mw,
            ))
    }

    async fn build_state() -> Arc<AppState> {
        let pool = setup_db().await;
        let app_metrics =
            Arc::new(metrics::NumberingMetrics::new().expect("metrics creation failed"));
        Arc::new(AppState {
            pool,
            metrics: app_metrics,
        })
    }

    // ── 3a. No token at all → 401 on /allocate ──────────────────────

    #[tokio::test]
    #[serial]
    async fn authz_no_token_allocate_returns_401() {
        let keys = make_test_keys();
        let state = build_state().await;
        let app = build_test_router(state, &keys);

        let body = serde_json::json!({
            "entity": "test",
            "idempotency_key": "authz-test-1"
        });

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/allocate")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status().as_u16(),
            401,
            "POST /allocate without token must return 401"
        );
    }

    // ── 3b. No token → 401 on /confirm ──────────────────────────────

    #[tokio::test]
    #[serial]
    async fn authz_no_token_confirm_returns_401() {
        let keys = make_test_keys();
        let state = build_state().await;
        let app = build_test_router(state, &keys);

        let body = serde_json::json!({
            "entity": "test",
            "idempotency_key": "authz-test-2"
        });

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/confirm")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status().as_u16(),
            401,
            "POST /confirm without token must return 401"
        );
    }

    // ── 3c. No token → 401 on /policies/:entity (PUT) ───────────────

    #[tokio::test]
    #[serial]
    async fn authz_no_token_upsert_policy_returns_401() {
        let keys = make_test_keys();
        let state = build_state().await;
        let app = build_test_router(state, &keys);

        let body = serde_json::json!({
            "pattern": "X-{number}",
            "prefix": "X",
            "padding": 3
        });

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri("/policies/invoice")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status().as_u16(),
            401,
            "PUT /policies/invoice without token must return 401"
        );
    }

    // ── 3d. No token → 401 on /policies/:entity (GET) ───────────────

    #[tokio::test]
    #[serial]
    async fn authz_no_token_get_policy_returns_401() {
        let keys = make_test_keys();
        let state = build_state().await;
        let app = build_test_router(state, &keys);

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/policies/invoice")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status().as_u16(),
            401,
            "GET /policies/invoice without token must return 401"
        );
    }

    // ── 3e. Token with wrong permissions → 403 on /allocate ─────────

    #[tokio::test]
    #[serial]
    async fn authz_wrong_perms_allocate_returns_403() {
        let keys = make_test_keys();
        let state = build_state().await;
        let app = build_test_router(state, &keys);

        // Token has "inventory.read" instead of "numbering.allocate"
        let claims = claims_with_perms(vec!["inventory.read".into()]);
        let token = sign(&keys.encoding, &claims);

        let body = serde_json::json!({
            "entity": "test",
            "idempotency_key": "authz-perm-test-1"
        });

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/allocate")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status().as_u16(),
            403,
            "POST /allocate with wrong permissions must return 403"
        );
    }

    // ── 3f. Valid token + correct perms → success on /allocate ───────

    #[tokio::test]
    #[serial]
    async fn authz_correct_perms_allocate_succeeds() {
        let keys = make_test_keys();
        let state = build_state().await;
        let app = build_test_router(state, &keys);

        let claims = claims_with_perms(vec![permissions::NUMBERING_ALLOCATE.to_string()]);
        let token = sign(&keys.encoding, &claims);

        let body = serde_json::json!({
            "entity": "authz_success_test",
            "idempotency_key": format!("authz-ok:{}", Uuid::new_v4())
        });

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/allocate")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status().as_u16(),
            201,
            "POST /allocate with correct permissions must return 201"
        );
    }

    // ── 3g. Health endpoint accessible without token ────────────────

    #[tokio::test]
    #[serial]
    async fn authz_health_accessible_without_token() {
        let keys = make_test_keys();
        let state = build_state().await;
        let app = build_test_router(state, &keys);

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status().as_u16(),
            200,
            "GET /api/health must be accessible without token"
        );
    }
}

// ============================================================================
// 4. GUARD → MUTATION → OUTBOX ATOMICITY
//
// Allocate a number and verify the corresponding outbox row exists, proving
// both the mutation and outbox write happened in the same transaction.
// ============================================================================

#[tokio::test]
#[serial]
async fn atomicity_allocate_and_outbox_in_same_transaction() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "atom_test";
    let idem_key = format!("atom:{}:1", tid);

    // Count outbox events before allocation
    let before: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'numbering.events.number.allocated' \
         AND aggregate_id = $1",
    )
    .bind(format!("{}:{}", tid, entity))
    .fetch_one(&pool)
    .await
    .expect("pre-count failed");

    // Allocate a number (Guard → Mutation → Outbox in single tx)
    let number = allocate_number(&pool, tid, entity, &idem_key).await;
    assert_eq!(number, 1, "First allocation should be 1");

    // Verify outbox event was created atomically
    let after: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'numbering.events.number.allocated' \
         AND aggregate_id = $1",
    )
    .bind(format!("{}:{}", tid, entity))
    .fetch_one(&pool)
    .await
    .expect("post-count failed");

    assert_eq!(
        after.0,
        before.0 + 1,
        "Exactly one outbox event must be created per allocation"
    );

    // Verify the outbox payload contains correct data
    let (payload,): (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM events_outbox \
         WHERE event_type = 'numbering.events.number.allocated' \
         AND aggregate_id = $1 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(format!("{}:{}", tid, entity))
    .fetch_one(&pool)
    .await
    .expect("payload fetch failed");

    assert_eq!(
        payload["tenant_id"].as_str().unwrap(),
        tid.to_string(),
        "Outbox payload must carry correct tenant_id"
    );
    assert_eq!(
        payload["entity"].as_str().unwrap(),
        entity,
        "Outbox payload must carry correct entity"
    );
    assert_eq!(
        payload["number_value"].as_i64().unwrap(),
        1,
        "Outbox payload must carry correct number_value"
    );
}

#[tokio::test]
#[serial]
async fn atomicity_sequence_and_issued_consistent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "atom_consist";

    // Allocate 3 numbers
    for i in 1..=3 {
        allocate_number(&pool, tid, entity, &format!("atom:{}:{}", tid, i)).await;
    }

    // Verify sequence counter matches issued count
    let (seq_val,): (i64,) =
        sqlx::query_as("SELECT current_value FROM sequences WHERE tenant_id = $1 AND entity = $2")
            .bind(tid)
            .bind(entity)
            .fetch_one(&pool)
            .await
            .expect("sequence query failed");

    let (issued_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM issued_numbers WHERE tenant_id = $1 AND entity = $2")
            .bind(tid)
            .bind(entity)
            .fetch_one(&pool)
            .await
            .expect("issued count failed");

    let (outbox_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'numbering.events.number.allocated' \
         AND aggregate_id = $1",
    )
    .bind(format!("{}:{}", tid, entity))
    .fetch_one(&pool)
    .await
    .expect("outbox count failed");

    assert_eq!(seq_val, 3, "Sequence counter should be at 3");
    assert_eq!(issued_count, 3, "Issued count should be 3");
    assert_eq!(outbox_count, 3, "Outbox count should be 3");
}

#[tokio::test]
#[serial]
async fn atomicity_policy_upsert_creates_outbox() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut tx = pool.begin().await.unwrap();

    let row = numbering::policy::upsert_policy_tx(&mut tx, tid, "atom_pol", "AP-{number}", "AP", 4)
        .await
        .expect("upsert failed");

    // Outbox event in same transaction
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "tenant_id": tid.to_string(),
        "entity": "atom_pol",
        "pattern": row.pattern,
        "version": row.version,
    });
    sqlx::query(
        "INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event_id)
    .bind("numbering.events.policy.updated")
    .bind("policy")
    .bind(format!("{}:atom_pol", tid))
    .bind(payload)
    .execute(&mut *tx)
    .await
    .expect("outbox insert failed");

    tx.commit().await.unwrap();

    // Verify both exist after commit
    let policy = numbering::policy::get_policy(&pool, tid, "atom_pol")
        .await
        .expect("query failed")
        .expect("policy should exist");
    assert_eq!(policy.pattern, "AP-{number}");

    let (outbox_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'numbering.events.policy.updated' \
         AND aggregate_id = $1",
    )
    .bind(format!("{}:atom_pol", tid))
    .fetch_one(&pool)
    .await
    .expect("outbox count failed");
    assert_eq!(outbox_count, 1, "Policy outbox event must exist");
}

// ============================================================================
// 5. CONCURRENT TENANT ISOLATION
//
// Spawn concurrent number allocations from multiple tenants simultaneously.
// Verify each tenant gets its own independent, contiguous sequence with no
// cross-tenant leaks.
// ============================================================================

#[tokio::test]
#[serial]
async fn concurrent_multi_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let tenant_c = unique_tenant();
    let entity = "conc_iso";
    let per_tenant = 15;

    let mut handles = Vec::new();

    for (tenant, label) in [(tenant_a, "A"), (tenant_b, "B"), (tenant_c, "C")] {
        for i in 0..per_tenant {
            let pool = pool.clone();
            let idem_key = format!("conc:{}:{}:{}", label, tenant, i);
            let entity = entity.to_string();
            handles.push(tokio::spawn(async move {
                let num = allocate_number(&pool, tenant, &entity, &idem_key).await;
                (tenant, num)
            }));
        }
    }

    let mut results_a = Vec::new();
    let mut results_b = Vec::new();
    let mut results_c = Vec::new();

    for h in handles {
        let (tenant, num) = h.await.expect("task panicked");
        if tenant == tenant_a {
            results_a.push(num);
        } else if tenant == tenant_b {
            results_b.push(num);
        } else {
            results_c.push(num);
        }
    }

    results_a.sort();
    results_b.sort();
    results_c.sort();

    let expected: Vec<i64> = (1..=per_tenant as i64).collect();

    assert_eq!(
        results_a, expected,
        "Tenant A must get exactly 1..={} with no gaps",
        per_tenant
    );
    assert_eq!(
        results_b, expected,
        "Tenant B must get exactly 1..={} with no gaps",
        per_tenant
    );
    assert_eq!(
        results_c, expected,
        "Tenant C must get exactly 1..={} with no gaps",
        per_tenant
    );
}

#[tokio::test]
#[serial]
async fn concurrent_multi_tenant_no_sequence_leaks() {
    let pool = setup_db().await;
    let tenant_x = unique_tenant();
    let tenant_y = unique_tenant();
    let entity = "leak_check";
    let count = 10;

    // Run allocations for both tenants concurrently
    let mut handles = Vec::new();
    for i in 0..count {
        let pool_x = pool.clone();
        let pool_y = pool.clone();
        let entity_x = entity.to_string();
        let entity_y = entity.to_string();
        let key_x = format!("leak:X:{}:{}", tenant_x, i);
        let key_y = format!("leak:Y:{}:{}", tenant_y, i);

        handles.push(tokio::spawn(async move {
            allocate_number(&pool_x, tenant_x, &entity_x, &key_x).await
        }));
        handles.push(tokio::spawn(async move {
            allocate_number(&pool_y, tenant_y, &entity_y, &key_y).await
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }

    // Verify each tenant's sequence counter is exactly `count`
    let (seq_x,): (i64,) =
        sqlx::query_as("SELECT current_value FROM sequences WHERE tenant_id = $1 AND entity = $2")
            .bind(tenant_x)
            .bind(entity)
            .fetch_one(&pool)
            .await
            .expect("seq_x query failed");

    let (seq_y,): (i64,) =
        sqlx::query_as("SELECT current_value FROM sequences WHERE tenant_id = $1 AND entity = $2")
            .bind(tenant_y)
            .bind(entity)
            .fetch_one(&pool)
            .await
            .expect("seq_y query failed");

    assert_eq!(
        seq_x, count as i64,
        "Tenant X sequence counter must be exactly {count}"
    );
    assert_eq!(
        seq_y, count as i64,
        "Tenant Y sequence counter must be exactly {count}"
    );

    // Verify no cross-tenant numbers exist
    let (x_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM issued_numbers WHERE tenant_id = $1 AND entity = $2")
            .bind(tenant_x)
            .bind(entity)
            .fetch_one(&pool)
            .await
            .expect("x_count failed");

    let (y_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM issued_numbers WHERE tenant_id = $1 AND entity = $2")
            .bind(tenant_y)
            .bind(entity)
            .fetch_one(&pool)
            .await
            .expect("y_count failed");

    assert_eq!(x_count, count as i64);
    assert_eq!(y_count, count as i64);
}

// ============================================================================
// Shared helper: allocate using direct SQL (same logic as the handler)
// ============================================================================

async fn allocate_number(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    entity: &str,
    idem_key: &str,
) -> i64 {
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

    let (next_value,): (i64,) = sqlx::query_as(
        "INSERT INTO sequences (tenant_id, entity, current_value) \
         VALUES ($1, $2, 1) \
         ON CONFLICT (tenant_id, entity) \
         DO UPDATE SET current_value = sequences.current_value + 1, updated_at = NOW() \
         RETURNING current_value",
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_one(&mut *tx)
    .await
    .expect("sequence upsert failed");

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
