//! Platform security primitives
//!
//! This crate provides shared security utilities for authentication,
//! authorization (RBAC), and rate limiting across platform modules.
//!
//! ## JWT verification
//!
//! Use [`JwtVerifier`] to validate access tokens issued by identity-auth:
//!
//! ```ignore
//! let verifier = JwtVerifier::from_public_pem(&public_key_pem)?;
//! let claims = verifier.verify(bearer_token)?;
//! println!("user={} tenant={}", claims.user_id, claims.tenant_id);
//! ```

pub mod audit_log;
pub mod authz_middleware;
pub mod claims;
pub mod middleware;
pub mod permissions;
pub mod ratelimit;
pub mod rbac;
pub mod redaction;
pub mod service_auth;
pub mod tracing;
pub mod webhook_verify;

// Re-export security audit log types
pub use audit_log::{security_event, SecurityOutcome};

// Re-export webhook verifier types
pub use webhook_verify::{
    GenericHmacVerifier, IntuitVerifier, NoopVerifier, StripeVerifier, VerifyError,
    WebhookVerifier, STRIPE_TIMESTAMP_TOLERANCE_SECS,
};

// Re-export service auth types
pub use service_auth::{
    generate_service_token, get_service_token, mint_service_jwt_with_context,
    verify_service_token, ServiceAuthClaims, ServiceAuthError,
};

// Re-export RBAC types
pub use rbac::check_permissions;
pub use rbac::{
    PERM_FLEET_MIGRATE, PERM_PROJECTION_LIST, PERM_PROJECTION_REBUILD, PERM_PROJECTION_STATUS,
    PERM_PROJECTION_VERIFY, PERM_TENANT_DEPROVISION, PERM_TENANT_SUSPEND,
};
#[allow(deprecated)]
pub use rbac::{Operation, RbacError, RbacPolicy, Role};

// Re-export JWT-backed authz middleware types
pub use authz_middleware::{
    optional_claims_mw, ClaimsLayer, ClaimsMiddleware, RawBearerToken, RequirePermissionsLayer,
};

// Re-export JWT claims types
pub use claims::{ActorType, JwtVerifier, VerifiedClaims};

/// Error type for security operations
#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("Invalid token")]
    InvalidToken,
    #[error("Token expired")]
    TokenExpired,
    #[error("Insufficient permissions")]
    InsufficientPermissions,
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
    #[error("JWKS unavailable: {0}")]
    JwksUnavailable(String),
}
