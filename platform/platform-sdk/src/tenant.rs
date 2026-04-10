//! Tenant context middleware — automatic tenant scoping via Axum extractor.
//!
//! # Design decision
//!
//! We chose an Axum `FromRequestParts` extractor over PostgreSQL RLS with
//! session variables because:
//!
//! - **Incremental adoption**: modules migrate one handler at a time.
//! - **No DB migration**: zero schema changes, no RLS policies to maintain.
//! - **Type safety**: handlers receive `Uuid` directly, not a string.
//! - **Connection-pool friendly**: no per-request `SET` commands that conflict
//!   with pgbouncer statement mode.
//!
//! RLS remains a viable second defence layer to add later.  When that time
//! comes, `TenantId` will be the single place to inject `SET app.current_tenant`.
//!
//! # Usage
//!
//! Replace the old `extract_tenant` + match boilerplate:
//!
//! ```rust,ignore
//! // Before:
//! pub async fn list_items(
//!     claims: Option<Extension<VerifiedClaims>>,
//!     // ...
//! ) -> impl IntoResponse {
//!     let tenant_id = match extract_tenant(&claims) {
//!         Ok(id) => id,
//!         Err(e) => return e.into_response(),
//!     };
//!     // ...
//! }
//!
//! // After:
//! pub async fn list_items(
//!     TenantId(tenant_id): TenantId,
//!     // ...
//! ) -> impl IntoResponse {
//!     // tenant_id is Uuid — guaranteed present, 401 returned automatically
//! }
//! ```

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use platform_http_contracts::ApiError;
use security::claims::VerifiedClaims;
use sqlx::PgPool;
use uuid::Uuid;

/// Tenant ID extracted from the caller's JWT claims.
///
/// Implements [`FromRequestParts`] so it can be used directly in handler
/// signatures.  Returns `401 Unauthorized` if no [`VerifiedClaims`] are
/// present in the request extensions (i.e. the auth middleware did not
/// find a valid Bearer token).
///
/// The inner value is a [`Uuid`], matching `VerifiedClaims::tenant_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantId(pub Uuid);

impl TenantId {
    /// Return the tenant ID as a `String` for passing to repo functions
    /// that still accept `&str`.
    pub fn as_string(&self) -> String {
        self.0.to_string()
    }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<S> FromRequestParts<S> for TenantId
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let claims = parts
            .extensions
            .get::<VerifiedClaims>()
            .ok_or_else(|| ApiError::unauthorized("Missing or invalid authentication"))?;
        Ok(TenantId(claims.tenant_id))
    }
}

/// Per-request database pool resolved from the authenticated tenant's identity.
///
/// Implements [`FromRequestParts`] so it can be used directly in handler signatures.
/// Combines [`TenantId`] (from JWT claims) with the module's [`TenantPoolResolver`]
/// (from the request extensions injected by [`ModuleBuilder`]) to return the correct
/// [`PgPool`] for the authenticated tenant.
///
/// **Single-database modules** (the majority of platform modules) get back the
/// default module pool — the resolver falls through to it when none is configured.
///
/// **Database-per-tenant modules** (e.g. verticals using [`DefaultTenantResolver`])
/// get back a tenant-specific pool from the cache.
///
/// # Defence-in-depth
///
/// The extractor enforces two isolation boundaries:
///
/// 1. **Authentication** — returns `401 Unauthorized` if no [`VerifiedClaims`] are
///    present (i.e. the JWT middleware did not find a valid token).
/// 2. **Pool isolation** — for multi-DB setups, the pool is scoped to the
///    authenticated tenant; a handler cannot accidentally receive another tenant's pool.
///
/// # Usage
///
/// ```rust,ignore
/// pub async fn list_orders(
///     TenantPool(pool): TenantPool,
///     // ...
/// ) -> impl IntoResponse {
///     let orders = order_repo::list(&pool, &query).await?;
///     // ...
/// }
/// ```
///
/// [`DefaultTenantResolver`]: crate::tenant_resolver::DefaultTenantResolver
/// [`ModuleBuilder`]: crate::builder::ModuleBuilder
pub struct TenantPool(pub PgPool);

impl<S> FromRequestParts<S> for TenantPool
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let claims = parts
            .extensions
            .get::<VerifiedClaims>()
            .ok_or_else(|| ApiError::unauthorized("Missing or invalid authentication"))?;

        let ctx = parts
            .extensions
            .get::<crate::context::ModuleContext>()
            .ok_or_else(|| {
                ApiError::internal(
                    "TenantPool extractor: ModuleContext not in extensions — \
                     ensure the module is started with ModuleBuilder",
                )
            })?;

        let pool = ctx
            .pool_for(claims.tenant_id)
            .await
            .map_err(|e| ApiError::internal(format!("tenant pool error: {e}")))?;

        Ok(TenantPool(pool))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use security::claims::{ActorType, VerifiedClaims};

    fn test_claims(tenant_id: Uuid) -> VerifiedClaims {
        VerifiedClaims {
            user_id: Uuid::new_v4(),
            tenant_id,
            app_id: None,
            roles: vec!["admin".into()],
            perms: vec![],
            actor_type: ActorType::User,
            issued_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(15),
            token_id: Uuid::new_v4(),
            version: "1".into(),
        }
    }

    #[tokio::test]
    async fn extracts_tenant_from_claims() {
        let tenant = Uuid::new_v4();
        let mut req = Request::builder().body(()).expect("build request");
        req.extensions_mut().insert(test_claims(tenant));

        let (mut parts, _body) = req.into_parts();
        let extracted = TenantId::from_request_parts(&mut parts, &())
            .await
            .expect("should extract");
        assert_eq!(extracted.0, tenant);
    }

    #[tokio::test]
    async fn returns_401_without_claims() {
        let req = Request::builder().body(()).expect("build request");
        let (mut parts, _body) = req.into_parts();
        let err = TenantId::from_request_parts(&mut parts, &())
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), 401);
    }

    #[test]
    fn display_formats_as_uuid() {
        let id = Uuid::new_v4();
        let tid = TenantId(id);
        assert_eq!(tid.to_string(), id.to_string());
    }

    #[test]
    fn as_string_matches_display() {
        let id = Uuid::new_v4();
        let tid = TenantId(id);
        assert_eq!(tid.as_string(), tid.to_string());
    }
}
