use axum::{routing::get, Json};
use std::sync::Arc;
use utoipa::OpenApi;

use integrations_rs::{
    domain::connectors::{
        ConfigField, ConfigFieldType, ConnectorCapabilities, ConnectorConfig,
        RegisterConnectorRequest, RunTestActionRequest, TestActionResult,
    },
    domain::external_refs::{CreateExternalRefRequest, ExternalRef, UpdateExternalRefRequest},
    domain::oauth::{refresh, ConnectionStatus, OAuthConnectionInfo},
    http,
    http::qbo_invoice::{UpdateInvoiceRequest, UpdateInvoiceResponse},
    metrics, AppState,
};
use platform_http_contracts::{ApiError, FieldError, PaginatedResponse, PaginationMeta};
use platform_sdk::ModuleBuilder;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Integrations Service",
        version = "2.3.0",
        description = "External system connectors, webhook routing, OAuth connection management, \
                        and reference linking.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims \
                        (not headers). Permissions: `integrations.read` for queries, \
                        `integrations.mutate` for writes.\n\n\
                        **Webhooks:** Inbound webhooks (Stripe, GitHub, QuickBooks) are \
                        unauthenticated and gated by HMAC-SHA256 signature verification.",
    ),
    paths(
        integrations_rs::http::external_refs::create_external_ref,
        integrations_rs::http::external_refs::list_by_entity,
        integrations_rs::http::external_refs::get_by_external,
        integrations_rs::http::external_refs::get_external_ref,
        integrations_rs::http::external_refs::update_external_ref,
        integrations_rs::http::external_refs::delete_external_ref,
        integrations_rs::http::connectors::list_connector_types,
        integrations_rs::http::connectors::register_connector,
        integrations_rs::http::connectors::list_connectors,
        integrations_rs::http::connectors::get_connector,
        integrations_rs::http::connectors::run_connector_test,
        integrations_rs::http::oauth::connect,
        integrations_rs::http::oauth::callback,
        integrations_rs::http::oauth::status,
        integrations_rs::http::oauth::disconnect,
        integrations_rs::http::oauth::import_tokens,
        integrations_rs::http::webhooks::inbound_webhook,
        integrations_rs::http::qbo_invoice::update_invoice,
    ),
    components(schemas(
        ExternalRef, CreateExternalRefRequest, UpdateExternalRefRequest,
        ConnectorConfig, ConnectorCapabilities, ConfigField, ConfigFieldType,
        RegisterConnectorRequest, RunTestActionRequest, TestActionResult,
        OAuthConnectionInfo, ConnectionStatus,
        integrations_rs::http::oauth::ImportTokensRequest,
        UpdateInvoiceRequest, UpdateInvoiceResponse,
        ApiError, FieldError,
        PaginatedResponse<ExternalRef>, PaginatedResponse<ConnectorConfig>, PaginationMeta,
    )),
    security(("bearer" = [])),
    modifiers(&SecurityAddon),
)]
struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::HttpBuilder::new()
                    .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

/// Warn at startup when INTUIT_WEBHOOK_VERIFIER_TOKEN is absent.
///
/// No longer fatal: per-tenant tokens stored via the admin API are the primary
/// mechanism. The env var remains supported as a global fallback for operators
/// who have not yet migrated. Missing it is now a deployment hint, not a crash.
fn validate_webhook_env() {
    let profile = std::env::var("APP_PROFILE").unwrap_or_default();
    if profile == "dev-local" {
        return;
    }
    let token = std::env::var("INTUIT_WEBHOOK_VERIFIER_TOKEN").unwrap_or_default();
    if token.is_empty() {
        tracing::warn!(
            "INTUIT_WEBHOOK_VERIFIER_TOKEN is not set — QBO webhook verification will use \
             per-tenant DB tokens only. Set this env var if a global fallback token is needed."
        );
    }
}

