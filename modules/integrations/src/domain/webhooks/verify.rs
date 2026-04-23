//! Webhook signature verification adapter for the integrations domain.
//!
//! Thin adapter layer that wires `platform/security` verifiers to source
//! system names. The verifier is selected at dispatch time based on the
//! `system` path parameter.
//!
//! ## Dispatch table
//!
//! | system       | verifier           | required env var                 |
//! |--------------|--------------------|------------------------------------|
//! | `stripe`     | `StripeVerifier`   | `STRIPE_WEBHOOK_SECRET`            |
//! | `github`     | `GenericHmac`      | `GITHUB_WEBHOOK_SECRET`            |
//! | `quickbooks` | `IntuitVerifier`   | per-tenant DB token (env fallback) |
//! | `internal`   | `NoopVerifier`     | —                                  |
//!
//! Unknown system names return `WebhookError::UnsupportedSystem`.

use security::{
    GenericHmacVerifier, IntuitVerifier, NoopVerifier, StripeVerifier, VerifyError, WebhookVerifier,
};
use sqlx::PgPool;
use std::collections::HashMap;

use super::models::WebhookError;

/// Verify the signature for an inbound webhook from `system`.
///
/// Looks up the appropriate verifier from environment configuration and
/// delegates to it. Called **before** any database writes.
pub fn verify_signature(
    system: &str,
    headers: &HashMap<String, String>,
    raw_body: &[u8],
) -> Result<(), WebhookError> {
    let verifier = resolve_verifier(system)?;
    verifier
        .verify(headers, raw_body)
        .map_err(|e| WebhookError::SignatureVerification(format_verify_error(&e)))
}

/// Returns the appropriate verifier for the given system.
fn resolve_verifier(system: &str) -> Result<Box<dyn WebhookVerifier>, WebhookError> {
    match system {
        "stripe" => {
            let secret = std::env::var("STRIPE_WEBHOOK_SECRET").unwrap_or_default();
            Ok(Box::new(StripeVerifier::new(&secret)))
        }
        "github" => {
            let secret = std::env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default();
            Ok(Box::new(GenericHmacVerifier::new(
                &secret,
                "x-hub-signature-256",
                Some("sha256="),
            )))
        }
        "quickbooks" => {
            let secret = std::env::var("INTUIT_WEBHOOK_VERIFIER_TOKEN").unwrap_or_default();
            Ok(Box::new(IntuitVerifier::new(&secret)))
        }
        "internal" => Ok(Box::new(NoopVerifier)),
        other => Err(WebhookError::UnsupportedSystem {
            system: other.to_string(),
        }),
    }
}

fn format_verify_error(e: &VerifyError) -> String {
    e.to_string()
}

