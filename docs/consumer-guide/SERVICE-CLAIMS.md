# Service Claims and `Uuid::nil()` Semantics

> Canonical reference for when to use `PlatformClient::service_claims`, when
> `Uuid::nil()` is the correct tenant ID, and how claims propagate across
> HTTP handlers, NATS consumers, and background jobs.

**Source:** `platform/platform-sdk/src/http_client.rs:84`

---

## 1. What `service_claims` Does

`PlatformClient::service_claims(tenant_id)` builds a `VerifiedClaims` value
for code paths that don't have an inbound JWT — event consumers, background
jobs, module-to-module HTTP calls triggered by internal logic.

```rust
// platform/platform-sdk/src/http_client.rs
pub fn service_claims(tenant_id: uuid::Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: uuid::Uuid::nil(),   // no human user
        tenant_id,                     // caller supplies this
        app_id: None,
        roles: vec![],
        perms: vec!["service.internal".to_string()],
        actor_type: ActorType::Service,
        issued_at: Utc::now(),
        expires_at: Utc::now() + TimeDelta::hours(1),
        token_id: uuid::Uuid::new_v4(),
        version: "1.0".to_string(),
    }
}
```

Key facts:

- `user_id` is always `Uuid::nil()` — there is no human user.
- `actor_type` is always `Service`, never `User`.
- `perms` contains only `"service.internal"` — modules that need to distinguish
  service callers from human callers can check this.
- The `tenant_id` you pass is what the receiving module uses for row-level
  isolation. **Getting this wrong breaks tenant boundaries.**

A convenience variant, `service_claims_from_str`, parses a string tenant ID so
you don't need the `Uuid::parse_str` boilerplate:

```rust
let claims = PlatformClient::service_claims_from_str(&envelope.tenant_id)?;
```

Both are also available on `ModuleContext`:

```rust
let claims = ctx.service_claims(tenant_id);
let claims = ctx.service_claims_from_str(&tenant_str)?;
```

---

## 2. When `Uuid::nil()` as `tenant_id` Is Allowed

Use `service_claims(Uuid::nil())` **only** when there is genuinely no tenant
in scope. Three situations qualify:

| Situation | Example | Why nil is correct |
|-----------|---------|-------------------|
| Platform-internal bookkeeping | `tenantctl` provisioning a new tenant before the tenant row exists | No tenant exists yet |
| Cross-tenant aggregation | System-level summary queries that span all tenants | Query is not scoped to one tenant |
| System-actor maintenance | Migration backfills, schema maintenance | Operation is at the platform level |

### When `Uuid::nil()` Is Forbidden

If you have a tenant ID available — from an event envelope, an HTTP request,
a job payload, or any other source — you must pass it. These are **bugs**, not
convenience shortcuts:

- Event consumer receives `envelope.tenant_id` but passes `Uuid::nil()` to
  a downstream HTTP call.
- Background job has a tenant scope but uses nil because "it's just internal."
- Test code uses nil to avoid creating a real tenant (use `Uuid::new_v4()`
  instead — test isolation requires distinct IDs, not nil).

**Rule of thumb:** if the data you're reading or writing belongs to a specific
tenant, the claims must carry that tenant's real UUID.

---

## 3. Claims Propagation by Context

### HTTP Handlers (User-Facing)

The auth middleware extracts `VerifiedClaims` from the inbound JWT and injects
them via Axum's `Extension`. Handlers receive the real user's identity:

```rust
// modules/reporting/src/http/statements.rs
pub async fn get_income_statement(
    State(state): State<AppState>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<StatementParams>,
) -> Result<Json<IncomeStatement>, ApiError> {
    let claims = claims.ok_or(ApiError::unauthorized("Missing claims"))?;
    // claims.user_id  = real user UUID
    // claims.tenant_id = real tenant UUID
    // ...
}
```

**Do not** call `service_claims` inside an HTTP handler. The inbound claims
already have the correct user and tenant. If the handler needs to make a
module-to-module call, pass the inbound claims through:

