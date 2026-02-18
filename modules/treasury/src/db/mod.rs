use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Centralized DB pool resolver for Treasury module.
///
/// This is the ONLY place where PgPool instances are created.
/// DB name follows the treasury_{app_id}_db convention; the caller supplies the
/// fully-resolved DATABASE_URL (e.g. postgres://user:pass@host/treasury_default_db).
///
/// # Architecture seam
/// This resolver is the injection point for per-app-id tenant routing.
/// All DB access for Treasury MUST flow through this function — no cross-module writes.
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
/// Convention: `treasury_{app_id}_db`
/// The base URL is expected to end without a database name (or with a placeholder).
/// Example: base_url = "postgres://user:pass@localhost" → returns URL for treasury_acme_db
pub fn database_url_for_app(base_url: &str, app_id: &str) -> String {
    // Sanitize app_id: lowercase alphanumeric + underscore only
    let safe_id: String = app_id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect::<String>()
        .to_lowercase()
        .replace('-', "_");

    format!("{}/treasury_{}_db", base_url.trim_end_matches('/'), safe_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_url_for_app() {
        let url = database_url_for_app("postgres://user:pass@localhost", "acme");
        assert_eq!(url, "postgres://user:pass@localhost/treasury_acme_db");
    }

    #[test]
    fn test_database_url_sanitizes_app_id() {
        let url = database_url_for_app("postgres://user:pass@localhost", "my-App.Co");
        assert_eq!(url, "postgres://user:pass@localhost/treasury_my_app_co_db");
    }

    #[test]
    fn test_database_url_trailing_slash() {
        let url = database_url_for_app("postgres://user:pass@localhost/", "test");
        assert_eq!(url, "postgres://user:pass@localhost/treasury_test_db");
    }

    #[test]
    fn test_database_url_alphanumeric_preserved() {
        let url = database_url_for_app("postgres://user:pass@localhost", "tenant123");
        assert_eq!(url, "postgres://user:pass@localhost/treasury_tenant123_db");
    }
}
