/// Application state for the control-plane service
use security::JwtVerifier;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

/// Shared state injected into all handlers
#[derive(Clone)]
pub struct AppState {
    /// Connection pool to the tenant-registry database
    pub pool: PgPool,
    /// Connection pool to the AR database (for platform billing runs).
    /// None if AR_DATABASE_URL is not configured or unreachable at startup.
    pub ar_pool: Option<PgPool>,
    /// JWT verifier for extracting caller claims. None disables claim extraction
    /// (all RBAC-protected routes return 401 without a valid token).
    pub jwt_verifier: Option<Arc<JwtVerifier>>,
    /// HTTP client for module vitals fanout. 3s connection timeout.
    pub http_client: reqwest::Client,
}

impl AppState {
    pub fn new(pool: PgPool, ar_pool: Option<PgPool>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("build reqwest client");
        Self {
            pool,
            ar_pool,
            jwt_verifier: None,
            http_client,
        }
    }

    /// Attach a JWT verifier to this state (builder pattern).
    pub fn with_verifier(mut self, verifier: Arc<JwtVerifier>) -> Self {
        self.jwt_verifier = Some(verifier);
        self
    }
}
