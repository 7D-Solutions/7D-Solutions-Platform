# customer-portal — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.


## 2.3.3
- chore: rustfmt reflow + regenerate typed clients (no behavior change)

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.3.2 | 2026-04-04 | bd-0clpi | Add pub mod db declaration for portal_repo module | Expose db repository module so extracted repos are accessible from lib root | No |
| 2.3.1 | 2026-04-04 | bd-0clpi | SoC: extract auth + status SQL into db/ repos | Separation of concerns — handler files mixed HTTP logic with raw SQL queries | No |
| 2.3.0 | 2026-04-02 | bd-39pj0 | Adopt [platform.services] — declare peer deps in module.toml, use ctx.platform_client | VerticalBuilder adoption | No |
| 2.2.0 | 2026-04-02 | bd-binuj | Remove dead health.rs (health/ready/version handlers). SDK ModuleBuilder provides these endpoints; the file was unreferenced dead code. | Dead code cleanup — annotation audit revealed health.rs handlers were never mounted after SDK conversion. | No |
| 2.1.4 | 2026-04-01 | bd-2gyqj | Update DistributionsClient to pass &VerifiedClaims via PlatformClient::service_claims(tenant_id). Constructor uses PlatformClient::new().with_bearer_token(). | New typed client API requires per-request &VerifiedClaims for tenant-scoped auth. | No |
| 2.1.3 | 2026-04-01 | bd-tfaon | Replace inline reqwest calls to Doc-Mgmt with platform-client-doc-mgmt typed client. Removes local DocMgmtDistributionList/DocMgmtDistribution structs. | Typed client consistency — eliminate raw HTTP in favour of generated clients. | No |
| 2.1.2 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder. Replaces manual startup boilerplate with SDK startup sequence. Health/middleware stripped from build_router(). | SDK batch conversion — eliminate two classes of modules. | No |
| 2.1.1 | 2026-03-30 | bd-nhmgu | Add openapi_dump utility binary for offline spec generation. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 1.0.0 | 2026-03-28 | bd-1xe02 | Initial promotion. External customer auth boundary with RS256 JWT, Argon2 password hashing, refresh token rotation, tenant-isolated portal users, document visibility via doc-mgmt distribution check, status feed with acknowledgments, outbox event emission for all auth lifecycle events. Proof script, clippy clean, 5 tests pass (1 unit + 4 real-DB integration). Security audit: no blocking findings. | Production readiness gate for first paying customer (Fireproof ERP). | No |
| 1.0.1 | 2026-03-28 | bd-29c9i.3 | Admin routes (/portal/admin/*) now require customer_portal.admin permission instead of party.mutate. | Security audit: party.mutate conflated party record management with portal user administration — two distinct privilege scopes. | No |
| 2.0.0 | 2026-03-30 | bd-c2dnv | Standard response envelopes. All error responses migrated from inline json!() tuples to platform ApiError with request_id on every error path. List endpoints (status feed, documents) now return PaginatedResponse with page/page_size query params. AuthResponse (login/refresh) preserved as-is — not wrapped in PaginatedResponse. TracingContext threaded through all handlers. | Plug-and-play wave 2: platform-wide response contract conformance. | YES — error shape changed from ad-hoc JSON to `{error, message, request_id}`; list responses wrapped in `{data, pagination}`. Consumers must update error parsing and list response handling. |
| 2.1.0 | 2026-03-30 | bd-c2dnv | OpenAPI spec via utoipa + startup improvements. All handlers annotated with #[utoipa::path], all types derive ToSchema/IntoParams. /api/openapi.json serves full spec. /healthz legacy endpoint added. Config migrated to ConfigValidator for structured validation with all-at-once error reporting. | Plug-and-play wave 2: discoverability and operational standardization. | No |