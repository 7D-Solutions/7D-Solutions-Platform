use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Centralized DB pool resolver for Integrations module.
///
/// DB name follows the integrations_{app_id}_db convention; the caller supplies the
/// fully-resolved DATABASE_URL (e.g. postgres://user:pass@host/integrations_default_db).
///
/// # Architecture seam
/// This resolver is the ONLY place where PgPool instances are created for Integrations.
/// All DB access MUST flow through this function — no cross-module writes.
pub async fn resolve_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let is_test = std::env::var("NODE_ENV").unwrap_or_default() == "test";
    let max_connections = if is_test { 5 } else { 10 };
    let idle_timeout = if is_test {
        std::time::Duration::from_secs(60)
    } else {
        std::time::Duration::from_secs(300)
    };

    PgPoolOptions::new()
        .max_connections(max_connections)
        .idle_timeout(Some(idle_timeout))
        .max_lifetime(Some(std::time::Duration::from_secs(1800)))
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(database_url)
        .await
}

/// Build the canonical DATABASE_URL for a given app_id.
///
/// Convention: `integrations_{app_id}_db`
pub fn database_url_for_app(base_url: &str, app_id: &str) -> String {
    let safe_id: String = app_id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .to_lowercase()
        .replace('-', "_");

    format!(
        "{}/integrations_{}_db",
        base_url.trim_end_matches('/'),
        safe_id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_url_for_app() {
        let url = database_url_for_app("postgres://user:pass@localhost", "acme");
        assert_eq!(url, "postgres://user:pass@localhost/integrations_acme_db");
    }

    #[test]
    fn test_database_url_sanitizes_app_id() {
        let url = database_url_for_app("postgres://user:pass@localhost", "my-App.Co");
        assert_eq!(
            url,
            "postgres://user:pass@localhost/integrations_my_app_co_db"
        );
    }

    #[test]
    fn test_database_url_trailing_slash() {
        let url = database_url_for_app("postgres://user:pass@localhost/", "test");
        assert_eq!(url, "postgres://user:pass@localhost/integrations_test_db");
    }

    /// Integration test: verify migrations apply cleanly and all expected
    /// tables + constraints exist in the database.
    ///
    /// Requires DATABASE_URL_INTEGRATIONS pointing to a live PostgreSQL instance.
    /// Run via: cargo test -p integrations-rs -- migration
    #[tokio::test]
    async fn test_migration_applies_and_schema_is_correct() {
        dotenvy::dotenv().ok();

        let db_url = match std::env::var("DATABASE_URL_INTEGRATIONS") {
            Ok(u) => u,
            Err(_) => {
                eprintln!(
                    "DATABASE_URL_INTEGRATIONS not set — skipping migration integration test"
                );
                return;
            }
        };

        let pool = resolve_pool(&db_url)
            .await
            .expect("Failed to connect to integrations test database");

        // Run migrations (idempotent — sqlx tracks applied versions)
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("Migrations should apply without error");

        // Verify all expected tables exist
        for table in &[
            "integrations_external_refs",
            "integrations_webhook_endpoints",
            "integrations_webhook_ingest",
            "integrations_outbox",
            "integrations_processed_events",
            "integrations_idempotency_keys",
        ] {
            let row: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM information_schema.tables
                 WHERE table_schema = 'public' AND table_name = $1",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|e| panic!("Failed to query table {table}: {e}"));

            assert_eq!(row.0, 1, "Table '{table}' should exist after migrations");
        }

        // Verify external_refs uniqueness constraint
        let constraint_row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM information_schema.table_constraints
             WHERE table_name = 'integrations_external_refs'
               AND constraint_name = 'integrations_external_refs_app_system_id_unique'
               AND constraint_type = 'UNIQUE'",
        )
        .fetch_one(&pool)
        .await
        .expect("Failed to query external_refs constraint");

        assert_eq!(
            constraint_row.0, 1,
            "UNIQUE constraint on integrations_external_refs(app_id, system, external_id) should exist"
        );

        // Verify webhook_ingest dedup constraint
        let ingest_constraint: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM information_schema.table_constraints
             WHERE table_name = 'integrations_webhook_ingest'
               AND constraint_name = 'integrations_webhook_ingest_dedup'
               AND constraint_type = 'UNIQUE'",
        )
        .fetch_one(&pool)
        .await
        .expect("Failed to query webhook_ingest constraint");

        assert_eq!(
            ingest_constraint.0, 1,
            "UNIQUE constraint on integrations_webhook_ingest(app_id, system, idempotency_key) should exist"
        );

        // Verify outbox partial index for unpublished events
        let idx_row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM pg_indexes
             WHERE tablename = 'integrations_outbox'
               AND indexname = 'idx_integrations_outbox_unpublished'",
        )
        .fetch_one(&pool)
        .await
        .expect("Failed to query outbox index");

        assert_eq!(
            idx_row.0, 1,
            "Partial index on integrations_outbox for unpublished rows should exist"
        );

        pool.close().await;
    }
}
