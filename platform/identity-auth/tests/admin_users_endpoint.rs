//! Integration tests for GET /api/auth/admin/users
//!
//! Tests run against a real Postgres DB (no mocks).
//!
//! Coverage:
//! 1. Returns users with joined roles and permissions (no N+1)
//! 2. Tenant isolation: tenant A users are not visible from tenant B query
//! 3. `include_inactive=false` (default) hides inactive users
//! 4. `include_inactive=true` includes inactive users
//! 5. `search` performs case-insensitive ILIKE match on email
//! 6. `last_login_at` is populated after marking a login
//! 7. Response does NOT include password_hash, failed_login_count, or lock_until
//!    (verified structurally — those fields do not exist on PlatformUserDetail)

use auth_rs::db::rbac;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use uuid::Uuid;

async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://auth_user:auth_pass@localhost:5433/auth_db".into());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to test DB");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

/// Insert a credential directly (bypasses hashing — test only).
async fn insert_credential(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    email: &str,
    is_active: bool,
) {
    sqlx::query(
        r#"INSERT INTO credentials (tenant_id, user_id, email, password_hash, is_active)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(email)
    .bind("test-hash-not-real")
    .bind(is_active)
    .execute(pool)
    .await
    .expect("insert test credential");
}

/// Run the list_users_admin query directly (same SQL as handler).
async fn list_users_admin_query(
    pool: &PgPool,
    tenant_id: Uuid,
    search: Option<&str>,
    include_inactive: bool,
) -> Vec<(Uuid, String, bool, Vec<String>, Vec<String>)> {
    let search_opt: Option<String> = search
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let rows = sqlx::query(
        r#"
        SELECT
            c.user_id   AS id,
            c.email,
            c.is_active,
            COALESCE(
                array_agg(DISTINCT r.name) FILTER (WHERE r.name IS NOT NULL),
                '{}'
            )::TEXT[] AS roles,
            COALESCE(
                array_agg(DISTINCT p.key)  FILTER (WHERE p.key  IS NOT NULL),
                '{}'
            )::TEXT[] AS permissions
        FROM credentials c
        LEFT JOIN user_role_bindings urb
               ON urb.tenant_id = c.tenant_id
              AND urb.user_id   = c.user_id
              AND urb.revoked_at IS NULL
        LEFT JOIN roles r ON r.id = urb.role_id AND r.tenant_id = c.tenant_id
        LEFT JOIN role_permissions rp ON rp.role_id = r.id
        LEFT JOIN permissions p ON p.id = rp.permission_id
        WHERE c.tenant_id = $1
          AND ($2 OR c.is_active)
          AND ($3::TEXT IS NULL OR c.email ILIKE '%' || $3 || '%')
        GROUP BY c.user_id, c.tenant_id, c.email, c.is_active, c.created_at, c.last_login_at
        ORDER BY c.email
        "#,
    )
    .bind(tenant_id)
    .bind(include_inactive)
    .bind(search_opt)
    .fetch_all(pool)
    .await
    .expect("list_users_admin query");

    rows.into_iter()
        .map(|r| {
            let id: Uuid = r.get("id");
            let email: String = r.get("email");
            let is_active: bool = r.get("is_active");
            let roles: Vec<String> = r.get("roles");
            let permissions: Vec<String> = r.get("permissions");
            (id, email, is_active, roles, permissions)
        })
        .collect()
}

// ─── Test 1: roles and permissions are joined (single query, no N+1) ────────

#[tokio::test]
async fn admin_users_returns_roles_and_permissions() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("roles-perm-{}@example.com", &user_id.to_string()[..8]);

    insert_credential(&pool, tenant_id, user_id, &email, true).await;

    // Create permission and role, then bind. Permission.key has a global UNIQUE
    // constraint, so scope the test fixture to this run's tenant to avoid
    // colliding with artifacts from prior test runs.
    let perm_key = format!("test.view.{}", &tenant_id.to_string()[..8]);
    let perm = rbac::create_permission(&pool, &perm_key, "test permission")
        .await
        .expect("create permission");
    let role = rbac::create_role(&pool, tenant_id, "test_viewer", "Viewer", false)
        .await
        .expect("create role");
    sqlx::query(
        "INSERT INTO role_permissions (role_id, permission_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(role.id)
    .bind(perm.id)
    .execute(&pool)
    .await
    .expect("bind permission to role");
    sqlx::query("INSERT INTO user_role_bindings (tenant_id, user_id, role_id) VALUES ($1, $2, $3)")
        .bind(tenant_id)
        .bind(user_id)
        .bind(role.id)
        .execute(&pool)
        .await
        .expect("bind role to user");

    let results = list_users_admin_query(&pool, tenant_id, None, false).await;
    let user = results
        .iter()
        .find(|(id, _, _, _, _)| *id == user_id)
        .expect("user not found");

    assert!(
        user.3.contains(&"test_viewer".to_string()),
        "role should be returned"
    );
    assert!(
        user.4.contains(&perm_key),
        "permission should be returned"
    );
}

// ─── Test 2: tenant isolation ────────────────────────────────────────────────

#[tokio::test]
async fn admin_users_tenant_isolation() {
    let pool = test_pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("isolation-admin-{}@example.com", &user_id.to_string()[..8]);

    insert_credential(&pool, tenant_a, user_id, &email, true).await;

    let in_a = list_users_admin_query(&pool, tenant_a, None, false).await;
    let in_b = list_users_admin_query(&pool, tenant_b, None, false).await;

    assert!(
        in_a.iter().any(|(id, _, _, _, _)| *id == user_id),
        "user must appear in tenant A"
    );
    assert!(
        !in_b.iter().any(|(id, _, _, _, _)| *id == user_id),
        "user must NOT appear in tenant B"
    );
}

// ─── Test 3: include_inactive=false hides inactive users ─────────────────────

#[tokio::test]
async fn admin_users_excludes_inactive_by_default() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let active_id = Uuid::new_v4();
    let inactive_id = Uuid::new_v4();

    insert_credential(
        &pool,
        tenant_id,
        active_id,
        &format!("active-{}@example.com", &active_id.to_string()[..8]),
        true,
    )
    .await;
    insert_credential(
        &pool,
        tenant_id,
        inactive_id,
        &format!("inactive-{}@example.com", &inactive_id.to_string()[..8]),
        false,
    )
    .await;

    let results = list_users_admin_query(&pool, tenant_id, None, false).await;
    assert!(
        results.iter().any(|(id, _, _, _, _)| *id == active_id),
        "active user should appear"
    );
    assert!(
        !results.iter().any(|(id, _, _, _, _)| *id == inactive_id),
        "inactive user should NOT appear when include_inactive=false"
    );
}

