//! Integration tests for service-to-service HMAC authentication.
//!
//! Tests generate and verify real HMAC-SHA256 tokens — no mocks.
//!
//! Because `sign_claims()` reads SERVICE_AUTH_SECRET from the process env,
//! and Rust tests run in parallel within a single process, we serialise
//! all env-mutating tests behind a global mutex.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use security::claims::{ActorType, JwtVerifier};
use security::service_auth::{
    generate_service_token, get_service_token, verify_service_token, ServiceAuthClaims,
    ServiceAuthError,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct RsaTestKeys {
    private_pem: String,
    public_pem: String,
}

fn generate_rsa_keys() -> RsaTestKeys {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key generation");
    let pub_key = priv_key.to_public_key();
    let private_pem = priv_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("private PEM")
        .to_string();
    let public_pem = pub_key
        .to_public_key_pem(LineEnding::LF)
        .expect("public PEM");
    RsaTestKeys {
        private_pem,
        public_pem,
    }
}

struct EnvGuard {
    previous: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set(entries: &[(&'static str, Option<&str>)]) -> Self {
        let mut previous = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            previous.push((*key, std::env::var(key).ok()));
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        Self { previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.previous.iter().rev() {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

#[derive(Clone, Default)]
struct TestWriter(Arc<Mutex<Vec<u8>>>);

struct TestWriterHandle(Arc<Mutex<Vec<u8>>>);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TestWriter {
    type Writer = TestWriterHandle;

    fn make_writer(&'a self) -> Self::Writer {
        TestWriterHandle(self.0.clone())
    }
}

impl Write for TestWriterHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Run `f` with SERVICE_AUTH_SECRET set to `secret`, then restore.
fn with_secret<F, R>(secret: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("SERVICE_AUTH_SECRET", secret);
    let result = f();
    std::env::remove_var("SERVICE_AUTH_SECRET");
    result
}

/// Helper to manually sign claims for expiration / future-dated tests.
fn manual_sign(claims_b64: &str, secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(claims_b64.as_bytes());
    let sig = mac.finalize().into_bytes();
    URL_SAFE_NO_PAD.encode(&sig)
}

// ── Generate + verify round-trip ─────────────────────────────────────────

#[test]
fn generate_and_verify_roundtrip() {
    with_secret("test-secret-roundtrip", || {
        let token = generate_service_token("billing-service", None).unwrap();
        let claims = verify_service_token(&token).unwrap();

        assert_eq!(claims.service_name, "billing-service");
        assert!(claims.expires_at > claims.issued_at);
    });
}

#[test]
fn default_validity_is_15_minutes() {
    with_secret("test-secret-validity", || {
        let token = generate_service_token("gl-service", None).unwrap();
        let claims = verify_service_token(&token).unwrap();

        let validity = claims.expires_at - claims.issued_at;
        assert_eq!(validity, 15 * 60);
    });
}

#[test]
fn custom_validity_applied() {
    with_secret("test-secret-custom", || {
        let token = generate_service_token("ar-service", Some(30)).unwrap();
        let claims = verify_service_token(&token).unwrap();

        let validity = claims.expires_at - claims.issued_at;
        assert_eq!(validity, 30 * 60);
    });
}

// ── Token format ─────────────────────────────────────────────────────────

#[test]
fn token_has_two_dot_separated_parts() {
    with_secret("test-secret-format", || {
        let token = generate_service_token("test", None).unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 2, "Token should be <claims>.<signature>");
    });
}

#[test]
fn claims_part_is_valid_base64_json() {
    with_secret("test-secret-b64", || {
        let token = generate_service_token("test-svc", None).unwrap();
        let claims_b64 = token.split('.').next().unwrap();
        let json_bytes = URL_SAFE_NO_PAD.decode(claims_b64).unwrap();
        let claims: ServiceAuthClaims = serde_json::from_slice(&json_bytes).unwrap();
        assert_eq!(claims.service_name, "test-svc");
    });
}

// ── Invalid tokens ───────────────────────────────────────────────────────

#[test]
fn invalid_format_no_dot() {
    with_secret("test-secret-invalid", || {
        let result = verify_service_token("nodottoken");
        assert!(matches!(result, Err(ServiceAuthError::InvalidFormat)));
    });
}

#[test]
fn invalid_format_too_many_dots() {
    with_secret("test-secret-dots", || {
        let result = verify_service_token("a.b.c");
        assert!(matches!(result, Err(ServiceAuthError::InvalidFormat)));
    });
}

#[test]
fn tampered_claims_rejected() {
    with_secret("test-secret-tamper", || {
        let token = generate_service_token("legit", None).unwrap();
        let parts: Vec<&str> = token.split('.').collect();

        let tampered_claims = ServiceAuthClaims {
            service_name: "evil".to_string(),
            issued_at: Utc::now().timestamp(),
            expires_at: Utc::now().timestamp() + 9999,
        };
        let tampered_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_string(&tampered_claims).unwrap());
        let tampered_token = format!("{}.{}", tampered_b64, parts[1]);

        let result = verify_service_token(&tampered_token);
        assert!(matches!(result, Err(ServiceAuthError::InvalidSignature)));
    });
}

#[test]
fn wrong_secret_rejects_token() {
    let token = with_secret("secret-A", || generate_service_token("svc", None).unwrap());

    with_secret("secret-B", || {
        let result = verify_service_token(&token);
        assert!(matches!(result, Err(ServiceAuthError::InvalidSignature)));
    });
}

// ── Expiration ───────────────────────────────────────────────────────────

#[test]
fn expired_token_rejected() {
    let secret = "test-secret-expired";
    with_secret(secret, || {
        let now = Utc::now().timestamp();
        let claims = ServiceAuthClaims {
            service_name: "old-svc".to_string(),
            issued_at: now - 7200,
            expires_at: now - 3600,
        };

        let claims_json = serde_json::to_string(&claims).unwrap();
        let claims_b64 = URL_SAFE_NO_PAD.encode(claims_json.as_bytes());
        let sig_b64 = manual_sign(&claims_b64, secret);
        let token = format!("{}.{}", claims_b64, sig_b64);

        let result = verify_service_token(&token);
        assert!(matches!(result, Err(ServiceAuthError::TokenExpired)));
    });
}

#[test]
fn future_issued_token_rejected() {
    let secret = "test-secret-future";
    with_secret(secret, || {
        let now = Utc::now().timestamp();
        let claims = ServiceAuthClaims {
            service_name: "future-svc".to_string(),
            issued_at: now + 300,
            expires_at: now + 1200,
        };

        let claims_json = serde_json::to_string(&claims).unwrap();
        let claims_b64 = URL_SAFE_NO_PAD.encode(claims_json.as_bytes());
        let sig_b64 = manual_sign(&claims_b64, secret);
        let token = format!("{}.{}", claims_b64, sig_b64);

        let result = verify_service_token(&token);
        assert!(matches!(result, Err(ServiceAuthError::TokenNotYetValid)));
    });
}

// ── Missing secret ───────────────────────────────────────────────────────

#[test]
fn missing_secret_returns_error() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::remove_var("SERVICE_AUTH_SECRET");
    let result = generate_service_token("svc", None);
    assert!(matches!(result, Err(ServiceAuthError::MissingSigningKey)));
}

// ── Service name preserved ───────────────────────────────────────────────

#[test]
fn various_service_names_preserved() {
    with_secret("test-secret-names", || {
        for name in ["tenantctl", "ar-service", "gl-service", "inventory-rs"] {
            let token = generate_service_token(name, None).unwrap();
            let claims = verify_service_token(&token).unwrap();
            assert_eq!(claims.service_name, name);
        }
    });
}

#[test]
fn test_rsa_path_with_key_set_succeeds() {
    let _guard = ENV_LOCK.lock().unwrap();
    let keys = generate_rsa_keys();
    let _env = EnvGuard::set(&[
        ("ENV", Some("production")),
        ("JWT_PRIVATE_KEY_PEM", Some(&keys.private_pem)),
        ("SERVICE_TOKEN", None),
        ("SERVICE_NAME", Some("billing-service")),
    ]);

    let token = get_service_token().expect("RSA path should succeed in production");
    let verifier = JwtVerifier::from_public_pem(&keys.public_pem).expect("verifier");
    let claims = verifier.verify(&token).expect("token should verify");

    assert_eq!(claims.user_id, uuid::Uuid::nil());
    assert_eq!(claims.tenant_id, uuid::Uuid::nil());
    assert_eq!(claims.actor_type, ActorType::Service);
}

#[test]
fn test_no_key_prod_errors() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env = EnvGuard::set(&[
        ("ENV", Some("production")),
        ("JWT_PRIVATE_KEY_PEM", None),
        ("SERVICE_TOKEN", None),
    ]);

    let result = get_service_token();
    assert!(matches!(result, Err(ServiceAuthError::MissingSigningKey)));
}

#[test]
fn test_invalid_key_prod_errors() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env = EnvGuard::set(&[
        ("ENV", Some("production")),
        ("JWT_PRIVATE_KEY_PEM", Some("not-a-valid-rsa-key")),
        ("SERVICE_TOKEN", None),
    ]);

    let result = get_service_token();
    assert!(matches!(result, Err(ServiceAuthError::MissingSigningKey)));
}

#[test]
fn test_no_key_env_unset_errors() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env = EnvGuard::set(&[
        ("ENV", None),
        ("JWT_PRIVATE_KEY_PEM", None),
        ("SERVICE_TOKEN", None),
    ]);

    let result = get_service_token();
    assert!(matches!(result, Err(ServiceAuthError::MissingSigningKey)));
}

#[test]
fn test_no_key_staging_errors() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env = EnvGuard::set(&[
        ("ENV", Some("staging")),
        ("JWT_PRIVATE_KEY_PEM", None),
        ("SERVICE_TOKEN", None),
    ]);

    let result = get_service_token();
    assert!(matches!(result, Err(ServiceAuthError::MissingSigningKey)));
}

#[test]
fn test_no_key_dev_succeeds_with_warn() {
    let _guard = ENV_LOCK.lock().unwrap();
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(TestWriter(buffer.clone()))
        .finish();

    let _env = EnvGuard::set(&[
        ("ENV", Some("development")),
        ("JWT_PRIVATE_KEY_PEM", None),
        ("SERVICE_TOKEN", None),
        ("SERVICE_AUTH_SECRET", Some("dev-secret")),
        ("SERVICE_NAME", Some("billing-service")),
    ]);

    let token = tracing::subscriber::with_default(subscriber, || {
        get_service_token().expect("development fallback should succeed")
    });

    let claims = verify_service_token(&token).expect("dev fallback token should verify");
    assert_eq!(claims.service_name, "billing-service");

    let logs = String::from_utf8(buffer.lock().unwrap().clone()).expect("warn log utf8");
    assert!(
        logs.contains("development") || logs.contains("legacy HMAC fallback"),
        "expected development fallback warning, got: {logs}"
    );
}