```rust
// Good: forward the user's claims to the downstream module
let resp = party_client.get_party(&claims, party_id).await?;
```

### Module-to-Module HTTP Calls (Service-Initiated)

When a consumer or background job calls another module's HTTP API, there is no
inbound JWT. Use `service_claims` with the tenant ID from your context:

```rust
// modules/shipping-receiving/src/integrations/inventory_client.rs
let claims = PlatformClient::service_claims(tenant_id);
let result = receipts.post_receipt(&claims, &body).await?;
```

```rust
// modules/consolidation/src/integrations/gl/client.rs
let claims = PlatformClient::service_claims(Self::parse_tenant(tenant_id)?);
let resp = self.client.get(&path, &claims).await?;
```

The tenant ID typically comes from:
- The event envelope's `tenant_id` field
- A database record's `tenant_id` column
- A job payload

### NATS Event Consumers

Consumers receive an `EventEnvelope` with `tenant_id` as a string. Two patterns:

**Pattern A — Direct database writes (no HTTP call).** The consumer writes to
its own database using the tenant ID from the envelope for row-level isolation.
No `service_claims` needed:

```rust
// modules/gl/src/consumers/gl_posting_consumer.rs
// Extracts tenant_id from envelope, uses it in SQL queries directly.
// No outbound HTTP call, so no service_claims.
```

**Pattern B — Downstream HTTP call.** The consumer calls another module. Build
service claims from the envelope's tenant ID:

```rust
// modules/subscriptions/src/gated_invoice_creation.rs
let tenant_uuid: Uuid = tenant_id.parse()?;
let claims = PlatformClient::service_claims(tenant_uuid);
let resp = ar_client.create_invoice(&claims, &create_req).await?;
```

### Background Jobs and Schedulers

Same as consumers: use `service_claims` with the tenant ID from the job's
payload. If the job operates across tenants, iterate and build claims per
tenant:

```rust
for tenant_id in active_tenants {
    let claims = PlatformClient::service_claims(tenant_id);
    billing_client.run_cycle(&claims, &params).await?;
}
```

---

## 4. Audit Trail Implications

The audit system (`platform/audit/src/actor.rs`) maps claim types to actors:

| `ActorType` | `Actor` constructor | `actor.id` | Audit trail shows |
|-------------|-------------------|-----------|-------------------|
| `User` | `Actor::user(user_id)` | Real user UUID | "User X did Y" |
| `Service` | `Actor::service("module-name")` | Deterministic UUID v5 | "Service shipping-receiving did Y" |
| `System` | `Actor::system()` | `Uuid::nil()` | "System did Y" |

When `service_claims` is used, the actor type is `Service` and user_id is nil.
Downstream modules that write audit records should use `Actor::service(name)`
rather than `Actor::user(claims.user_id)` — a nil user ID in audit logs is a
sign that claims were constructed correctly (no human user), not that something
went wrong.

**If your code writes audit records for user-scoped mutations and the actor is
`Service` with `Uuid::nil()` as user_id, that's a red flag.** It means a
human's action lost attribution somewhere in the call chain. Trace back to
where the original `VerifiedClaims` should have been forwarded instead of
replaced with `service_claims`.

---

## Quick Reference

| Context | Claims source | `tenant_id` |
|---------|--------------|-------------|
| HTTP handler (user request) | Axum `Extension<VerifiedClaims>` | From JWT |
| HTTP handler calling another module | Forward inbound `VerifiedClaims` | From JWT |
| Event consumer calling another module | `PlatformClient::service_claims(tenant_id)` | From `EventEnvelope.tenant_id` |
| Consumer writing to own DB | SQL uses `envelope.tenant_id` directly | From envelope |
| Background job (tenant-scoped) | `PlatformClient::service_claims(tenant_id)` | From job payload |
| Platform maintenance (no tenant) | `PlatformClient::service_claims(Uuid::nil())` | nil — intentional |
| Tests | `PlatformClient::service_claims(Uuid::new_v4())` | Random — test isolation |