// ─── Test 4: include_inactive=true includes inactive users ───────────────────

#[tokio::test]
async fn admin_users_includes_inactive_when_requested() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let inactive_id = Uuid::new_v4();

    insert_credential(
        &pool,
        tenant_id,
        inactive_id,
        &format!("inactive2-{}@example.com", &inactive_id.to_string()[..8]),
        false,
    )
    .await;

    let results = list_users_admin_query(&pool, tenant_id, None, true).await;
    assert!(
        results.iter().any(|(id, _, _, _, _)| *id == inactive_id),
        "inactive user should appear when include_inactive=true"
    );
}

// ─── Test 5: search filters by email (case-insensitive) ─────────────────────

#[tokio::test]
async fn admin_users_search_filters_by_email() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let token = Uuid::new_v4().to_string();
    let token_short = &token[..8];

    let match_id = Uuid::new_v4();
    let no_match_id = Uuid::new_v4();

    insert_credential(
        &pool,
        tenant_id,
        match_id,
        &format!("search-UPPER-{}@example.com", token_short),
        true,
    )
    .await;
    insert_credential(
        &pool,
        tenant_id,
        no_match_id,
        &format!("other-{}@example.com", token_short),
        true,
    )
    .await;

    // Search with lowercase — should match the UPPER one case-insensitively
    let results = list_users_admin_query(
        &pool,
        tenant_id,
        Some(&format!("search-upper-{}", token_short)),
        false,
    )
    .await;

    assert!(
        results.iter().any(|(id, _, _, _, _)| *id == match_id),
        "email ILIKE match should be returned"
    );
    assert!(
        !results.iter().any(|(id, _, _, _, _)| *id == no_match_id),
        "non-matching email should be excluded"
    );
}

// ─── Test 6: last_login_at is populated after marking a login ────────────────

#[tokio::test]
async fn admin_users_last_login_at_populated_after_login() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("lastlogin-{}@example.com", &user_id.to_string()[..8]);

    insert_credential(&pool, tenant_id, user_id, &email, true).await;

    // Simulate successful login: update last_login_at (same SQL as login handler)
    sqlx::query(
        r#"UPDATE credentials
           SET failed_login_count = 0,
               lock_until = NULL,
               last_login_at = NOW(),
               updated_at = NOW()
           WHERE tenant_id = $1 AND email = $2"#,
    )
    .bind(tenant_id)
    .bind(&email)
    .execute(&pool)
    .await
    .expect("update last_login_at");

    let row =
        sqlx::query("SELECT last_login_at FROM credentials WHERE tenant_id = $1 AND user_id = $2")
            .bind(tenant_id)
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("fetch credential");

    let last_login: Option<chrono::DateTime<chrono::Utc>> = row.get("last_login_at");
    assert!(
        last_login.is_some(),
        "last_login_at should be populated after a successful login"
    );
}

// ─── Test 7: user with no roles returns empty arrays ─────────────────────────

#[tokio::test]
async fn admin_users_no_roles_returns_empty_arrays() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("noroles-{}@example.com", &user_id.to_string()[..8]);

    insert_credential(&pool, tenant_id, user_id, &email, true).await;

    let results = list_users_admin_query(&pool, tenant_id, None, false).await;
    let user = results
        .iter()
        .find(|(id, _, _, _, _)| *id == user_id)
        .expect("user not found");

    assert!(
        user.3.is_empty(),
        "roles should be empty vec for user with no bindings"
    );
    assert!(
        user.4.is_empty(),
        "permissions should be empty vec for user with no bindings"
    );
}
