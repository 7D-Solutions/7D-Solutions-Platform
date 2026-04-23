//! Encrypted storage for per-tenant QBO webhook verifier tokens.
//!
//! The caller is responsible for loading INTEGRATIONS_SECRETS_KEY from env
//! and confirming it is exactly 32 bytes before calling any function here.

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use sqlx::PgPool;
use thiserror::Error;

const NONCE_LEN: usize = 12;

#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("ciphertext too short to contain nonce")]
    InvalidCiphertext,
    #[error("decryption failed")]
    DecryptionFailed,
    #[error("decrypted bytes are not valid UTF-8")]
    InvalidUtf8,
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Encrypts `plaintext` with AES-256-GCM using `key`.
///
/// Output layout: 12-byte random nonce || GCM ciphertext+tag.
pub fn encrypt_token(key: &[u8; 32], plaintext: &str) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("key is 32 bytes");
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .expect("AES-GCM encryption is infallible for valid inputs");
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend(ct);
    out
}

/// Decrypts bytes produced by `encrypt_token`.
pub fn decrypt_token(key: &[u8; 32], ciphertext: &[u8]) -> Result<String, SecretStoreError> {
    if ciphertext.len() < NONCE_LEN {
        return Err(SecretStoreError::InvalidCiphertext);
    }
    let (nonce_bytes, ct) = ciphertext.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(key).expect("key is 32 bytes");
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ct)
        .map_err(|_| SecretStoreError::DecryptionFailed)?;
    String::from_utf8(plaintext).map_err(|_| SecretStoreError::InvalidUtf8)
}

