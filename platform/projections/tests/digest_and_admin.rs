/// Integration tests for projection digest validation and admin queries.
///
/// Covers: (1) compute_digest determinism, (2) versioned digest validation detects
/// stale projections, (3) admin query_projection_status, (4) admin query_consistency_check,
/// (5) admin query_projection_list.
/// All tests run against a real PostgreSQL database — no mocks.
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use projections::admin::{
    query_consistency_check, query_projection_list, query_projection_status,
    ConsistencyCheckRequest, ProjectionStatusRequest,
};
use projections::cursor::ProjectionCursor;
use projections::digest::{compute_versioned_digest, DIGEST_VERSION};
use projections::rebuild::compute_digest;

async fn test_pool() -> PgPool {
    let url = std::env::var("PROJECTIONS_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://projections_user:projections_pass@localhost:5439/projections_db".to_string()
    });
    PgPool::connect(&url)
        .await
        .expect("connect to projections DB")
}

async fn ensure_cursors_table(pool: &PgPool) {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS projection_cursors (
            projection_name VARCHAR(100) NOT NULL,
            tenant_id VARCHAR(100) NOT NULL,
            last_event_id UUID NOT NULL,
            last_event_occurred_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
            events_processed BIGINT NOT NULL DEFAULT 1,
            PRIMARY KEY (projection_name, tenant_id)
        )"#,
    )
    .execute(pool)
    .await
    .expect("ensure projection_cursors table");
}

async fn cleanup_cursors(pool: &PgPool, projection_name: &str) {
    sqlx::query("DELETE FROM projection_cursors WHERE projection_name = $1")
        .bind(projection_name)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// compute_digest: deterministic hash
// ============================================================================

#[tokio::test]
async fn compute_digest_is_deterministic() {
    let pool = test_pool().await;
    let table = format!("test_digest_{}", &Uuid::new_v4().to_string()[..8]);

    sqlx::query(&format!(
        "CREATE TABLE {} (id UUID PRIMARY KEY, val INT NOT NULL)",
        table
    ))
    .execute(&pool)
    .await
    .expect("create table");

    // Insert deterministic data
    let id1 = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    let id2 = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    sqlx::query(&format!(
        "INSERT INTO {} (id, val) VALUES ($1, 10), ($2, 20)",
        table
    ))
    .bind(id1)
    .bind(id2)
    .execute(&pool)
    .await
    .expect("insert data");

    // Compute digest twice — must be identical
    let digest1 = compute_digest(&pool, &table, "id").await.expect("digest1");
    let digest2 = compute_digest(&pool, &table, "id").await.expect("digest2");

    assert_eq!(digest1, digest2, "digest must be deterministic");
    assert!(!digest1.is_empty(), "digest must not be empty");

    sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", table))
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn compute_digest_changes_with_data() {
    let pool = test_pool().await;
    let table = format!("test_digest_chg_{}", &Uuid::new_v4().to_string()[..8]);

    sqlx::query(&format!(
        "CREATE TABLE {} (id UUID PRIMARY KEY, val INT NOT NULL)",
        table
    ))
    .execute(&pool)
    .await
    .expect("create table");

    let id1 = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    sqlx::query(&format!("INSERT INTO {} (id, val) VALUES ($1, 10)", table))
        .bind(id1)
        .execute(&pool)
        .await
        .expect("insert");

    let digest_before = compute_digest(&pool, &table, "id").await.expect("before");

    // Modify data
    sqlx::query(&format!("UPDATE {} SET val = 99 WHERE id = $1", table))
        .bind(id1)
        .execute(&pool)
        .await
        .expect("update");

    let digest_after = compute_digest(&pool, &table, "id").await.expect("after");

    assert_ne!(
        digest_before, digest_after,
        "digest must change when data changes"
    );

    sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", table))
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn compute_digest_empty_table() {
    let pool = test_pool().await;
    let table = format!("test_digest_empty_{}", &Uuid::new_v4().to_string()[..8]);

    sqlx::query(&format!(
        "CREATE TABLE {} (id UUID PRIMARY KEY, val INT NOT NULL)",
        table
    ))
    .execute(&pool)
    .await
    .expect("create table");

    let digest = compute_digest(&pool, &table, "id")
        .await
        .expect("empty digest");
    assert!(
        !digest.is_empty(),
        "empty-table digest must still produce a hash"
    );

    sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", table))
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Versioned digest: format and staleness detection
// ============================================================================

#[tokio::test]
async fn versioned_digest_format() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;

    // projection_cursors is in the allowlist, so we can use it
    let vd = compute_versioned_digest(&pool, "projection_cursors", "tenant_id")
        .await
        .expect("versioned digest");

    assert_eq!(vd.version, DIGEST_VERSION);
    assert!(vd.row_count >= 0);
    assert!(!vd.content_hash.is_empty());

    // String roundtrip
    let s = vd.to_string();
    let parsed = projections::digest::VersionedDigest::from_string(&s)
        .expect("parse versioned digest string");
    assert_eq!(parsed, vd);
}

#[tokio::test]
async fn versioned_digest_detects_stale_projection() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_stale_{}", &Uuid::new_v4().to_string()[..8]);
    let tenant = "tenant-stale";

    // Insert a cursor row so the table has data
    ProjectionCursor::save(&pool, &proj, tenant, Uuid::new_v4(), Utc::now())
        .await
        .expect("save cursor");

    // Compute digest
    let digest1 = compute_versioned_digest(&pool, "projection_cursors", "tenant_id")
        .await
        .expect("digest1");

    // Add another cursor (changes the data)
    let proj2 = format!("test_stale2_{}", &Uuid::new_v4().to_string()[..8]);
    ProjectionCursor::save(&pool, &proj2, tenant, Uuid::new_v4(), Utc::now())
        .await
        .expect("save cursor 2");

    let digest2 = compute_versioned_digest(&pool, "projection_cursors", "tenant_id")
        .await
        .expect("digest2");

    // Digests must differ — staleness detected
    assert_ne!(
        digest1.content_hash, digest2.content_hash,
        "digest must change when projection data changes (stale detection)"
    );
    assert!(digest2.row_count > digest1.row_count);

    cleanup_cursors(&pool, &proj).await;
    cleanup_cursors(&pool, &proj2).await;
}

// ============================================================================
// Admin: query_projection_status
// ============================================================================

#[tokio::test]
async fn admin_projection_status_returns_cursors() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_admin_st_{}", &Uuid::new_v4().to_string()[..8]);

    // Insert cursors for two tenants
    ProjectionCursor::save(&pool, &proj, "tenant-a", Uuid::new_v4(), Utc::now())
        .await
        .expect("save a");
    ProjectionCursor::save(&pool, &proj, "tenant-b", Uuid::new_v4(), Utc::now())
        .await
        .expect("save b");

    let req = ProjectionStatusRequest {
        projection_name: proj.clone(),
        tenant_id: None,
    };
    let resp = query_projection_status(&pool, &req)
        .await
        .expect("query status");

    assert_eq!(resp.projection_name, proj);
    assert_eq!(resp.status, "ok");
    assert_eq!(resp.cursors.len(), 2);

    cleanup_cursors(&pool, &proj).await;
}

#[tokio::test]
async fn admin_projection_status_filters_by_tenant() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_admin_filt_{}", &Uuid::new_v4().to_string()[..8]);

    ProjectionCursor::save(&pool, &proj, "tenant-x", Uuid::new_v4(), Utc::now())
        .await
        .expect("save x");
    ProjectionCursor::save(&pool, &proj, "tenant-y", Uuid::new_v4(), Utc::now())
        .await
        .expect("save y");

    let req = ProjectionStatusRequest {
        projection_name: proj.clone(),
        tenant_id: Some("tenant-x".to_string()),
    };
    let resp = query_projection_status(&pool, &req)
        .await
        .expect("query status");

    assert_eq!(resp.cursors.len(), 1);
    assert_eq!(resp.cursors[0].tenant_id, "tenant-x");

    cleanup_cursors(&pool, &proj).await;
}

