//! Migration safety test helpers for 7D-Solutions Platform modules.
//!
//! Every proven module (version ≥ 1.0.0) uses these helpers in its
//! `tests/migration_safety_test.rs` to enforce three invariants:
//!
//! 1. **Apply cleanly** — all migrations run without error on a fresh DB.
//! 2. **Forward-fix rollback** — dropping the entire public schema and
//!    re-applying all migrations must succeed.  This proves the migration
//!    set is safe to replay from zero, which is the production recovery
//!    procedure for a corrupt schema.
//! 3. **Tenant isolation** — top-level data tables carry a `tenant_id`
//!    column so no cross-tenant data leakage is possible at the DB layer.
//!
//! ## FORWARD-ONLY annotation
//!
//! Any migration that cannot be rolled back must include this comment:
//! ```sql
//! -- FORWARD-ONLY: <one-line reason and production recovery path>
//! ```
//! The `check_last_n_migrations` helper surfaces this annotation so CI
//! can assert that every migration is either reversible or documented.

use std::path::Path;

use sqlx::{postgres::PgPoolOptions, PgPool};

// ── Connection ────────────────────────────────────────────────────────────────

/// Connect to PostgreSQL.
///
/// Checks `DATABASE_URL` env var first (allows CI / local `.env` override),
/// then falls back to `default_url`.
pub async fn connect_pool(default_url: &str) -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| default_url.to_string());
    PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .unwrap_or_else(|e| panic!("Failed to connect to test database at {url}: {e}"))
}

// ── Schema reset ──────────────────────────────────────────────────────────────

/// Drop the entire public schema and recreate it.
///
/// This is the **forward-fix rollback** procedure: blow away everything in the
/// public schema (all tables, types, sequences, views, functions) and then let
/// `sqlx::migrate!` re-apply from scratch.
///
/// Using `DROP SCHEMA … CASCADE` is more reliable than listing individual DROP
/// statements because it handles custom types, enums, triggers, and FK ordering
/// automatically.
pub async fn reset_public_schema(pool: &PgPool) {
    sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
        .execute(pool)
        .await
        .expect("reset_public_schema: drop failed");

    sqlx::query("CREATE SCHEMA public")
        .execute(pool)
        .await
        .expect("reset_public_schema: create failed");

    // Restore default privileges so the connection user can create objects.
    sqlx::query("GRANT ALL ON SCHEMA public TO public")
        .execute(pool)
        .await
        .expect("reset_public_schema: grant to public failed");

    sqlx::query("GRANT ALL ON SCHEMA public TO CURRENT_USER")
        .execute(pool)
        .await
        .expect("reset_public_schema: grant to current_user failed");
}

// ── Migration count ───────────────────────────────────────────────────────────

/// Return the number of successfully applied migrations in `_sqlx_migrations`.
pub async fn count_applied_migrations(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = true")
        .fetch_one(pool)
        .await
        .expect("count_applied_migrations: query failed")
}

// ── Table assertions ──────────────────────────────────────────────────────────

/// Panic unless every table name in `tables` exists in the `public` schema.
pub async fn assert_tables_exist(pool: &PgPool, tables: &[&str]) {
    for &table in tables {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = 'public' AND table_name = $1
            )",
        )
        .bind(table)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("assert_tables_exist: checking '{table}': {e}"));

        assert!(exists, "Table '{table}' must exist after migrations");
    }
}

// ── Tenant isolation ──────────────────────────────────────────────────────────

/// Panic unless every table name in `tables` has a `tenant_id` column.
pub async fn assert_tenant_id_columns(pool: &PgPool, tables: &[&str]) {
    for &table in tables {
        let has_tenant: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public'
                  AND table_name = $1
                  AND column_name = 'tenant_id'
            )",
        )
        .bind(table)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("assert_tenant_id_columns: checking '{table}': {e}"));

        assert!(
            has_tenant,
            "Table '{table}' must have a tenant_id column for multi-tenant isolation"
        );
    }
}

/// Panic unless at least `min_count` tables in the public schema have a
/// `tenant_id` column.  Use this for modules where the exact table list is
/// subject to change, as a lightweight invariant that isolation is present.
pub async fn assert_min_tables_with_tenant_id(pool: &PgPool, min_count: i64) {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT t.table_name)
         FROM information_schema.tables t
         JOIN information_schema.columns c
           ON c.table_schema = t.table_schema
          AND c.table_name   = t.table_name
          AND c.column_name  = 'tenant_id'
         WHERE t.table_schema = 'public'
           AND t.table_type   = 'BASE TABLE'
           AND t.table_name  != '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await
    .expect("assert_min_tables_with_tenant_id: query failed");

    assert!(
        count >= min_count,
        "Expected at least {min_count} tables with tenant_id for multi-tenant isolation, found {count}"
    );
}

// ── Migration file inspection ─────────────────────────────────────────────────

/// Metadata about a single migration `.sql` file.
pub struct MigrationInfo {
    /// Filename only (not the full path).
    pub filename: String,
    /// `true` if the file contains a `-- FORWARD-ONLY:` annotation.
    pub is_forward_only: bool,
    /// The reason text after `-- FORWARD-ONLY:`, if present.
    pub forward_only_reason: Option<String>,
}

/// Read the last `n` migration files (sorted ascending by filename) and return
/// their [`MigrationInfo`].
///
/// Pass `concat!(env!("CARGO_MANIFEST_DIR"), "/db/migrations")` as
/// `migrations_dir` from a module's test so the path is resolved at
/// compile time relative to the module's `Cargo.toml`.
pub fn check_last_n_migrations(migrations_dir: &str, n: usize) -> Vec<MigrationInfo> {
    let dir = Path::new(migrations_dir);
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("check_last_n_migrations: cannot read '{migrations_dir}': {e}"))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "sql"))
        .map(|e| e.path())
        .collect();

    paths.sort();

    let last_n: Vec<_> = paths.iter().rev().take(n).rev().collect();

    last_n
        .into_iter()
        .map(|path| {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();
            let content = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("check_last_n_migrations: cannot read '{filename}': {e}"));

            let forward_only_line = content
                .lines()
                .find(|line| line.trim_start().starts_with("-- FORWARD-ONLY:"))
                .map(|l| l.to_owned());

            let reason = forward_only_line.as_deref().map(|l| {
                l.trim_start()
                    .trim_start_matches("--")
                    .trim()
                    .trim_start_matches("FORWARD-ONLY:")
                    .trim()
                    .to_string()
            });

            MigrationInfo {
                filename,
                is_forward_only: forward_only_line.is_some(),
                forward_only_reason: reason,
            }
        })
        .collect()
}