/// Upserts an encrypted verifier token for `(app_id, realm_id)`.
///
/// Pass `realm_id = ""` for the app-wide fallback row.
pub async fn upsert_token(
    pool: &PgPool,
    app_id: &str,
    realm_id: &str,
    plaintext: &str,
    key: &[u8; 32],
) -> Result<(), SecretStoreError> {
    let token_enc = encrypt_token(key, plaintext);
    sqlx::query(
        r#"
        INSERT INTO integrations_qbo_webhook_secrets (app_id, realm_id, token_enc)
        VALUES ($1, $2, $3)
        ON CONFLICT (app_id, realm_id)
        DO UPDATE SET token_enc = EXCLUDED.token_enc, configured_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(realm_id)
    .bind(&token_enc)
    .execute(pool)
    .await?;
    Ok(())
}

/// Retrieves and decrypts the verifier token for `(app_id, realm_id)`.
///
/// Lookup order:
/// 1. Exact row `(app_id, realm_id)`.
/// 2. Fallback row `(app_id, "")` if the exact row is absent.
///
/// Returns `Ok(None)` when neither row exists.
pub async fn get_token(
    pool: &PgPool,
    app_id: &str,
    realm_id: &str,
    key: &[u8; 32],
) -> Result<Option<String>, SecretStoreError> {
    let row: Option<(Vec<u8>,)> = sqlx::query_as(
        r#"
        SELECT token_enc
        FROM integrations_qbo_webhook_secrets
        WHERE app_id = $1 AND realm_id = $2
        "#,
    )
    .bind(app_id)
    .bind(realm_id)
    .fetch_optional(pool)
    .await?;

    let enc = match row {
        Some((enc,)) => enc,
        None => {
            // Try the app-wide fallback row only when realm_id is non-empty.
            if realm_id.is_empty() {
                return Ok(None);
            }
            let fallback: Option<(Vec<u8>,)> = sqlx::query_as(
                r#"
                SELECT token_enc
                FROM integrations_qbo_webhook_secrets
                WHERE app_id = $1 AND realm_id = ''
                "#,
            )
            .bind(app_id)
            .fetch_optional(pool)
            .await?;
            match fallback {
                Some((enc,)) => enc,
                None => return Ok(None),
            }
        }
    };

    decrypt_token(key, &enc).map(Some)
}

/// Upserts encrypted carrier credentials for `(app_id, carrier_type)`.
pub async fn upsert_carrier_creds(
    pool: &PgPool,
    app_id: &str,
    carrier_type: &str,
    creds_json_str: &str,
    key: &[u8; 32],
) -> Result<(), SecretStoreError> {
    let creds_enc = encrypt_token(key, creds_json_str);
    sqlx::query(
        r#"
        INSERT INTO integrations_carrier_credentials (app_id, carrier_type, creds_enc)
        VALUES ($1, $2, $3)
        ON CONFLICT (app_id, carrier_type)
        DO UPDATE SET creds_enc = EXCLUDED.creds_enc, configured_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(carrier_type)
    .bind(&creds_enc)
    .execute(pool)
    .await?;
    Ok(())
}

/// Retrieves and decrypts carrier credentials for `(app_id, carrier_type)`.
///
/// Returns `Ok(None)` if no row exists (caller should fall back to connector_configs).
pub async fn get_carrier_creds(
    pool: &PgPool,
    app_id: &str,
    carrier_type: &str,
    key: &[u8; 32],
) -> Result<Option<String>, SecretStoreError> {
    let row: Option<(Vec<u8>,)> = sqlx::query_as(
        r#"
        SELECT creds_enc FROM integrations_carrier_credentials
        WHERE app_id = $1 AND carrier_type = $2
        "#,
    )
    .bind(app_id)
    .bind(carrier_type)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((enc,)) => decrypt_token(key, &enc).map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn test_key() -> [u8; 32] {
        [0x42u8; 32]
    }

    fn test_db_url() -> String {
        std::env::var("INTEGRATIONS_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| {
                "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db"
                    .to_string()
            })
    }

    async fn test_pool() -> PgPool {
        let pool = sqlx::PgPool::connect(&test_db_url())
            .await
            .expect("connect to integrations test db");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    async fn cleanup(pool: &PgPool, app_id: &str) {
        sqlx::query("DELETE FROM integrations_qbo_webhook_secrets WHERE app_id = $1")
            .bind(app_id)
            .execute(pool)
            .await
            .ok();
    }

    #[test]
    fn round_trip_encrypt_decrypt() {
        let key = test_key();
        let plaintext = "my-secret-verifier-token";
        let ct = encrypt_token(&key, plaintext);
        let got = decrypt_token(&key, &ct).expect("decrypt");
        assert_eq!(got, plaintext);
    }

    #[test]
    fn decrypt_rejects_short_ciphertext() {
        let key = test_key();
        let result = decrypt_token(&key, &[0u8; 5]);
        assert!(matches!(result, Err(SecretStoreError::InvalidCiphertext)));
    }

    #[tokio::test]
    #[serial]
    async fn upsert_then_get_exact_realm_id() {
        let pool = test_pool().await;
        let app = "test-secret-store-exact";
        cleanup(&pool, app).await;

        let key = test_key();
        upsert_token(&pool, app, "realm-abc", "token-abc", &key)
            .await
            .expect("upsert");

        let got = get_token(&pool, app, "realm-abc", &key).await.expect("get");
        assert_eq!(got, Some("token-abc".to_string()));
    }

    #[tokio::test]
    #[serial]
    async fn fallback_row_returned_for_non_matching_realm() {
        let pool = test_pool().await;
        let app = "test-secret-store-fallback";
        cleanup(&pool, app).await;

        let key = test_key();
        // Insert the app-wide fallback row (realm_id = "").
        upsert_token(&pool, app, "", "fallback-token", &key)
            .await
            .expect("upsert fallback");

        // Request for a realm that has no dedicated row — should return fallback.
        let got = get_token(&pool, app, "realm-xyz", &key).await.expect("get");
        assert_eq!(got, Some("fallback-token".to_string()));
    }

    #[tokio::test]
    #[serial]
    async fn get_on_empty_table_returns_none() {
        let pool = test_pool().await;
        let app = "test-secret-store-empty";
        cleanup(&pool, app).await;

        let key = test_key();
        let got = get_token(&pool, app, "realm-any", &key).await.expect("get");
        assert_eq!(got, None);
    }
}