/// Load the AES-256-GCM secrets key.
///
/// Lookup order:
/// 1. Google Secret Manager (if GOOGLE_APPLICATION_CREDENTIALS and GCP_PROJECT_ID are both set).
///    GCP_SECRET_NAME defaults to "integrations-secrets-key".
///    Any failure in this step logs a warning and falls through to step 2.
/// 2. INTEGRATIONS_SECRETS_KEY env var (hex or base64 → 32 bytes). Fatal if absent or invalid.
async fn load_secrets_key() -> [u8; 32] {
    let gcp_creds = std::env::var("GOOGLE_APPLICATION_CREDENTIALS").unwrap_or_default();
    let gcp_project = std::env::var("GCP_PROJECT_ID").unwrap_or_default();

    if !gcp_creds.is_empty() && !gcp_project.is_empty() {
        let secret_name = std::env::var("GCP_SECRET_NAME")
            .unwrap_or_else(|_| "integrations-secrets-key".to_string());

        match fetch_key_from_gcp(&gcp_creds, &gcp_project, &secret_name).await {
            Ok(key) => return key,
            Err(e) => tracing::warn!("GCP fetch failed: {}, falling back to env var", e),
        }
    }

    load_key_from_env()
}

/// Fetch the 32-byte key from Google Secret Manager.
async fn fetch_key_from_gcp(
    creds_path: &str,
    project_id: &str,
    secret_name: &str,
) -> Result<[u8; 32], String> {
    let sa_key = yup_oauth2::read_service_account_key(creds_path)
        .await
        .map_err(|e| format!("read service account key: {e}"))?;

    let auth = yup_oauth2::ServiceAccountAuthenticator::builder(sa_key)
        .build()
        .await
        .map_err(|e| format!("build authenticator: {e}"))?;

    let token = auth
        .token(&["https://www.googleapis.com/auth/cloud-platform"])
        .await
        .map_err(|e| format!("get token: {e}"))?;

    let url = format!(
        "https://secretmanager.googleapis.com/v1/projects/{project_id}/secrets/{secret_name}/versions/latest:access"
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(token.token().ok_or("token has no value")?)
        .send()
        .await
        .map_err(|e| format!("HTTP request: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Secret Manager returned HTTP {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("parse response: {e}"))?;

    let b64 = body["payload"]["data"]
        .as_str()
        .ok_or("missing payload.data field")?;

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode: {e}"))?;

    if bytes.len() != 32 {
        return Err(format!(
            "GCP secret decoded to {} bytes, expected 32, falling back to env var",
            bytes.len()
        ));
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Avalara AvaTax credentials loaded from Secret Manager or env-var fallback.
///
/// Returns `None` when Avalara is not configured for this deployment — Avalara
/// is opt-in per tenant (tenants on external_accounting_software tax source do
/// not need it). Consumers (AR's AvalaraProvider) use `Option` to decide
/// whether to instantiate the provider at all.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct AvalaraCredentials {
    pub account_id: String,
    pub license_key: String,
}

/// Load Avalara credentials with the same GCP-first, env-var fallback pattern
/// as `load_secrets_key`. Returns `None` if neither source yields both values.
///
/// Lookup order per field:
/// 1. Google Secret Manager (if `GOOGLE_APPLICATION_CREDENTIALS` + `GCP_PROJECT_ID` set)
///    under `avalara-sandbox-account-id` / `avalara-sandbox-license-key`.
///    Any failure logs a warning and falls through to step 2.
/// 2. `AVALARA_ACCOUNT_ID` / `AVALARA_LICENSE_KEY` env vars.
#[allow(dead_code)]
pub async fn load_avalara_credentials() -> Option<AvalaraCredentials> {
    let gcp_creds = std::env::var("GOOGLE_APPLICATION_CREDENTIALS").unwrap_or_default();
    let gcp_project = std::env::var("GCP_PROJECT_ID").unwrap_or_default();

    if !gcp_creds.is_empty() && !gcp_project.is_empty() {
        let id_secret = std::env::var("AVALARA_ACCOUNT_ID_SECRET_NAME")
            .unwrap_or_else(|_| "avalara-sandbox-account-id".to_string());
        let key_secret = std::env::var("AVALARA_LICENSE_KEY_SECRET_NAME")
            .unwrap_or_else(|_| "avalara-sandbox-license-key".to_string());

        let id = fetch_string_from_gcp(&gcp_creds, &gcp_project, &id_secret).await;
        let key = fetch_string_from_gcp(&gcp_creds, &gcp_project, &key_secret).await;

        if let (Ok(a), Ok(l)) = (id, key) {
            if !a.is_empty() && !l.is_empty() {
                return Some(AvalaraCredentials {
                    account_id: a,
                    license_key: l,
                });
            }
        } else {
            tracing::warn!("GCP fetch for Avalara credentials failed, falling back to env vars");
        }
    }

    let id = std::env::var("AVALARA_ACCOUNT_ID").unwrap_or_default();
    let key = std::env::var("AVALARA_LICENSE_KEY").unwrap_or_default();
    if id.is_empty() || key.is_empty() {
        return None;
    }
    Some(AvalaraCredentials {
        account_id: id,
        license_key: key,
    })
}

/// Fetch a UTF-8 string secret from Google Secret Manager. Used for Avalara
/// credentials; `fetch_key_from_gcp` handles the 32-byte binary case.
#[allow(dead_code)]
async fn fetch_string_from_gcp(
    creds_path: &str,
    project_id: &str,
    secret_name: &str,
) -> Result<String, String> {
    let sa_key = yup_oauth2::read_service_account_key(creds_path)
        .await
        .map_err(|e| format!("read service account key: {e}"))?;

    let auth = yup_oauth2::ServiceAccountAuthenticator::builder(sa_key)
        .build()
        .await
        .map_err(|e| format!("build authenticator: {e}"))?;

    let token = auth
        .token(&["https://www.googleapis.com/auth/cloud-platform"])
        .await
        .map_err(|e| format!("get token: {e}"))?;

    let url = format!(
        "https://secretmanager.googleapis.com/v1/projects/{project_id}/secrets/{secret_name}/versions/latest:access"
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(token.token().ok_or("token has no value")?)
        .send()
        .await
        .map_err(|e| format!("HTTP request: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Secret Manager returned HTTP {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("parse response: {e}"))?;

    let b64 = body["payload"]["data"]
        .as_str()
        .ok_or("missing payload.data field")?;

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode: {e}"))?;

    String::from_utf8(bytes).map_err(|e| format!("utf8 decode: {e}"))
}

/// Decode INTEGRATIONS_SECRETS_KEY env var (hex or base64) to exactly 32 bytes.
/// Panics with an actionable message if absent, undecodable, or wrong length.
fn load_key_from_env() -> [u8; 32] {
    let raw = std::env::var("INTEGRATIONS_SECRETS_KEY").unwrap_or_default();
    if raw.is_empty() {
        panic!(
            "Startup validation failed: INTEGRATIONS_SECRETS_KEY is not set or empty. \
             The service cannot encrypt or decrypt QBO webhook verifier tokens without it. \
             Set INTEGRATIONS_SECRETS_KEY to a 32-byte value encoded as hex (64 hex chars) \
             or base64 (44 chars with padding)."
        );
    }
    let bytes: Vec<u8> = if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        (0..raw.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&raw[i..i + 2], 16).expect("valid hex"))
            .collect()
    } else {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(&raw)
            .unwrap_or_else(|_| {
                panic!(
                    "Startup validation failed: INTEGRATIONS_SECRETS_KEY is not valid hex or base64. \
                     Provide a 32-byte key as 64 hex characters or base64."
                )
            })
    };
    if bytes.len() != 32 {
        panic!(
            "Startup validation failed: INTEGRATIONS_SECRETS_KEY decoded to {} bytes; \
             exactly 32 bytes are required for AES-256-GCM.",
            bytes.len()
        );
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    key
}

fn validate_oauth_env() {
    integrations_rs::http::oauth_validation::validate_oauth_env_pub();
}

/// Validate the QBO env contract before any worker starts.
///
/// Called only when QBO_CLIENT_ID is present (i.e. QBO integration is enabled).
/// Panics with an actionable message listing every missing or invalid var so ops
/// never gets a silent misconfiguration.
fn validate_qbo_env() {
    const REQUIRED: &[&str] = &[
        "QBO_CLIENT_ID",
        "QBO_CLIENT_SECRET",
        "QBO_REDIRECT_URI",
        "OAUTH_ENCRYPTION_KEY",
    ];

    let missing: Vec<&str> = REQUIRED
        .iter()
        .filter(|var| std::env::var(var).map_or(true, |v| v.is_empty()))
        .copied()
        .collect();

    if !missing.is_empty() {
        panic!(
            "Startup validation failed: QBO is enabled (QBO_CLIENT_ID is set) but required \
             env vars are missing or empty: {}",
            missing.join(", ")
        );
    }

    let redirect_uri = std::env::var("QBO_REDIRECT_URI")
        .expect("QBO_REDIRECT_URI presence already validated above");
    if !redirect_uri.starts_with("https://") && !redirect_uri.starts_with("http://localhost") {
        panic!(
            "Startup validation failed: QBO_REDIRECT_URI '{}' is invalid — must start with \
             https:// (or http://localhost for dev)",
            redirect_uri
        );
    }

    // Production requires a real NATS bus — in-memory bus silently drops sync events.
    let env_name = std::env::var("ENV").unwrap_or_default();
    if env_name == "production" {
        let bus_type = std::env::var("BUS_TYPE").unwrap_or_default().to_lowercase();
        if bus_type == "inmemory" || bus_type.is_empty() {
            panic!(
                "Startup validation failed: BUS_TYPE=inmemory is not allowed in production. \
                 Set BUS_TYPE=nats and NATS_URL to a reachable NATS server. \
                 Sync events (authority changes, conflict notifications) would be silently dropped."
            );
        }
        let nats_url = std::env::var("NATS_URL").unwrap_or_default();
        if nats_url.is_empty() {
            panic!(
                "Startup validation failed: NATS_URL is required in production when QBO is enabled. \
                 Sync events cannot be delivered without a NATS connection."
            );
        }
    }
}

#[tokio::main]
async fn main() {
    let webhooks_key = load_secrets_key().await;

    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .on_startup(|pool| async move {
            let threshold_secs: i64 = std::env::var("SYNC_PULL_ORPHAN_THRESHOLD_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(600);
            if let Err(e) = integrations_rs::sync_pull_recovery::reconcile_orphan_inflight_pulls(
                &pool,
                threshold_secs,
            )
            .await
            {
                tracing::error!(error = %e, "orphan reconciliation failed at startup, continuing");
            }
            Ok::<_, platform_sdk::StartupError>(())
        })
        .routes(move |ctx| {
            validate_webhook_env();
            validate_oauth_env();
            let webhooks_key = webhooks_key;

            let bus = ctx.bus_arc().expect("Integrations requires event bus");

            // Spawn conditional background workers
            if std::env::var("QBO_CLIENT_ID").map_or(false, |v| !v.is_empty()) {
                validate_qbo_env();
                let refresher: Arc<dyn refresh::TokenRefresher> =
                    Arc::new(refresh::HttpTokenRefresher {
                        client: reqwest::Client::new(),
                        qbo_client_id: std::env::var("QBO_CLIENT_ID").unwrap_or_default(),
                        qbo_client_secret: std::env::var("QBO_CLIENT_SECRET").unwrap_or_default(),
                        qbo_token_url: std::env::var("QBO_TOKEN_URL").unwrap_or_else(|_| {
                            "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer".to_string()
                        }),
                    });
                let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
                refresh::spawn_refresh_worker(
                    ctx.pool().clone(),
                    refresher,
                    std::time::Duration::from_secs(30),
                    shutdown_rx,
                );
                tracing::info!("Integrations: OAuth refresh worker started (30s poll)");

                let (_cdc_shutdown_tx, cdc_shutdown_rx) = tokio::sync::watch::channel(false);
                let cdc_interval_secs: u64 = std::env::var("CDC_POLL_INTERVAL_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(integrations_rs::domain::qbo::cdc::DEFAULT_CDC_POLL_INTERVAL_SECS);
                integrations_rs::domain::qbo::cdc::spawn_cdc_worker(
                    ctx.pool().clone(),
                    std::time::Duration::from_secs(cdc_interval_secs),
                    cdc_shutdown_rx,
                );
                tracing::info!(
                    interval_secs = cdc_interval_secs,
                    "Integrations: QBO CDC polling worker started"
                );

                if integrations_rs::domain::qbo::outbound::legacy_consumers_enabled() {
                    let (_outbound_shutdown_tx, outbound_shutdown_rx) =
                        tokio::sync::watch::channel(false);
                    integrations_rs::domain::qbo::outbound::spawn_outbound_consumer(
                        ctx.pool().clone(),
                        bus.clone(),
                        outbound_shutdown_rx,
                    );
                    tracing::info!("Integrations: QBO outbound consumer started");

                    let (_order_ingested_shutdown_tx, order_ingested_shutdown_rx) =
                        tokio::sync::watch::channel(false);
                    integrations_rs::domain::qbo::outbound::spawn_order_ingested_consumer(
                        ctx.pool().clone(),
                        bus.clone(),
                        order_ingested_shutdown_rx,
                    );
                    tracing::info!("Integrations: QBO order-ingested consumer started");
                } else {
                    tracing::info!(
                        "Integrations: QBO legacy outbound consumers disabled \
                         (QBO_LEGACY_CONSUMERS_ENABLED != 1) — set to 1 to re-enable"
                    );
                }
            }

            integrations_rs::domain::file_jobs::ebay_fulfillment::start_ebay_fulfillment_consumer(
                bus.clone(),
                ctx.pool().clone(),
            );
            tracing::info!("Integrations: eBay fulfillment consumer started");

            tokio::spawn(
                integrations_rs::domain::sync::push_attempts::run_watchdog_task(ctx.pool().clone()),
            );
            tracing::info!("Integrations: push-attempt watchdog started (60s poll, 10min timeout)");

            let integrations_metrics = Arc::new(
                metrics::IntegrationsMetrics::new()
                    .expect("Integrations: failed to create metrics"),
            );

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: integrations_metrics,
                bus,
                webhooks_key,
            });

            http::router(app_state).route("/api/openapi.json", get(openapi_json))
        })
        .run()
        .await
        .expect("integrations module failed");
}

#[cfg(test)]
mod startup_guard_tests {
    use super::validate_webhook_env;
    use serial_test::serial;
    use std::sync::Mutex;

    // Env vars are process-global — serialize all tests that touch them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for (k, v) in vars {
            std::env::set_var(k, v);
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        for (k, _) in vars {
            std::env::remove_var(k);
        }
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    #[serial]
    fn dev_local_profile_allows_empty_token() {
        with_env(
            &[
                ("APP_PROFILE", "dev-local"),
                ("INTUIT_WEBHOOK_VERIFIER_TOKEN", ""),
            ],
            || validate_webhook_env(),
        );
    }

    #[test]
    #[serial]
    fn staging_with_token_set_passes() {
        with_env(
            &[
                ("APP_PROFILE", "staging"),
                ("INTUIT_WEBHOOK_VERIFIER_TOKEN", "a-real-token"),
            ],
            || validate_webhook_env(),
        );
    }

    #[test]
    #[serial]
    fn production_with_token_set_passes() {
        with_env(
            &[
                ("APP_PROFILE", "production"),
                ("INTUIT_WEBHOOK_VERIFIER_TOKEN", "a-real-token"),
            ],
            || validate_webhook_env(),
        );
    }

    #[test]
    #[serial]
    fn staging_without_token_no_longer_panics() {
        // env var is now optional — warn only, not fatal
        with_env(
            &[
                ("APP_PROFILE", "staging"),
                ("INTUIT_WEBHOOK_VERIFIER_TOKEN", ""),
            ],
            || validate_webhook_env(),
        );
    }

    #[test]
    #[serial]
    fn production_without_token_no_longer_panics() {
        with_env(
            &[
                ("APP_PROFILE", "production"),
                ("INTUIT_WEBHOOK_VERIFIER_TOKEN", ""),
            ],
            || validate_webhook_env(),
        );
    }

    #[test]
    #[serial]
    fn no_profile_without_token_no_longer_panics() {
        with_env(
            &[("APP_PROFILE", ""), ("INTUIT_WEBHOOK_VERIFIER_TOKEN", "")],
            || validate_webhook_env(),
        );
    }
}

#[cfg(test)]
mod secrets_key_tests {
    use super::{load_key_from_env, load_secrets_key};
    use serial_test::serial;

    // 64-char hex string = 32 bytes
    const VALID_HEX: &str = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

    fn set_no_gcp() {
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        std::env::remove_var("GCP_PROJECT_ID");
    }

    /// GCP vars absent, valid hex env var → returns key without warn.
    #[tokio::test]
    #[serial]
    async fn secrets_key_env_var_only() {
        set_no_gcp();
        std::env::set_var("INTEGRATIONS_SECRETS_KEY", VALID_HEX);
        let key = load_secrets_key().await;
        assert_eq!(key[0], 0x01);
        assert_eq!(key[31], 0x20);
        std::env::remove_var("INTEGRATIONS_SECRETS_KEY");
    }

    /// Both GCP vars empty → skip GCP, fall back to env var.
    #[tokio::test]
    #[serial]
    async fn secrets_key_gcp_vars_empty_skips() {
        std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", "");
        std::env::set_var("GCP_PROJECT_ID", "");
        std::env::set_var("INTEGRATIONS_SECRETS_KEY", VALID_HEX);
        let key = load_secrets_key().await;
        assert_eq!(key.len(), 32);
        std::env::remove_var("INTEGRATIONS_SECRETS_KEY");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        std::env::remove_var("GCP_PROJECT_ID");
    }

    /// GCP creds path does not exist → warn + fall back to env var.
    #[tokio::test]
    #[serial]
    async fn secrets_key_file_missing() {
        std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", "/nonexistent/sa.json");
        std::env::set_var("GCP_PROJECT_ID", "test-project");
        std::env::set_var("INTEGRATIONS_SECRETS_KEY", VALID_HEX);
        let key = load_secrets_key().await;
        assert_eq!(key.len(), 32);
        std::env::remove_var("INTEGRATIONS_SECRETS_KEY");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        std::env::remove_var("GCP_PROJECT_ID");
    }

    /// GCP creds file exists but contains invalid JSON → warn + fall back.
    #[tokio::test]
    #[serial]
    async fn secrets_key_file_invalid_json() {
        let path = std::env::temp_dir().join("invalid_sa_test.json");
        std::fs::write(&path, "not json").expect("write test file");
        std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", path.to_str().expect("valid path"));
        std::env::set_var("GCP_PROJECT_ID", "test-project");
        std::env::set_var("INTEGRATIONS_SECRETS_KEY", VALID_HEX);
        let key = load_secrets_key().await;
        assert_eq!(key.len(), 32);
        std::fs::remove_file(&path).ok();
        std::env::remove_var("INTEGRATIONS_SECRETS_KEY");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        std::env::remove_var("GCP_PROJECT_ID");
    }

    /// Invalid hex/base64 env var → panic.
    #[test]
    #[serial]
    #[should_panic(expected = "not valid hex or base64")]
    fn secrets_key_invalid_hex() {
        std::env::set_var("INTEGRATIONS_SECRETS_KEY", "notvalidhexorbase64!!!");
        load_key_from_env();
    }

    /// Valid base64 but decodes to wrong byte count → panic.
    #[test]
    #[serial]
    #[should_panic(expected = "decoded to 16 bytes")]
    fn secrets_key_wrong_byte_count() {
        use base64::Engine as _;
        let short = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
        std::env::set_var("INTEGRATIONS_SECRETS_KEY", short);
        load_key_from_env();
    }

    /// Env var absent → panic.
    #[test]
    #[serial]
    #[should_panic(expected = "INTEGRATIONS_SECRETS_KEY is not set or empty")]
    fn secrets_key_neither_set() {
        std::env::remove_var("INTEGRATIONS_SECRETS_KEY");
        load_key_from_env();
    }
}

#[cfg(test)]
mod avalara_credentials_tests {
    use super::load_avalara_credentials;
    use serial_test::serial;

    fn clear_avalara_env() {
        std::env::remove_var("AVALARA_ACCOUNT_ID");
        std::env::remove_var("AVALARA_LICENSE_KEY");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        std::env::remove_var("GCP_PROJECT_ID");
    }

    /// No env vars set and no GCP config → returns None (Avalara is opt-in).
    #[tokio::test]
    #[serial]
    async fn load_avalara_credentials_neither_source_returns_none() {
        clear_avalara_env();
        assert!(load_avalara_credentials().await.is_none());
    }

    /// Both env vars populated, no GCP → returns Some with the values.
    #[tokio::test]
    #[serial]
    async fn load_avalara_credentials_env_both_set_returns_some() {
        clear_avalara_env();
        std::env::set_var("AVALARA_ACCOUNT_ID", "acct-12345");
        std::env::set_var("AVALARA_LICENSE_KEY", "lic-abcdef");
        let creds = load_avalara_credentials().await.expect("expected Some");
        assert_eq!(creds.account_id, "acct-12345");
        assert_eq!(creds.license_key, "lic-abcdef");
        clear_avalara_env();
    }

    /// Only account id set, license key empty → returns None
    /// (partial config is useless; refuse to instantiate).
    #[tokio::test]
    #[serial]
    async fn load_avalara_credentials_only_account_id_returns_none() {
        clear_avalara_env();
        std::env::set_var("AVALARA_ACCOUNT_ID", "acct-12345");
        std::env::set_var("AVALARA_LICENSE_KEY", "");
        assert!(load_avalara_credentials().await.is_none());
        clear_avalara_env();
    }

    /// Only license key set, account id empty → returns None.
    #[tokio::test]
    #[serial]
    async fn load_avalara_credentials_only_license_key_returns_none() {
        clear_avalara_env();
        std::env::set_var("AVALARA_ACCOUNT_ID", "");
        std::env::set_var("AVALARA_LICENSE_KEY", "lic-abcdef");
        assert!(load_avalara_credentials().await.is_none());
        clear_avalara_env();
    }

    /// GCP path points at missing file → warn + fall back to env vars.
    #[tokio::test]
    #[serial]
    async fn load_avalara_credentials_gcp_failure_falls_back_to_env() {
        clear_avalara_env();
        std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", "/nonexistent/sa.json");
        std::env::set_var("GCP_PROJECT_ID", "test-project");
        std::env::set_var("AVALARA_ACCOUNT_ID", "fallback-acct");
        std::env::set_var("AVALARA_LICENSE_KEY", "fallback-lic");
        let creds = load_avalara_credentials().await.expect("expected Some");
        assert_eq!(creds.account_id, "fallback-acct");
        assert_eq!(creds.license_key, "fallback-lic");
        clear_avalara_env();
    }
}
