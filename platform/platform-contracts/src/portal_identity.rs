//! # Portal Identity Contract
//!
//! Canonical types for the external customer portal identity boundary.
//!
//! Portal users are **not** internal platform users. They occupy a separate
//! trust domain with distinct JWT claims, scopes (not RBAC permissions),
//! and party-scoped access.
//!
//! See ADR-017 for the full architectural decision record.

use serde::{Deserialize, Serialize};

// ── JWT Constants ────────────────────────────────────────────────────────

/// JWT issuer for portal-issued tokens. Internal services MUST reject
/// tokens with this issuer; only the portal service accepts them.
pub const PORTAL_ISSUER: &str = "portal-auth";

/// JWT audience for portal tokens. Internal services validate
/// `aud: "7d-platform"` and will reject portal tokens automatically.
pub const PORTAL_AUDIENCE: &str = "7d-portal";

/// Current portal claims schema version. Bump when adding/removing fields.
pub const PORTAL_CLAIMS_VERSION: &str = "1";

/// Actor type string for portal users in events and JWT claims.
/// Must never collide with internal actor types ("user", "service", "system").
pub const PORTAL_ACTOR_TYPE: &str = "portal_user";

// ── Portal Scopes ────────────────────────────────────────────────────────

/// Scope constants for portal user capabilities.
///
/// Portal users do not participate in the internal RBAC system.
/// Instead they carry a flat list of scope strings in their JWT.
pub mod scopes {
    pub const DOCUMENTS_READ: &str = "documents.read";
    pub const DOCUMENTS_ACKNOWLEDGE: &str = "documents.acknowledge";
    pub const ORDERS_READ: &str = "orders.read";
    pub const INVOICES_READ: &str = "invoices.read";
    pub const SHIPMENTS_READ: &str = "shipments.read";
    pub const QUALITY_READ: &str = "quality.read";
    pub const ACKNOWLEDGMENTS_WRITE: &str = "acknowledgments.write";

    /// All known portal scopes, for validation.
    pub const ALL: &[&str] = &[
        DOCUMENTS_READ,
        DOCUMENTS_ACKNOWLEDGE,
        ORDERS_READ,
        INVOICES_READ,
        SHIPMENTS_READ,
        QUALITY_READ,
        ACKNOWLEDGMENTS_WRITE,
    ];

    /// Returns true if the given string is a recognised portal scope.
    pub fn is_valid(scope: &str) -> bool {
        ALL.contains(&scope)
    }
}

// ── Portal Access Claims ─────────────────────────────────────────────────

/// JWT claims payload for portal access tokens.
///
/// This is the portal equivalent of `identity-auth`'s `AccessClaims`.
/// Key differences:
/// - `iss` = `"portal-auth"` (not `"auth-rs"`)
/// - `aud` = `"7d-portal"` (not `"7d-platform"`)
/// - `actor_type` = `"portal_user"` (not `"user"`)
/// - `party_id` is required (scopes access to one party)
/// - `scopes` replaces `roles` + `perms`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortalAccessClaims {
    // ── Standard JWT (RFC 7519) ──
    /// Portal user ID (UUID string)
    pub sub: String,
    /// Issuer — always "portal-auth"
    pub iss: String,
    /// Audience — always "7d-portal"
    pub aud: String,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Expires at (Unix timestamp)
    pub exp: i64,
    /// Unique token ID (UUID)
    pub jti: String,

    // ── Portal identity ──
    /// Owning tenant UUID
    pub tenant_id: String,
    /// Linked party UUID (customer/supplier from party module)
    pub party_id: String,
    /// Always "portal_user"
    pub actor_type: String,
    /// Granted portal scopes (e.g. ["documents.read", "orders.read"])
    pub scopes: Vec<String>,

    // ── Versioning ──
    /// Claims schema version
    pub ver: String,
}

// ── Portal Event Types ───────────────────────────────────────────────────

/// Event type constants for portal identity lifecycle events.
///
/// All portal events use `actor_type: "portal_user"` in the EventEnvelope.
pub mod events {
    pub const USER_INVITED: &str = "portal.user.invited";
    pub const USER_ACTIVATED: &str = "portal.user.activated";
    pub const USER_LOGIN: &str = "portal.user.login";
    pub const USER_LOGIN_FAILED: &str = "portal.user.login_failed";
    pub const USER_LOGOUT: &str = "portal.user.logout";
    pub const USER_DEACTIVATED: &str = "portal.user.deactivated";
    pub const USER_SCOPES_UPDATED: &str = "portal.user.scopes_updated";
    pub const USER_PASSWORD_RESET: &str = "portal.user.password_reset";
    pub const TOKEN_REFRESHED: &str = "portal.token.refreshed";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_issuer_differs_from_internal() {
        // Internal issuer is "auth-rs" — portal must never collide
        assert_ne!(PORTAL_ISSUER, "auth-rs");
    }

    #[test]
    fn portal_audience_differs_from_internal() {
        // Internal audience is "7d-platform" — portal must never collide
        assert_ne!(PORTAL_AUDIENCE, "7d-platform");
    }

    #[test]
    fn portal_actor_type_differs_from_internal() {
        assert_ne!(PORTAL_ACTOR_TYPE, "user");
        assert_ne!(PORTAL_ACTOR_TYPE, "service");
        assert_ne!(PORTAL_ACTOR_TYPE, "system");
    }

    #[test]
    fn all_scopes_are_valid() {
        for scope in scopes::ALL {
            assert!(scopes::is_valid(scope), "scope {scope} should be valid");
        }
    }

    #[test]
    fn unknown_scope_is_invalid() {
        assert!(!scopes::is_valid("admin.all"));
        assert!(!scopes::is_valid("ar.mutate"));
        assert!(!scopes::is_valid(""));
    }

    #[test]
    fn portal_claims_roundtrip() {
        let claims = PortalAccessClaims {
            sub: "00000000-0000-0000-0000-000000000001".into(),
            iss: PORTAL_ISSUER.into(),
            aud: PORTAL_AUDIENCE.into(),
            iat: 1709424000,
            exp: 1709424900,
            jti: "00000000-0000-0000-0000-000000000002".into(),
            tenant_id: "00000000-0000-0000-0000-000000000003".into(),
            party_id: "00000000-0000-0000-0000-000000000004".into(),
            actor_type: PORTAL_ACTOR_TYPE.into(),
            scopes: vec![scopes::DOCUMENTS_READ.into(), scopes::ORDERS_READ.into()],
            ver: PORTAL_CLAIMS_VERSION.into(),
        };

        let json = serde_json::to_string(&claims).expect("serialize claims");
        let decoded: PortalAccessClaims = serde_json::from_str(&json).expect("deserialize claims");

        assert_eq!(decoded.sub, claims.sub);
        assert_eq!(decoded.iss, PORTAL_ISSUER);
        assert_eq!(decoded.aud, PORTAL_AUDIENCE);
        assert_eq!(decoded.tenant_id, claims.tenant_id);
        assert_eq!(decoded.party_id, claims.party_id);
        assert_eq!(decoded.actor_type, PORTAL_ACTOR_TYPE);
        assert_eq!(decoded.scopes, vec!["documents.read", "orders.read"]);
        assert_eq!(decoded.ver, PORTAL_CLAIMS_VERSION);
    }

    #[test]
    fn claims_version_is_set() {
        assert_eq!(PORTAL_CLAIMS_VERSION, "1");
    }
}
