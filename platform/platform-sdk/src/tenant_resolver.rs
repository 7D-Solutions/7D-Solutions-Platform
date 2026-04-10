//! DefaultTenantResolver — database-per-tenant pool manager with Moka TTL cache.
//!
//! Provides a concrete [`TenantPoolResolver`] for modules that use a separate
//! PostgreSQL database per tenant.  Pools are created on first access and
//! cached for a configurable TTL (default: 1 hour) using a Moka async cache.
//!
//! # Design decisions
//!
//! ## Why Moka over a plain `RwLock<HashMap>`
//!
//! Moka coalesces concurrent cache misses: if two requests arrive
//! simultaneously for the same uncached tenant, the lookup closure runs
//! exactly once and both waiters receive the result.  A plain `RwLock`
//! would require the caller to handle the thundering-herd case manually.
//! The TTL eviction also frees pools for inactive tenants automatically.
//!
//! ## Defense-in-depth note
//!
//! `DefaultTenantResolver` is a *pool-level* isolation boundary — it ensures
//! each request receives a pool scoped to the authenticated tenant.  This
//! complements (not replaces) SQL-layer isolation (WHERE tenant_id = $N)
//! and optionally PostgreSQL RLS (`SET app.current_tenant`).
//!
//! PostgreSQL RLS with `SET LOCAL app.current_tenant` was evaluated as a
//! second defense layer.  It was deferred because it requires per-connection
//! `SET` commands that conflict with `pgbouncer` statement mode.  When
//! pgbouncer is eventually moved to session/transaction mode, RLS can be
//! added without changing callers.
//!
//! # Usage
//!
//! ```rust,ignore
//! use platform_sdk::{DefaultTenantResolver, TenantPoolError};
//!
//! // Simple: derive DB URL from env var
//! let resolver = DefaultTenantResolver::new(|tenant_id| async move {
//!     std::env::var(format!("TENANT_{tenant_id}_DB_URL"))
//!         .map_err(|_| TenantPoolError::UnknownTenant(tenant_id))
//! });
//!
//! // Management-DB backed
//! let resolver = DefaultTenantResolver::from_management_pool(
//!     &mgmt_pool,
//!     "SELECT db_url FROM tenant_databases WHERE tenant_id = $1",
//!     Some("SELECT tenant_id FROM tenant_databases WHERE active = true"),
//! );
//!
//! ModuleBuilder::from_manifest("module.toml")
//!     .tenant_pool_resolver(resolver)
//!     .run()
//!     .await?;
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use moka::future::Cache;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

use crate::context::{TenantPoolError, TenantPoolResolver};

/// Boxed async function that returns the database URL for a given tenant.
type LookupFn =
    dyn Fn(Uuid) -> Pin<Box<dyn Future<Output = Result<String, TenantPoolError>> + Send>>
        + Send
        + Sync;

/// Boxed async function that lists all known tenant IDs.
type ListFn = dyn Fn() -> Pin<Box<dyn Future<Output = Result<Vec<Uuid>, TenantPoolError>> + Send>>
    + Send
    + Sync;

/// A concrete [`TenantPoolResolver`] backed by a Moka async TTL cache.
///
/// On cache miss, calls the supplied `lookup` closure to get the database URL
/// for a tenant, creates a [`PgPool`], and caches it for `ttl`.
///
/// Configure via [`DefaultTenantResolver::new`] (closure) or
/// [`DefaultTenantResolver::from_management_pool`] (management-DB backed).
pub struct DefaultTenantResolver {
    lookup: Arc<LookupFn>,
    list_tenants: Option<Arc<ListFn>>,
    cache: Cache<Uuid, PgPool>,
    max_connections: u32,
}

impl DefaultTenantResolver {
    /// Create a resolver with a custom async lookup closure.
    ///
    /// The closure receives a `tenant_id` and must return the PostgreSQL
    /// connection URL for that tenant's database.
    ///
    /// Default TTL: **1 hour**.  Default pool max: **5 connections per tenant**.
    pub fn new<F, Fut>(lookup: F) -> Self
    where
        F: Fn(Uuid) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, TenantPoolError>> + Send + 'static,
    {
        Self::with_config(lookup, Duration::from_secs(3600), 5)
    }

    /// Create a resolver with explicit TTL and per-tenant max connections.
    ///
    /// `ttl` controls how long a cached pool is kept before the next access
    /// triggers a fresh lookup.  `max_connections` is applied to every
    /// per-tenant pool created by this resolver.
    pub fn with_config<F, Fut>(lookup: F, ttl: Duration, max_connections: u32) -> Self
    where
        F: Fn(Uuid) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, TenantPoolError>> + Send + 'static,
    {
        let lookup: Arc<LookupFn> = Arc::new(move |tenant_id| Box::pin(lookup(tenant_id)));
        let cache = Cache::builder()
            .time_to_live(ttl)
            .max_capacity(1_000)
            .build();
        Self {
            lookup,
            list_tenants: None,
            cache,
            max_connections,
        }
    }

    /// Register an async closure that enumerates all known tenant IDs.
    ///
    /// Required by [`all_pools`](TenantPoolResolver::all_pools), which the
    /// multi-tenant outbox publisher calls to iterate every tenant database.
    /// Without this, `all_pools()` returns an error.
    pub fn with_list_tenants<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<Uuid>, TenantPoolError>> + Send + 'static,
    {
        self.list_tenants = Some(Arc::new(move || Box::pin(f())));
        self
    }

