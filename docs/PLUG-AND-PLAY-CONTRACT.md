# Plug-and-Play Contract

This is the binding definition of what "plug and play" means for verticals integrating with the 7D Solutions Platform. If any item in the "Platform Provides" column does not work as stated, the platform is not plug-and-play.

## What the Vertical Provides

| Item | Details |
|------|---------|
| `Cargo.toml` | `platform-sdk` + any `platform-client-*` crates needed |
| `module.toml` | Module name, port, database, bus, `[platform.services]` declaring which services to call |
| `main.rs` | `ModuleBuilder::from_manifest("module.toml")` with `.routes()`, `.consumer()`, `.migrator()` |
| Business logic | Routes, handlers, domain models, SQL migrations |
| Env vars | `DATABASE_URL`, `NATS_URL`. Service URLs optional (manifest `default_url` fallback) |

That is ALL. No auth middleware. No CORS setup. No health endpoints. No HTTP client boilerplate. No header injection. No retry logic. No event plumbing.

## What the Platform Provides

### Infrastructure (via ModuleBuilder — working today)

| Capability | Status | How |
|-----------|--------|-----|
| Database pool | WORKING | `ctx.pool()` — configured from `[database]` |
| NATS event bus | WORKING | `ctx.bus()` — configured from `[bus]` |
| JWT auth + JWKS | NEEDS VERIFICATION | `[auth]` manifest section, auto-middleware |
| CORS | WORKING | `[cors]` manifest section |
| Rate limiting | WORKING | `[rate_limit]` manifest section |
| Health endpoints | WORKING | `/healthz`, `/api/health`, `/api/ready` — probes Postgres + NATS |
| Request timeout | WORKING | `[server].request_timeout` |
| Body limit | WORKING | `[server].body_limit` |
| Database migrations | WORKING | `.migrator(&MIGRATOR)` + `auto_migrate = true` |
| Outbox publisher | WORKING (single-db) | `[events.publish].outbox_table` — polls and publishes to NATS |
| Event consumers | WORKING | `.consumer("subject", handler)` with retry |
| Graceful shutdown | WORKING | SIGTERM drain |
| Tracing context | WORKING | Correlation ID + trace ID on HTTP |
| Metrics | WORKING | `/metrics` Prometheus endpoint |

### Service Access (via VerticalBuilder — partially working)

| Capability | Status | Gap |
|-----------|--------|-----|
| Typed platform clients from manifest | WORKING | `ctx.platform_client::<PartiesClient>()` resolves from `[platform.services]` |
| Tenant headers auto-injected | WORKING | PlatformClient sends x-tenant-id, x-correlation-id, x-app-id |
| Retry on transient errors (GET only) | WORKING | 429/503 retry with exponential backoff |
| Mutations never retried | WORKING | POST/PUT/PATCH/DELETE use send_once |
| Configurable per-service timeout | WORKING | `timeout_secs` in manifest |
| All GET endpoints return typed data | BROKEN | 45 GET endpoints return () — incomplete OpenAPI specs |
| Every platform module has a generated client | BROKEN | Maintenance has zero OpenAPI (33 handlers undocumented) |

### Auth (UNVERIFIED)

| Capability | Status | Gap |
|-----------|--------|-----|
| SDK JWT verification for vertical requests | UNVERIFIED | Both real verticals bypassed it — needs investigation |
| RBAC permission checking | WORKING | `ctx.require_permission()` |
| Vertical app auth (login/session/token lifecycle) | NOT PROVIDED | Verticals must proxy to identity-auth themselves |

### Tenant Isolation

| Capability | Status | Gap |
|-----------|--------|-----|
| Tenant ID extraction from JWT | WORKING | `extract_tenant()` available in SDK |
| Automatic tenant scoping on queries | NOT PROVIDED | Every SQL query needs manual `WHERE tenant_id = $1` |
| Database-per-tenant support | NOT PROVIDED | SDK assumes single database. Outbox publisher breaks on multi-db |

### Events

| Capability | Status | Gap |
|-----------|--------|-----|
| Publish via outbox (single db) | WORKING | Declare table in manifest, SDK runs relay |
| Subscribe to platform events | WORKING | `.consumer("subject", handler)` |
| Correlation ID propagation through events | BROKEN | wire_consumers doesn't use TracingContext::from_envelope |
| Provisioning events arrive | BROKEN | Outbox relay for provisioning not wired |
| Event subjects match between publisher/consumer | BROKEN | 11+ mismatches, AP double-prefix confirmed |

## Acceptance Test

A brand new vertical can go from `cargo new` to a running, authenticated module that:
1. Serves HTTP endpoints with JWT auth and tenant isolation
2. Calls Party to create and read a customer via `ctx.platform_client::<PartiesClient>()`
3. Subscribes to a platform event and processes it
4. Publishes an event through the outbox that arrives on NATS
5. Returns standard PaginatedResponse/ApiError on all endpoints

Using ONLY `module.toml` configuration and `ModuleBuilder` — zero hand-written platform code.

Time to working: under 1 hour for a Rust developer familiar with Axum.

## Gap Summary

| # | Gap | Impact | Fix |
|---|-----|--------|-----|
| 1 | 45 GET endpoints return () | Typed clients silently discard data | Fix OpenAPI specs (bd-9v3vx) |
| 2 | Maintenance has no OpenAPI | No generated client possible | Add utoipa annotations (bd-29n3k) |
| 3 | SDK auth unverified for verticals | Both verticals bypassed — may not work | Investigate and fix or document |
| 4 | No automatic tenant scoping | Manual WHERE on every query, data leak risk | Centralize extract_tenant (bd-o1a03), evaluate RLS |
| 5 | Database-per-tenant not supported | SDK outbox breaks, verticals bypass | Add ctx.pool_for(tenant_id) or document single-db requirement |
| 6 | Correlation IDs break at events | Traces discontinuous across event boundaries | Wire TracingContext::from_envelope (bd-4d7sx) |
| 7 | Provisioning events never arrive | Verticals can't auto-provision on new tenant | Wire provisioning outbox relay (bd-cinhj) |
| 8 | Event subject mismatches | Consumers subscribed to wrong subjects | Audit and fix (bd-155vo, bd-thx8s) |
| 9 | No vertical app auth kit | Every vertical writes 400-1800 lines of auth boilerplate | Create reusable auth package |

## Definition of Done

This contract is satisfied when:
- Every WORKING item above has an integration test proving it
- Every BROKEN item is fixed with a passing test
- Every NOT PROVIDED item is either implemented or explicitly documented as "vertical responsibility" with a rationale
- The acceptance test passes for a new vertical with zero hand-written platform code
- At least one real vertical (Fireproof or TrashTech) has been converted and the hand-written platform code is deleted