#[tokio::test]
async fn admin_projection_status_empty_when_no_cursors() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;

    let req = ProjectionStatusRequest {
        projection_name: "nonexistent_projection_xyz".to_string(),
        tenant_id: None,
    };
    let resp = query_projection_status(&pool, &req)
        .await
        .expect("query status");

    assert_eq!(resp.status, "no_cursors");
    assert!(resp.cursors.is_empty());
}

// ============================================================================
// Admin: query_consistency_check
// ============================================================================

#[tokio::test]
async fn admin_consistency_check_for_cursors_table() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;

    let req = ConsistencyCheckRequest {
        projection_name: "projection_cursors".to_string(),
        order_by: "tenant_id".to_string(),
    };
    let resp = query_consistency_check(&pool, &req)
        .await
        .expect("consistency check");

    assert!(resp.table_exists);
    assert_eq!(resp.status, "ok");
    assert_eq!(resp.digest_version, DIGEST_VERSION);
    assert!(!resp.digest.is_empty());
    assert!(resp.row_count >= 0);
}

#[tokio::test]
async fn admin_consistency_check_missing_table() {
    let pool = test_pool().await;

    let req = ConsistencyCheckRequest {
        projection_name: "nonexistent_table_abc".to_string(),
        order_by: "tenant_id".to_string(),
    };
    let resp = query_consistency_check(&pool, &req)
        .await
        .expect("consistency check");

    assert!(!resp.table_exists);
    assert_eq!(resp.status, "table_not_found");
}

// ============================================================================
// Admin: query_projection_list
// ============================================================================

#[tokio::test]
async fn admin_projection_list_returns_known_projections() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_admin_list_{}", &Uuid::new_v4().to_string()[..8]);

    // Insert a cursor
    ProjectionCursor::save(&pool, &proj, "tenant-list", Uuid::new_v4(), Utc::now())
        .await
        .expect("save");

    let resp = query_projection_list(&pool).await.expect("list");
    assert_eq!(resp.status, "ok");

    let found = resp.projections.iter().any(|p| p.projection_name == proj);
    assert!(found, "inserted projection must appear in list");

    cleanup_cursors(&pool, &proj).await;
}

#[tokio::test]
async fn admin_projection_list_shows_tenant_count() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_list_tc_{}", &Uuid::new_v4().to_string()[..8]);

    for i in 0..3 {
        let tenant = format!("tenant-lc-{}", i);
        ProjectionCursor::save(&pool, &proj, &tenant, Uuid::new_v4(), Utc::now())
            .await
            .expect("save");
    }

    let resp = query_projection_list(&pool).await.expect("list");
    let entry = resp
        .projections
        .iter()
        .find(|p| p.projection_name == proj)
        .expect("projection must be in list");

    assert_eq!(entry.tenant_count, 3);
    assert_eq!(entry.total_events_processed, 3);

    for i in 0..3 {
        let tenant = format!("tenant-lc-{}", i);
        sqlx::query("DELETE FROM projection_cursors WHERE projection_name = $1 AND tenant_id = $2")
            .bind(&proj)
            .bind(&tenant)
            .execute(&pool)
            .await
            .ok();
    }
}