    /// Build a resolver backed by a management-plane PostgreSQL table.
    ///
    /// `lookup_query` must accept a `$1: UUID` parameter and return a single
    /// `TEXT` column containing the database URL, e.g.:
    ///
    /// ```sql
    /// SELECT db_url FROM tenant_databases WHERE tenant_id = $1
    /// ```
    ///
    /// `list_query` (optional) accepts no parameters and returns a column of
    /// `UUID` tenant IDs — used by the multi-tenant outbox publisher.
    ///
    /// ```sql
    /// SELECT tenant_id FROM tenant_databases WHERE active = true
    /// ```
    pub fn from_management_pool(
        pool: PgPool,
        lookup_query: impl Into<String>,
        list_query: Option<impl Into<String>>,
    ) -> Self {
        let pool_lookup = pool.clone();
        let query = lookup_query.into();

        let resolver = Self::new(move |tenant_id| {
            let p = pool_lookup.clone();
            let q = query.clone();
            async move {
                let db_url: Option<String> = sqlx::query_scalar(&q)
                    .bind(tenant_id)
                    .fetch_optional(&p)
                    .await
                    .map_err(|e| TenantPoolError::Pool(e.to_string()))?;
                db_url.ok_or(TenantPoolError::UnknownTenant(tenant_id))
            }
        });

        if let Some(lq) = list_query {
            let pool_list = pool.clone();
            let list_q = lq.into();
            resolver.with_list_tenants(move || {
                let p = pool_list.clone();
                let q = list_q.clone();
                async move {
                    let ids: Vec<Uuid> = sqlx::query_scalar(&q)
                        .fetch_all(&p)
                        .await
                        .map_err(|e| TenantPoolError::Pool(e.to_string()))?;
                    Ok(ids)
                }
            })
        } else {
            resolver
        }
    }
}

#[async_trait]
impl TenantPoolResolver for DefaultTenantResolver {
    /// Return the pool for `tenant_id`, creating it on first access.
    ///
    /// Concurrent requests for the same uncached tenant are coalesced:
    /// the lookup closure runs once and all waiters receive the same pool.
    async fn pool_for(&self, tenant_id: Uuid) -> Result<PgPool, TenantPoolError> {
        let lookup = Arc::clone(&self.lookup);
        let max_connections = self.max_connections;

        self.cache
            .try_get_with(tenant_id, async move {
                let db_url = lookup(tenant_id).await?;
                PgPoolOptions::new()
                    .max_connections(max_connections)
                    .connect(&db_url)
                    .await
                    .map_err(|e| TenantPoolError::Pool(format!("connect failed: {e}")))
            })
            .await
            .map_err(|arc_err| TenantPoolError::Pool(arc_err.to_string()))
    }

    /// Return pools for all known tenants.
    ///
    /// Requires a `list_tenants` closure (see [`with_list_tenants`](Self::with_list_tenants)).
    /// Returns an error if none was registered.
    async fn all_pools(&self) -> Result<Vec<(Uuid, PgPool)>, TenantPoolError> {
        let list_fn = self.list_tenants.as_ref().ok_or_else(|| {
            TenantPoolError::Pool(
                "DefaultTenantResolver: list_tenants not configured — \
                 call .with_list_tenants() to enable all_pools()"
                    .into(),
            )
        })?;

        let tenant_ids = list_fn().await?;
        let mut result = Vec::with_capacity(tenant_ids.len());

        for tenant_id in tenant_ids {
            let pool = self.pool_for(tenant_id).await?;
            result.push((tenant_id, pool));
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Stub resolver: maps tenant UUIDs to in-memory URLs.
    fn stub_resolver(map: HashMap<Uuid, String>) -> DefaultTenantResolver {
        let map = Arc::new(Mutex::new(map));
        // We test the closure/cache logic without a real DB by using an invalid
        // URL and verifying the error path.
        DefaultTenantResolver::new(move |tenant_id| {
            let m = Arc::clone(&map);
            async move {
                m.lock()
                    .expect("test mutex poisoned")
                    .get(&tenant_id)
                    .cloned()
                    .ok_or(TenantPoolError::UnknownTenant(tenant_id))
            }
        })
    }

    #[tokio::test]
    async fn unknown_tenant_returns_error() {
        let resolver = stub_resolver(HashMap::new());
        let id = Uuid::new_v4();
        let err = resolver.pool_for(id).await.unwrap_err();
        // The connection attempt will fail (invalid URL), but we get Pool error
        // from the lookup path — unknown tenant → Pool error.
        assert!(matches!(err, TenantPoolError::Pool(_) | TenantPoolError::UnknownTenant(_)));
    }

    #[tokio::test]
    async fn all_pools_without_list_tenants_returns_error() {
        let resolver = stub_resolver(HashMap::new());
        let err = resolver.all_pools().await.unwrap_err();
        assert!(matches!(err, TenantPoolError::Pool(_)));
    }

    #[tokio::test]
    async fn all_pools_with_empty_list_returns_empty_vec() {
        let resolver = stub_resolver(HashMap::new())
            .with_list_tenants(|| async { Ok::<Vec<Uuid>, TenantPoolError>(vec![]) });
        let pools = resolver.all_pools().await.expect("all_pools should succeed with empty list");
        assert!(pools.is_empty());
    }
}
