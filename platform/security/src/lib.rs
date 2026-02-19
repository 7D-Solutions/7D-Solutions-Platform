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

pub mod authz;
pub mod authz_middleware;
pub mod claims;
pub mod middleware;
pub mod permissions;
pub mod rbac;
pub mod ratelimit;
pub mod redaction;
pub mod service_auth;
pub mod tracing;

// Re-export service auth types
pub use service_auth::{
    generate_service_token, get_service_token, verify_service_token, ServiceAuthClaims,
    ServiceAuthError,
};

// Re-export RBAC types
pub use rbac::{Operation, RbacError, RbacPolicy, Role};

// Re-export authz middleware types (legacy placeholder)
pub use authz::{AuthzConfig, AuthzLayer, AuthzMiddleware, AuthzStatus};

// Re-export JWT-backed authz middleware types
pub use authz_middleware::{
    optional_claims_mw, ClaimsLayer, ClaimsMiddleware, RequirePermissionsLayer,
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
}
