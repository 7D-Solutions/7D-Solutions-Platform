# Plug-and-Play Wave 2: Remaining 22 Modules

## Context

We just completed plug-and-play productization on 3 modules (Inventory, Party, BOM). The pattern is proven and verified:

- **platform-http-contracts** crate provides `PaginatedResponse<T>` and `ApiError`
- **config-validator** crate provides env validation collector pattern
- Each module now: serves OpenAPI via utoipa, auto-runs migrations, validates env on startup, returns standard response envelopes, has health/ready/version endpoints

All 3 modules verified running natively: OpenAPI served (43+12+19 endpoints), health endpoints 200, standard error format, JWT security schemes documented, self-bootstrapping from empty DB.

## What we need from you

Investigate the remaining 22 modules and tell us exactly what each one needs. For each module, read the actual code and answer:

### Per-module investigation checklist

1. **Handler count**: How many HTTP handler files in `src/http/`? How many endpoints total?
2. **Response formats**: What list endpoints exist? Do they return bare `Vec<T>`, custom envelopes, or already use `PaginatedResponse`? What error format do they use — inline `json!()`, custom ErrorBody, or already ApiError?
3. **Files over 500 LOC in `src/`**: Which ones? What's the split strategy?
4. **Config validation**: Does `config.rs` use `Result<Self, String>` or panic? Does it fail on first error or collect all? Does it already use ConfigValidator?
5. **Migrations**: Does `main.rs` call `sqlx::migrate!()`? Or are migrations missing?
6. **Event bus**: Does it have consumers? How many? Are they wired into `main.rs`?
7. **Special concerns**: Anything unusual? Multiple DB pools? External service dependencies? Custom middleware? Non-standard body limits?
8. **Estimated effort**: Simple (copy pattern), Medium (needs file splits + some adaptation), Heavy (significant unique concerns)?

### The 22 modules to investigate

**Simple candidates (likely copy-paste pattern):**
- consolidation, customer-portal, notifications, numbering, pdf-editor, subscriptions, ttp, timekeeping

**Medium candidates (may need file splits):**
- shipping-receiving, maintenance, payments, quality-inspection, reporting, fixed-assets, production, integrations, workforce-competence

**Heavy candidates (many oversize files, many handlers):**
- gl (20 handlers, 6 files >500 LOC, 11 event consumers)
- ap (12 handlers, 11 files >500 LOC)
- ar (24 handlers, 10 files >500 LOC, Tilled payment integration)
- treasury (9 handlers, 3 files >500 LOC)
- workflow (4 handlers, 3 files >500 LOC)

### What the plug-and-play treatment includes (same for every module)

1. **Split oversize files** if any src/ files exceed 500 LOC (PATCH bump)
2. **Response envelopes**: All list endpoints → `PaginatedResponse<T>`, all errors → `ApiError` from platform-http-contracts (MAJOR bump to 2.0.0)
3. **OpenAPI**: utoipa 5.x + utoipa-axum on all endpoints, serve `/api/openapi.json`, Bearer JWT SecurityScheme, document tenant/permissions from JWT (MINOR bump)
4. **Startup**: ConfigValidator for env validation, auto-run migrations if missing, NATS graceful degradation if applicable (MINOR bump)
5. **Version bumps**: PATCH for splits, MAJOR for envelopes, MINOR for OpenAPI + startup. All require REVISIONS.md entries.

### Reference implementations (read these first)

- **Inventory** (most complex, 43 endpoints, NATS consumers): `modules/inventory/src/main.rs`, `modules/inventory/src/http/`, `modules/inventory/src/domain/error_conversions.rs`
- **Party** (medium, 12 endpoints): `modules/party/src/main.rs`, `modules/party/src/http/`, `modules/party/src/domain/error_conversions.rs`
- **BOM** (has recursive explosion + external NumberingClient dep): `modules/bom/src/main.rs`, `modules/bom/src/http/`
- **platform-http-contracts**: `platform/http-contracts/src/lib.rs` (PaginatedResponse, ApiError, FieldError)
- **config-validator**: `platform/config-validator/src/lib.rs` (ConfigValidator pattern)

### Output format

For each of the 22 modules, give me a structured assessment:

```
## Module: {name}
- Handlers: {count} files, ~{N} endpoints
- Response migration: {X} list endpoints need PaginatedResponse, {Y} handlers need ApiError
- File splits needed: {list of files >500 LOC with split strategy}
- Config: {current state} → ConfigValidator
- Migrations: {present/missing in main.rs}
- Event bus: {N} consumers, {wired/not wired}
- Special concerns: {anything unusual}
- Effort: Simple / Medium / Heavy
- Recommended bead count: {1 for simple, 2 for medium, 3+ for heavy}
```

Then at the end, give me the recommended wave grouping and bead creation order. AR should be last (99 endpoints, highest complexity).

### Constraints

- All Rust compilation is native via `./scripts/cargo-slot.sh` (never Docker)
- Tests hit real services, no mocks
- Files must stay under 500 LOC
- Every module is v1.0.0+ (proven) — response format changes are MAJOR bumps
- Agents cannot run Docker commands
