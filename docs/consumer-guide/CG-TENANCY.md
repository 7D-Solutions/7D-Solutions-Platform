# Consumer Guide — Tenancy, Multi-Tenancy Patterns & App Roles

> **Who reads this:** Agents building vertical apps on the 7D Platform.
> **What it covers:** Tenant provisioning, database-per-tenant routing, per-app roles, cross-app navigation, and support sessions.
> **Parent:** [PLATFORM-CONSUMER-GUIDE.md](./PLATFORM-CONSUMER-GUIDE.md)

## Contents

1. [Tenant Provisioning](#tenant-provisioning) — provisioning flow, status check endpoint
2. [Database-Per-Tenant Routing Pattern](#database-per-tenant-routing-pattern) — when to use, architecture overview
   - [Axum Middleware Pattern](#axum-middleware-pattern) — pool map, request extension, handler access
   - [Tenant Provisioning — Database Creation](#tenant-provisioning--database-creation) — NATS subscriber, create DB, run migrations, register pool
   - [Migration Strategy](#migration-strategy) — migrate all tenants at startup
   - [Management Database Schema](#management-database-schema-minimal)
3. [Per-App Roles and Cross-App Navigation](#per-app-roles-and-cross-app-navigation) — permission naming, RequirePermissionsLayer, defining your strings, launch link mechanics
4. [Support Sessions — Technical Mechanism](#support-sessions--technical-mechanism) — BFF routes, root layout polling, SupportSessionBanner

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-CONSUMER-GUIDE.md. Tenant provisioning, database-per-tenant pattern (management DB, pool map, middleware, NATS subscriber, migration strategy), per-app roles, cross-app navigation, support sessions. |
| 1.1 | 2026-02-20 | Platform Orchestrator | Fixed agent name in dev provisioning flow (BrightHill → Platform Orchestrator). Added clarifying note distinguishing dev-time manual provisioning from production NATS-driven flow. |

---

## Tenant Provisioning

Tenant provisioning is an **internal admin process** — there is no public API endpoint for creating tenants.

**Flow:**
1. Contact Platform Orchestrator to provision your tenant (development-time only; production provisioning uses NATS — see [Tenant Provisioning — Database Creation](#tenant-provisioning--database-creation))
2. Platform Orchestrator creates the tenant record via admin tools
3. You receive: `tenant_id` (UUID) + `app_id` (string, e.g. `trashtech-pro`)
4. Provisioning states: `pending` → `provisioning` → `active` | `failed`
5. Only `active` tenants can log in — identity-auth enforces this at login time

**After provisioning:**
```
GET http://7d-tenant-registry/api/tenants/{tenant_id}/status
→ { "tenant_id": "<uuid>", "status": "active" }
```

---

## Database-Per-Tenant Routing Pattern

Use this section if your vertical app gives each of your customers (tenants) an isolated Postgres database — no `tenant_id` columns on operational tables, the DB connection is the tenant boundary.

**When to use this pattern:** Full schema isolation required, regulatory data separation, or your SLA requires one customer's noisy queries to never affect another's performance.

**Platform modules do NOT use this pattern.** AR, GL, Payments, etc. use `tenant_id` columns. This pattern is for your vertical app's own operational database only.

---

### How tenant database selection works

The JWT already carries `tenant_id` (UUID). Your app uses that as the key into a pool map.

**Architecture:**
- **Management database** — one stable Postgres DB (not per-tenant). Stores your routing table: `(tenant_id → connection_string)`.
- **Pool map** — `HashMap<Uuid, PgPool>` in Axum state. Populated at startup from the management DB, updated when new tenants are provisioned.
- **Tenant DB middleware** — reads `tenant_id` from JWT claims, looks up the pool, attaches it to request extensions.

**You never send an `x-tenant-id` header.** The JWT is the source of truth.

---

### Axum middleware pattern

```rust
// In your AppState:
pub struct AppState {
    pub tenant_pools: Arc<RwLock<HashMap<Uuid, PgPool>>>,
    pub management_db: PgPool,  // one stable DB for routing config
    // ... other fields
}

// Tenant DB selection middleware:
async fn tenant_db_mw(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    mut req: Request,
    next: Next,
) -> Response {
    let pool = {
        state.tenant_pools.read().await
            .get(&claims.tenant_id)
            .cloned()
    };
    match pool {
        Some(pool) => {
            req.extensions_mut().insert(pool);
            next.run(req).await
        }
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "tenant database not available",
        ).into_response(),
    }
}

// In handlers — pull the pool from extensions:
async fn my_handler(
    Extension(pool): Extension<PgPool>,
    // ...
) -> Result<Json<MyResponse>, AppError> {
    let row = sqlx::query_as!(MyRow, "SELECT ...").fetch_one(&pool).await?;
    // ...
}
```

Register the middleware in your Axum router **after** the JWT claims middleware, so claims are always populated before pool selection runs.

---

### Tenant provisioning — database creation

When the platform provisions a new tenant, it publishes a `tenant.provisioned` NATS event. **TCP (the Tenant Control Plane, Phase 40) publishes this event** when a 7D staff member creates a new tenant in the admin UI. Subscribe to this subject in your app.

On receiving `tenant.provisioned`:

1. Create a new Postgres database: `CREATE DATABASE your_app_tenant_{short_id}`
2. Run all sqlx migrations against the new database (see below)
3. Insert the connection string into your management DB routing table
4. Create a new `PgPool` and insert it into the pool map

```rust
// NATS subscriber (pseudocode — adapt to your event envelope):
async fn handle_tenant_provisioned(
    event: TenantProvisionedEvent,
    state: AppState,
) -> Result<(), Error> {
    let conn_str = format!(
        "postgres://user:pass@host/{}_{}",
        YOUR_APP_PREFIX,
        &event.tenant_id.to_string().replace('-', "")[..8]
    );

    // 1. Create the database
    sqlx::query(&format!("CREATE DATABASE {}", db_name))
        .execute(&state.management_db).await?;

    // 2. Run migrations
    let pool = PgPool::connect(&conn_str).await?;
    sqlx::migrate!("./db/migrations").run(&pool).await?;

    // 3. Persist to routing table
    sqlx::query!(
        "INSERT INTO tenant_routing (tenant_id, connection_string) VALUES ($1, $2)",
        event.tenant_id, conn_str
    )
    .execute(&state.management_db).await?;

    // 4. Add to live pool map
    state.tenant_pools.write().await.insert(event.tenant_id, pool);

    Ok(())
}
```

---

### Migration strategy

One migrations directory. Runs against every tenant DB. No per-tenant migration divergence allowed.

```rust
// At startup: migrate all registered tenant DBs
async fn migrate_all_tenants(state: &AppState) -> Result<(), Error> {
    let tenant_pools = state.tenant_pools.read().await;
    for (tenant_id, pool) in tenant_pools.iter() {
        sqlx::migrate!("./db/migrations")
            .run(pool)
            .await
            .map_err(|e| anyhow::anyhow!("migrate tenant {}: {}", tenant_id, e))?;
    }
    Ok(())
}
```

Call `migrate_all_tenants()` during app startup before accepting requests. Call `sqlx::migrate!(...).run(&new_pool)` immediately after creating each new tenant DB.

---

### Management database schema (minimal)

```sql
-- Your app's management database (one DB, not per-tenant)
CREATE TABLE tenant_routing (
    tenant_id        UUID PRIMARY KEY,
    connection_string TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

---

## Per-App Roles and Cross-App Navigation

### Per-app permission model

The JWT `perms` field uses dot-notation. Platform module permissions follow the pattern `{module}.{action}` (e.g., `ar.mutate`). Vertical apps follow the same pattern using their `app_id` as the prefix:

```
{app-id}.admin        → full administrative access in the app
{app-id}.dispatcher   → app-specific named role (example)
{app-id}.viewer       → read-only access in the app
```

**A user can hold different roles across different apps.** The JWT carries all permissions for all apps the user has access to. Each app checks only the permissions it cares about:

```rust
// In your Axum router — require app-specific admin permission:
.route_layer(RequirePermissionsLayer::new(&["trashtech-pro.admin"]))

// Viewer access — allow read routes for any authenticated viewer:
.route_layer(RequirePermissionsLayer::new(&["trashtech-pro.viewer"]))
```

TCP manages role assignment. When a TCP staff member changes a user's role in your app, the new permission strings appear in the user's next JWT. Your app does not need to know anything about TCP — it only reads the JWT.

### Defining your permission strings

When onboarding a new vertical app to the platform, register your permission strings by sending a list to the platform orchestrator. Follow the naming convention:

```
{your-app-id}.admin
{your-app-id}.{role-name}   (one entry per named role your app defines)
{your-app-id}.viewer
```

Minimum: define at least `{your-app-id}.admin` and `{your-app-id}.viewer`. Add named roles if your app has distinct permission levels (e.g., `trashtech-pro.dispatcher` vs `trashtech-pro.driver`).

### Cross-app navigation (the launch link)

All 7D apps share an auth domain. The user's JWT is stored in an httpOnly cookie scoped to the root domain (e.g., `.7d.io`). Any app on a subdomain reads this cookie automatically.

**What this means for your app:** you do not need to implement any special token exchange or deep-link authentication. If a user arrives at your app from TCP's launch link, they are already authenticated. Your standard JWT verification middleware handles it.

**What your app must do:**
- Verify the JWT on every request (standard `ClaimsLayer` setup — already required)
- Enforce app-scoped permissions on protected routes (`RequirePermissionsLayer::new(&["your-app-id.admin"])`)
- Return `403 Forbidden` with a clear message if a user has no role in your app

**What TCP does on the launch link:**
- Shows a card for each app the tenant has subscribed to
- The Launch button is a simple `<a href target="_blank">` pointing to the app's URL
- No token is passed in the URL — the cookie does the work

**The user's experience:**
- Click Launch in TCP
- New tab opens at the app's URL
- App reads JWT from cookie, checks permissions, renders the appropriate role's view
- If the user has no role in that app, the app shows a "you don't have access" page — not an error, just an appropriate no-access state

---

## Support Sessions — Technical Mechanism

A support session is a time-limited JWT issued by the platform's identity-auth service with `actor_type: "support"`. Your app does not need to handle this specially in your permission enforcement — the JWT carries the tenant's permissions and `RequirePermissionsLayer` validates it normally.

What your app *does* need to handle: detecting when a support session is active for your tenant and displaying the `SupportSessionBanner`. This is done by polling a BFF endpoint, not by inspecting the JWT.

**The support token is only ever held by the 7D support person** — it is never sent to the customer's browser. The customer's session remains their own unchanged JWT. The banner is triggered by the customer's app detecting (via polling) that the platform has an active support session on their account.

### Your BFF routes needed

```typescript
// GET /api/support-sessions/active
// Calls platform: GET {AUTH_BASE_URL}/api/support-sessions/active?tenant_id={tenant_id}
// Returns: { agent_name, reason, expires_at, session_id } | null

// DELETE /api/support-sessions/{session_id}
// Calls platform: DELETE {AUTH_BASE_URL}/api/support-sessions/{session_id}
// Revokes the support token, ends the session immediately
```

### Your root layout

```typescript
const { data: supportSession } = useQuery({
  queryKey: ['support-session'],
  queryFn: () => fetch('/api/support-sessions/active').then(r => r.json()),
  refetchInterval: SUPPORT_SESSION_POLL_MS,  // 30_000 — from lib/constants.ts
  staleTime: 0,
});

{supportSession && <SupportSessionBanner session={supportSession} />}
```

For the full `SupportSessionBanner` component spec — what it displays, rendering rules, and the "End Session Now" handler — see `docs/frontend/PLATFORM-COMPONENTS.md` → SupportSessionBanner.

---

> See `docs/PLATFORM-CONSUMER-GUIDE.md` for the master index and critical concepts.