/// QBO-specific async verification with per-tenant token lookup.
///
/// Parses the batched Intuit payload to extract the realm ID, resolves the
/// app_id from the OAuth connection table, fetches the verifier token from the
/// encrypted secret store (falling back to INTUIT_WEBHOOK_VERIFIER_TOKEN), then
/// validates the HMAC-SHA256 signature in the `intuit-signature` header.
pub async fn verify_qbo_signature(
    pool: &PgPool,
    headers: &HashMap<String, String>,
    raw_body: &[u8],
    key: &[u8; 32],
) -> Result<(), WebhookError> {
    let events: Vec<serde_json::Value> = serde_json::from_slice(raw_body)
        .map_err(|_| WebhookError::SignatureVerification("malformed payload".to_string()))?;

    if events.is_empty() {
        return Err(WebhookError::SignatureVerification(
            "empty event batch".to_string(),
        ));
    }

    let realm_id = events[0]
        .get("intuitaccountid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            WebhookError::SignatureVerification("missing intuitaccountid".to_string())
        })?;

    let row: Option<(String,)> = sqlx::query_as(
        "SELECT app_id FROM integrations_oauth_connections \
         WHERE realm_id = $1 AND provider = 'quickbooks' AND connection_status != 'disconnected'",
    )
    .bind(realm_id)
    .fetch_optional(pool)
    .await
    .map_err(WebhookError::Database)?;

    let (app_id,) = row.ok_or_else(|| {
        WebhookError::SignatureVerification("no connection for realm".to_string())
    })?;

    let token_opt = crate::domain::webhooks::secret_store::get_token(pool, &app_id, realm_id, key)
        .await
        .map_err(|e| WebhookError::SignatureVerification(format!("token store error: {}", e)))?;

    let token = match token_opt {
        Some(t) => t,
        None => match std::env::var("INTUIT_WEBHOOK_VERIFIER_TOKEN")
            .ok()
            .filter(|s| !s.is_empty())
        {
            Some(env_token) => {
                tracing::warn!(
                    realm_id,
                    app_id = %app_id,
                    "QBO verifier token resolved from env var (deprecated — use admin API to store per-tenant token)"
                );
                env_token
            }
            None => {
                return Err(WebhookError::SignatureVerification(
                    "no verifier token configured".to_string(),
                ));
            }
        },
    };

    IntuitVerifier::new(&token)
        .verify(headers, raw_body)
        .map_err(|e: VerifyError| WebhookError::SignatureVerification(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use hmac::{Hmac, Mac};
    use serial_test::serial;
    use sha2::Sha256;

    const TEST_KEY: [u8; 32] = [0x42u8; 32];

    fn intuit_sig(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC");
        mac.update(body);
        STANDARD.encode(mac.finalize().into_bytes())
    }

    fn intuit_headers(sig: &str) -> HashMap<String, String> {
        HashMap::from([("intuit-signature".to_string(), sig.to_string())])
    }

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db"
                .to_string()
        })
    }

    async fn test_pool() -> sqlx::PgPool {
        let pool = sqlx::PgPool::connect(&test_db_url())
            .await
            .expect("connect to integrations test db");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    async fn seed_connection(pool: &sqlx::PgPool, app_id: &str, realm_id: &str) {
        sqlx::query(
            "DELETE FROM integrations_oauth_connections \
             WHERE provider = 'quickbooks' AND realm_id = $1",
        )
        .bind(realm_id)
        .execute(pool)
        .await
        .ok();
        sqlx::query(
            "DELETE FROM integrations_oauth_connections \
             WHERE app_id = $1 AND provider = 'quickbooks'",
        )
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
        sqlx::query(
            "INSERT INTO integrations_oauth_connections \
             (app_id, provider, realm_id, access_token, refresh_token, \
              access_token_expires_at, refresh_token_expires_at, scopes_granted, connection_status) \
             VALUES ($1, 'quickbooks', $2, 't'::bytea, 't'::bytea, \
                     NOW() + INTERVAL '1 hour', NOW() + INTERVAL '90 days', \
                     'accounting', 'connected')",
        )
        .bind(app_id)
        .bind(realm_id)
        .execute(pool)
        .await
        .expect("seed oauth connection");
    }

    async fn cleanup_connection(pool: &sqlx::PgPool, realm_id: &str) {
        sqlx::query(
            "DELETE FROM integrations_oauth_connections \
             WHERE provider = 'quickbooks' AND realm_id = $1",
        )
        .bind(realm_id)
        .execute(pool)
        .await
        .ok();
    }

    async fn cleanup_secrets(pool: &sqlx::PgPool, app_id: &str) {
        sqlx::query("DELETE FROM integrations_qbo_webhook_secrets WHERE app_id = $1")
            .bind(app_id)
            .execute(pool)
            .await
            .ok();
    }

    fn make_body(realm_id: &str) -> Vec<u8> {
        serde_json::json!([{
            "id": "evt-verify-test",
            "type": "qbo.invoice.created.v1",
            "time": "2026-04-22T00:00:00Z",
            "intuitentityid": "42",
            "intuitaccountid": realm_id,
            "data": {}
        }])
        .to_string()
        .into_bytes()
    }

    #[tokio::test]
    #[serial]
    async fn verify_qbo_token_from_db_correct_hmac() {
        let pool = test_pool().await;
        let realm_id = "vqbo-realm-db-correct";
        let app_id = "vqbo-app-db-correct";
        let token = "correct-db-token";

        seed_connection(&pool, app_id, realm_id).await;
        cleanup_secrets(&pool, app_id).await;
        crate::domain::webhooks::secret_store::upsert_token(
            &pool, app_id, realm_id, token, &TEST_KEY,
        )
        .await
        .expect("upsert token");

        let body = make_body(realm_id);
        let sig = intuit_sig(token, &body);
        let result = verify_qbo_signature(&pool, &intuit_headers(&sig), &body, &TEST_KEY).await;
        assert!(
            result.is_ok(),
            "correct HMAC with DB token must pass: {:?}",
            result
        );

        cleanup_secrets(&pool, app_id).await;
        cleanup_connection(&pool, realm_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn verify_qbo_token_from_db_wrong_hmac() {
        let pool = test_pool().await;
        let realm_id = "vqbo-realm-db-wrong";
        let app_id = "vqbo-app-db-wrong";
        let token = "correct-db-token";

        seed_connection(&pool, app_id, realm_id).await;
        cleanup_secrets(&pool, app_id).await;
        crate::domain::webhooks::secret_store::upsert_token(
            &pool, app_id, realm_id, token, &TEST_KEY,
        )
        .await
        .expect("upsert token");

        let body = make_body(realm_id);
        let sig = intuit_sig("wrong-token", &body);
        let result = verify_qbo_signature(&pool, &intuit_headers(&sig), &body, &TEST_KEY).await;
        assert!(
            matches!(result, Err(WebhookError::SignatureVerification(_))),
            "wrong HMAC must fail with SignatureVerification: {:?}",
            result
        );

        cleanup_secrets(&pool, app_id).await;
        cleanup_connection(&pool, realm_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn verify_qbo_fallback_to_env_var() {
        let pool = test_pool().await;
        let realm_id = "vqbo-realm-env-fallback";
        let app_id = "vqbo-app-env-fallback";
        let env_token = "env-fallback-token";

        seed_connection(&pool, app_id, realm_id).await;
        cleanup_secrets(&pool, app_id).await;

        let body = make_body(realm_id);
        let sig = intuit_sig(env_token, &body);

        std::env::set_var("INTUIT_WEBHOOK_VERIFIER_TOKEN", env_token);
        let result = verify_qbo_signature(&pool, &intuit_headers(&sig), &body, &TEST_KEY).await;
        std::env::remove_var("INTUIT_WEBHOOK_VERIFIER_TOKEN");

        assert!(
            result.is_ok(),
            "env var fallback with correct HMAC must pass: {:?}",
            result
        );

        cleanup_connection(&pool, realm_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn verify_qbo_no_token_anywhere() {
        let pool = test_pool().await;
        let realm_id = "vqbo-realm-no-token";
        let app_id = "vqbo-app-no-token";

        seed_connection(&pool, app_id, realm_id).await;
        cleanup_secrets(&pool, app_id).await;

        let body = make_body(realm_id);
        std::env::remove_var("INTUIT_WEBHOOK_VERIFIER_TOKEN");
        let result = verify_qbo_signature(&pool, &intuit_headers("any"), &body, &TEST_KEY).await;

        assert!(
            matches!(result, Err(WebhookError::SignatureVerification(ref msg)) if msg.contains("no verifier token")),
            "no token anywhere must return 'no verifier token configured': {:?}",
            result
        );

        cleanup_connection(&pool, realm_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn verify_qbo_unknown_realm() {
        let pool = test_pool().await;
        let realm_id = "vqbo-unknown-realm-zzz";

        cleanup_connection(&pool, realm_id).await;

        let body = make_body(realm_id);
        let result = verify_qbo_signature(&pool, &intuit_headers("any"), &body, &TEST_KEY).await;
        assert!(
            matches!(result, Err(WebhookError::SignatureVerification(ref msg)) if msg.contains("no connection for realm")),
            "unknown realm must return 'no connection for realm': {:?}",
            result
        );
    }

    #[tokio::test]
    async fn verify_qbo_empty_batch() {
        let pool = test_pool().await;
        let result = verify_qbo_signature(&pool, &intuit_headers("any"), b"[]", &TEST_KEY).await;
        assert!(
            matches!(result, Err(WebhookError::SignatureVerification(ref msg)) if msg.contains("empty event batch")),
            "empty batch must return 'empty event batch': {:?}",
            result
        );
    }

    #[tokio::test]
    async fn verify_qbo_malformed_payload() {
        let pool = test_pool().await;
        let result =
            verify_qbo_signature(&pool, &intuit_headers("any"), b"not-json{{{", &TEST_KEY).await;
        assert!(
            matches!(result, Err(WebhookError::SignatureVerification(ref msg)) if msg.contains("malformed payload")),
            "non-JSON must return 'malformed payload': {:?}",
            result
        );
    }

    #[test]
    fn test_internal_system_always_passes() {
        let headers = HashMap::new();
        let result = verify_signature("internal", &headers, b"{}");
        assert!(result.is_ok());
    }

    #[test]
    fn test_unknown_system_rejected() {
        let headers = HashMap::new();
        let result = verify_signature("acme-payments", &headers, b"{}");
        assert!(matches!(
            result,
            Err(WebhookError::UnsupportedSystem { .. })
        ));
    }
}
