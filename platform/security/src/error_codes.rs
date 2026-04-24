//! Stable JWT/tenant failure taxonomy for platform-wide error_code values.
//!
//! These codes are **public API** — once shipped, the wire strings must never change.
//! Each variant carries a `const &'static str` so any rename is caught at compile time.
//!
//! # Frontend handling guidance
//!
//! | Code                     | HTTP status | Recommended action                        |
//! |--------------------------|-------------|-------------------------------------------|
//! | `invalid_jwt`            | 401         | Discard token, prompt re-login            |
//! | `expired_jwt`            | 401         | Attempt silent refresh, then re-login     |
//! | `missing_tenant_claim`   | 401         | Token malformed — re-login                |
//! | `tenant_not_found`       | 401         | Tenant deprovisioned or wrong environment |
//! | `insufficient_permissions`| 403        | Show "access denied" — do not retry       |
//! | `revoked_token`          | 401         | Token revoked — re-login                  |

use serde::Serialize;

/// Stable error codes for JWT/tenant authentication and authorization failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthErrorCode {
    /// The JWT signature is invalid, the format is malformed, or the algorithm is unsupported.
    InvalidJwt,
    /// The JWT `exp` claim is in the past.
    ExpiredJwt,
    /// The JWT is valid but does not carry a `tenant_id` claim.
    MissingTenantClaim,
    /// The `tenant_id` in the JWT does not exist in the tenant registry.
    TenantNotFound,
    /// The caller's permissions do not satisfy the required policy for this operation.
    InsufficientPermissions,
    /// The token has been explicitly revoked (e.g. via logout or admin revocation).
    RevokedToken,
}

impl AuthErrorCode {
    /// The stable wire string for this code.
    ///
    /// These strings are public API — they must never change after the first release.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidJwt => "invalid_jwt",
            Self::ExpiredJwt => "expired_jwt",
            Self::MissingTenantClaim => "missing_tenant_claim",
            Self::TenantNotFound => "tenant_not_found",
            Self::InsufficientPermissions => "insufficient_permissions",
            Self::RevokedToken => "revoked_token",
        }
    }
}

impl std::fmt::Display for AuthErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire strings are public API — this test pins them so any rename is caught immediately.
    #[test]
    fn auth_error_codes_stable() {
        assert_eq!(AuthErrorCode::InvalidJwt.as_str(), "invalid_jwt");
        assert_eq!(AuthErrorCode::ExpiredJwt.as_str(), "expired_jwt");
        assert_eq!(AuthErrorCode::MissingTenantClaim.as_str(), "missing_tenant_claim");
        assert_eq!(AuthErrorCode::TenantNotFound.as_str(), "tenant_not_found");
        assert_eq!(
            AuthErrorCode::InsufficientPermissions.as_str(),
            "insufficient_permissions"
        );
        assert_eq!(AuthErrorCode::RevokedToken.as_str(), "revoked_token");
    }

    #[test]
    fn auth_error_codes_display_matches_as_str() {
        let codes = [
            AuthErrorCode::InvalidJwt,
            AuthErrorCode::ExpiredJwt,
            AuthErrorCode::MissingTenantClaim,
            AuthErrorCode::TenantNotFound,
            AuthErrorCode::InsufficientPermissions,
            AuthErrorCode::RevokedToken,
        ];
        for code in codes {
            assert_eq!(code.to_string(), code.as_str());
        }
    }

    #[test]
    fn auth_error_codes_serialize_as_snake_case() -> Result<(), serde_json::Error> {
        let json = serde_json::to_string(&AuthErrorCode::InvalidJwt)?;
        assert_eq!(json, r#""invalid_jwt""#);

        let json = serde_json::to_string(&AuthErrorCode::InsufficientPermissions)?;
        assert_eq!(json, r#""insufficient_permissions""#);
        Ok(())
    }
}
