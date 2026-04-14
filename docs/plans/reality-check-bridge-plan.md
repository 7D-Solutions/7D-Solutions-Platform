# Reality Check — Bridge Plan (2026-04-10/11)

**Author:** BrightHill (Orchestrator)
**Source:** Reality check run 2026-04-10, after swarm landed 25 commits
**Status:** Round 5 COMPLETE — ready for bead generation
**Tracking bead:** bd-us2u8

## Vision gap summary

The platform is delivery-capable: all 7 manufacturing phases done, accounting spine tested, multi-tenancy enforced, outbox atomicity proven. But there are concrete gaps between "the tests pass" and "a new vertical can onboard and ship."

SageDesert already validated this with bd-s56d3: the first real cross-module e2e test found a silent nil-tenant-UUID bug in `PlatformClient::inject_headers`. That bug passed every module-level test because module-level tests don't exercise cross-service calls. The pattern is: **the gap isn't bad code, it's missing coverage at the boundaries**.

## Gap inventory

### GAP-01: Platform completion gate is stale [REFINED ROUND 3]
**What:** `scripts/proof_platform_completion.sh` defines 4 sub-gates (contract validation, breaking-change, perf smoke, onboarding e2e). Today's 25 commits bumped versions on AP, Party, Notifications, Security, Production, Shipping-Receiving, BOM, plus added new contracts (bd-e5yna, bd-0f1oq).
**Why it bites:** A vertical pulling latest will hit compile errors on response shape changes from bd-1vq9e. The breaking-change gate exists specifically to catch this.
**Blast radius:** Every downstream vertical. Cross-project API drift goes undetected.
**Round 3 acceptance sharpening:**
- Run mode: `./scripts/proof_platform_completion.sh --skip-perf --skip-e2e` as minimum viable (gates 1+2 only). Perf needs k6 + staging; e2e needs Playwright + TCP UI running.
- Expected output: Both gate 1 (contract validation) and gate 2 (breaking-change) must exit 0.
- **If gate 2 fails:** Each failure is a real breaking change. For each failure, decide: (a) bump module MAJOR version + migration note in REVISIONS.md, (b) revert the breaking change, or (c) document it as an accepted breaking change with migration guide.
- **bd-1vq9e specifically is a known risk:** it changed AP allocations/payment_runs/reports response types. Expected to fail gate 2 unless the version bump (AP 3.3.1→3.3.2) is MINOR and the responses are backwards-compat (they're not — typed structs replaced raw JSON).
- This is a MINOR bump in Cargo.toml but a BREAKING change for consumers. **The versioning gate needs to recognize this case** — PATCH/MINOR bumps that change response shapes are currently allowed to slip through.
**Acceptance:**
1. Run completion gate, capture output.
2. If gate 2 fails on bd-1vq9e, either: upgrade AP to 4.0.0 (major bump, acknowledge breaking), OR revert the response-type changes. Same for any other failing module.
3. Repeat until gate 1+2 exit 0.
4. Document findings in `docs/reality-check-completion-gate-run-2026-04-11.md`.
**Files:** `scripts/proof_platform_completion.sh`, possibly `modules/ap/Cargo.toml` (major bump) and REVISIONS.md, possibly other modules.

### GAP-02: Dockerfile.runtime deleted — dev cross-build image unreproducible [REVISED ROUND 5]
**What:** bd-0txiq changed `infra/Dockerfile.runtime` CMD from `dev-entrypoint.sh` + supervisord to hardcoded `/app/service`. Then bd-d77cl.1 ("delete obsolete dev image recipes") **deleted `infra/Dockerfile.runtime` entirely.** The currently running `7d-runtime` containers use an image that was built before these changes — when Docker image cache is cleared or recreated, nobody can rebuild the `7d-runtime` image.
**Round 5 scope correction (critical update):** `docker-compose.services.yml` uses `flywheel/rust-dev-runtime:2026-04` (external Docker Hub image, not rebuilt locally). `docker-compose.cross.yml` is what uses `image: 7d-runtime` — this is the **dev cross-compilation overlay** used when developers iterate locally. The blast radius is **dev workflow, not production services**. Production services use the external flywheel image which is unaffected.
**What currently exists in infra/:** `supervisord.conf` (uses `%(ENV_SERVICE_BINARY)s`), `dev-entrypoint.sh` (starts supervisord, requires `SERVICE_BINARY`), `watch-binary.sh`. The image infrastructure WORKS — only the Dockerfile that packages it was deleted.
**When it bites:** Any developer does a fresh checkout or clears Docker local images — they cannot `docker build -t 7d-runtime` to start the dev environment. `docker-compose.cross.yml` line 8 says "Build it once: docker build -t 7d-runtime -f infra/Dockerfile.runtime ." — that file is gone.
**Proposed fix:** Recreate `infra/Dockerfile.runtime` as a supervisord-based dev image using the existing `infra/dev-entrypoint.sh`, `infra/supervisord.conf`, and `infra/watch-binary.sh` that are all still present. The image should start supervisord as its entrypoint, which reads `SERVICE_BINARY` from the environment — this matches every service in `docker-compose.cross.yml` which already sets `SERVICE_BINARY`.
**Acceptance:**
1. `infra/Dockerfile.runtime` exists and builds cleanly: `docker build -t 7d-runtime -f infra/Dockerfile.runtime .` exits 0.
2. Container started with the rebuilt image and `SERVICE_BINARY=/usr/local/bin/ar-rs` starts the binary correctly via supervisord.
3. `watch-binary.sh` watcher is running in the container (ps shows the process).
4. Binary replacement (overwrite the mounted binary) triggers automatic restart within 5s.
**Files:** `infra/Dockerfile.runtime` (recreate), `docs/dev/CROSS-COMPILE-SETUP.md` (document build-once step so devs know to rebuild after clearing Docker).

### GAP-03: Service-to-service tenant context only verified on one path [REFINED ROUND 3]
**What:** SageDesert fixed `inject_headers` to mint per-request JWTs with real tenant context (bd-s56d3 in 653d7b97). That fix is on the happy path. **We don't know how many other places in platform-sdk still use the startup-time nil-UUID bearer token.**
**Why it bites:** Every silent nil-tenant call is a potential cross-tenant data leak or a silent "returns empty" bug (like what BOM enrichment hit).
**Round 3 audit plan — concrete greps:**
1. `grep -rn "get_service_token()" platform/ modules/` — every caller of the startup-time token mint
2. `grep -rn "bearer_token" platform/platform-sdk/src/` — every place the cached token is used
3. `grep -rn "PlatformClient::new" platform/ modules/` — direct client construction that might bypass the SDK wrapper
4. `grep -rn "reqwest::Client" modules/ platform/` — raw HTTP client usage that might not mint JWTs at all
5. Check every `PlatformClient` field access in handler code
**Round 3 canary test design:**
- Spin up 2 tenants (tenant-A and tenant-B) with distinct data (items, invoices, WOs).
- For each cross-module call in the platform, issue a request as tenant-A and assert:
  1. The call succeeds with 200.
  2. The downstream service's logged `claims.tenant_id` matches tenant-A's UUID (NOT nil, NOT tenant-B's).
  3. Response data is tenant-A's (not empty, not tenant-B's).
- Calls to cover: BOM→Inventory, Production→BOM, Production→Numbering, Production→Inventory, Shipping→Inventory, Shipping→QI, AR→Party, AP→Party, AP→Inventory, Notifications→Email.
**Round 3 acceptance:**
1. Grep audit produces a finding list: file:line for every suspect pattern.
2. Every finding is either (a) verified safe with a comment explaining why, or (b) fixed with a patch.
3. Canary test covers 10+ cross-module endpoints, all asserting real tenant propagation.
4. Test is added to `e2e-tests/tests/tenant_context_canary_e2e.rs`.
5. CI job runs the canary on every PR.
**Files:** `e2e-tests/tests/tenant_context_canary_e2e.rs` (new), audit findings committed to `docs/audits/tenant-context-audit-2026-04-11.md`, possibly fixes in `platform/platform-sdk/src/http_client.rs` and consumer-side callers.

### GAP-04: Tenant provisioning status lacks per-module visibility [REFINED ROUND 4]
**What:** ~~There is no provisioning API~~ — **Round 4 correction:** The API exists. `POST /api/control/tenants` returns 202 Accepted with tenant_id. `GET /api/control/tenants/{tenant_id}/provisioning` returns step-level status (7 steps: validate, create DBs, run migrations, seed, verify connectivity, verify schema versions, activate). `POST /api/control/tenants/{tenant_id}/retry` retries failed provisioning. The create handler is in `platform/control-plane/src/handlers/create_tenant.rs`; the status handler is `platform/control-plane/src/handlers/provisioning_status.rs`.

**What's actually missing:** (a) The provisioning status endpoint returns 7-step level detail, not 26-module-level detail. `cp_tenant_module_status` table tracks per-module status but the API doesn't expose it. (b) **Round 5 confirmed:** `create_tenant` handler has ZERO RBAC middleware — no `RequirePermissionsLayer`, no `Claims` extraction, no permission check. Any valid JWT can create tenants. This is an unauthorized access gap. (c) No e2e test that calls `POST /api/control/tenants`, polls until status is active, then makes a tenant-scoped call to each module.
**Why it bites:** A vertical UI calling `GET /api/control/tenants/{id}/provisioning` can see "step 3 of 7 complete" but cannot tell which of the 26 module databases actually failed. Debugging a broken provisioning requires looking at server logs.
**Round 4 acceptance:**
1. `GET /api/control/tenants/{tenant_id}/provisioning` response includes `module_statuses: [{ module_code, status, error? }]` populated from `cp_tenant_module_status`. All 26 modules visible.
2. Create-tenant endpoint requires `platform.tenants.create` permission (add RBAC constant to `platform/security/src/permissions.rs`).
3. E2e test: `POST /api/control/tenants` → poll `/provisioning` until all modules `ready` → call `GET /api/ap/invoices` (tenant-scoped) → assert 200. Test fails the scenario where one module fails migration.
4. Concurrent test: 5 simultaneous creates, all reach active state, no cross-tenant contamination.
**Files:** `platform/control-plane/src/handlers/provisioning_status.rs` (add module_statuses field), `platform/security/src/permissions.rs` (add PLATFORM_TENANTS_CREATE constant), `platform/control-plane/src/handlers/create_tenant.rs` (add RBAC gate), new `e2e-tests/tests/tenant_provisioning_api_e2e.rs`.

### GAP-05: GL period close enforcement at AP/AR entry boundary [REVISED ROUND 4]
**What:** ~~Zero handlers exist~~ — **Round 4 correction:** GL period close is comprehensively implemented. `modules/gl/src/http/period_close.rs` exposes `POST /api/gl/periods/{period_id}/validate-close`, `POST /api/gl/periods/{period_id}/close`, `GET /api/gl/periods/{period_id}/close-status`, plus reopen request/approve/reject routes. `period_repo::assert_period_open()` returns `PeriodError::PeriodClosed` on any insert to a closed period. All GL consumers comment that `process_gl_posting_request` enforces period state. `period_close_service` has three sub-modules (validation, snapshot with SHA-256 hash, execution). Multiple test files: `period_close_enforcement_test.rs`, `period_close_validation_test.rs`, `period_close_schema_test.rs`.

**What's actually missing:** (a) AP and AR modules accept entry creation with any `effective_date` — they do NOT pre-validate that the GL period is open for that date. A user can create a backdated AP invoice into a closed period. It will sit in AP with the wrong date. When AP emits the `ap_vendor_bill_approved` event, GL will reject it via `process_gl_posting_request → assert_period_open` — but the rejection is at the event-consumer level, giving the user a confusing late error. (b) No e2e test proves the full AP→GL rejection path for a closed period. (c) The period reopen approval workflow is implemented but untested end-to-end.
**Why it bites:** Backdated entries silently accepted in AP/AR, rejected asynchronously by GL. User sees approval succeed in AP, then discovers the GL posting failed hours later. Worse: the AP record now exists but has no corresponding GL entry — silent financial inconsistency.
**Acceptance:**
1. AP and AR invoice/bill creation endpoints validate that the GL period for the entry's `effective_date` is open BEFORE accepting the entry. Call `GET /api/gl/periods?date={effective_date}&status=open` or a direct DB check via the GL period repo exported as a shared lib.
2. If the period is closed, return `422 Unprocessable Entity` with `{ error: "PERIOD_CLOSED", message: "Period for {date} is closed — request reopen or adjust the date" }`.
3. E2e test: close a GL period → attempt AP invoice in closed period → assert 422 → request period reopen → approve → attempt again → assert 201.
4. E2e test: verify GL rejects the event-bus posting path as a defense-in-depth (belt-and-suspenders — AP pre-validates, GL also enforces).
**Files:** `modules/ap/src/http/invoices.rs` (add period-open guard), `modules/ar/src/http/invoices.rs` (add period-open guard), new `e2e-tests/tests/period_close_enforcement_e2e.rs`.

### GAP-06: Composite endpoints not exercised in full manufacturing cycle
**What:** bd-dhl7p (composite WO create), bd-6pyqw (composite outbound ship), bd-k5bla (batch WO), bd-i6if4 (derived WO status), bd-8v63o (routing enrichment), bd-7lb9x (BOM enrichment) all landed today. Each has unit + integration tests. **bd-s56d3 is in flight adding e2e coverage but so far has tested each module in isolation.**
**Why it bites:** The realistic customer path is: create WO → receive raw parts → issue components to WO → complete ops → final QI → FG receipt → ship outbound → invoice customer. We have ZERO tests that exercise that full chain.
**Blast radius:** Every customer-facing manufacturing flow. Silent bugs live at the seams.

### GAP-07: ECO change control is recording, not gating [CONFIRMED ROUND 4]
**What:** Phase D complete — ECO entity, workflow, BOM revision supersession, numbering all work. **There is no enforcement that prevents a Production WO from consuming a BOM revision that was superseded by an ECO.**
**Round 4 confirmation:** `modules/bom/src/domain/eco_service/service.rs:131` sets `status = 'superseded'` on the old BOM revision when an ECO is applied and emits `bom.revision_superseded` event. `modules/production/src/domain/work_orders.rs:219` has `bom_revision_id: Option<Uuid>` — no status check. `CompositeCreateWorkOrderRequest` accepts any `bom_revision_id` without querying BOM to verify it's in `draft` or `released` status (not `superseded`). Zero supersession guards found anywhere in `modules/production/src/`.
**Why it bites:** For aerospace (Fireproof), ECO enforcement is compliance. The whole point is "once rev B is approved, rev A cannot be built against."
**Blast radius:** First-paying-customer-facing. Aerospace sign-off depends on this.
**Acceptance:**
1. `composite_create_work_order` and `create_work_order` in `modules/production/src/http/work_orders.rs` reject any `bom_revision_id` whose BOM revision status is `superseded`. Call BOM service (via SDK client) or read the revision status from the cross-module event store.
2. Guard returns `422` with `{ error: "BOM_REVISION_SUPERSEDED", message: "BOM revision {id} was superseded by ECO {eco_number}. Use revision {new_rev_id} instead." }`.
3. Integration test: apply an ECO that supersedes rev A → attempt WO creation with rev A → assert 422 with ECO reference. Attempt WO with rev B → assert 201.
4. Existing WOs referencing superseded revisions are NOT retroactively invalidated — only new WO creation is gated.
**Files:** `modules/production/src/http/work_orders.rs` (add guard), `modules/production/src/domain/bom_client.rs` (new — query BOM revision status via platform_client), or use a read-through from `bom_revision_superseded` events if preferred.

### GAP-08: Cross-service rate limiting not activated
**What:** `platform/security` has `TieredRateLimiter` with per-tier strategies (Composite/IpOnly/TenantOnly, bd-6sle9). **Only wired up in Identity-Auth.** AR, AP, GL, Production, Inventory, etc. have no rate limiting against internal callers.
**Why it bites:** A compromised or misbehaving service can flood another service. Blast radius grows with every new integration.

### GAP-09: Carrier adapters require opt-in with no guidance
**What:** Real USPS/FedEx/UPS adapters exist. `StubCarrierProvider` is the default-wired provider. A vertical shipping physical goods must manually swap providers at startup.
**Why it bites:** Zero documentation on how to opt in. A vertical will ship with stub by accident and find out when orders don't actually get picked up.

### GAP-10: Observability at service boundaries is thin
**What:** `correlation_id` flows through events (bd-4xmvr RouteOutcome). `tracing::info_span!` is used in handlers. **Distributed tracing across HTTP service calls isn't verified.** The SageDesert tenant-context bug would have been caught by span propagation that included claims.tenant_id.
**Why it bites:** Silent cross-service bugs (like today's nil-UUID) are effectively impossible to debug without span context propagation.

### GAP-11: Breaking change detection isn't gate-enforced on merge
**What:** `scripts/ci/check-openapi-breaking-changes.sh` exists. It's run by the completion gate. **It is not run on every PR.** Today's bd-1vq9e changed AP, Party response shapes. That's a breaking change and it landed without the gate flagging it.
**Why it bites:** Silent breaking changes go out, verticals find out on next pull.

### GAP-12: Tenant pool resolver isn't wired for default isolation strategy
**What:** `DefaultTenantResolver` exists with Moka cache. **Single-database-per-module modules (all 26 core) don't use it.** They rely on `WHERE tenant_id = $1` in every query.
**Why it bites:** A single missed `WHERE tenant_id` = cross-tenant data leak. Row-level security would be a defense-in-depth layer that's currently missing.
**Decision:** Deferred. See [docs/architecture/RLS-EVALUATION.md](../architecture/RLS-EVALUATION.md) for the current evaluation and revisit conditions.
**Acceptance:** Document that RLS is not the chosen strategy right now and explain the defense (tenant middleware + SQL scoping + audit). If the pool layer later supports `SET LOCAL app.current_tenant`, add Postgres RLS policies and integration tests then.

### GAP-13: OpenAPI contract drift between modules and client code [CLARIFIED ROUND 4]
**What:** bd-tpcki generated TS clients for all 27 modules. bd-e5yna + bd-0f1oq generated `contracts/*/openapi.json` files. **Nothing enforces that the committed `contracts/*/openapi.json` matches what `openapi_dump` produces from the live module code today.** A module developer can change a handler, forget to regenerate the contract, and ship divergent client types.
**Why it bites:** Silent API drift. Vertical integrators compile against stale TS types and hit runtime errors.
**Round 4 clarification on CI approach:** `openapi_dump` is a standalone binary (`cargo run --bin openapi_dump > openapi.json`) that generates the spec from utoipa annotations at compile time — no DB or NATS connection needed (confirmed by reading `modules/party/src/bin/openapi_dump.rs`). Compile failure means drift is caught at the build step before the dump even runs. The CI job is: (1) `cargo build --bin openapi_dump --release` (compile errors = drift caught), (2) run binary and diff against committed JSON, (3) fail on diff. No special graceful-failure handling needed — compile failure is its own gate. TS client regen is a downstream step: after openapi.json is confirmed current, regenerate `.d.ts` via the TS codegen tool and diff.
**Acceptance:** CI job that (1) builds `openapi_dump` for every module, (2) runs each binary and diffs output against `contracts/*/openapi.json`, (3) fails PR on any diff. Second job (can share the build artifact) regenerates TS clients from fresh openapi.json and diffs against committed `.d.ts` files.
**Files:** `.github/workflows/ci.yml`, new script `tools/ci/check-contract-drift.sh`.

### GAP-14: Event dedup and replay windows are not explicitly configured
**What:** NATS JetStream provides dedup via message IDs within a window. `EventEnvelope` has `idempotency_key`. **The dedup window is default (2 minutes in JetStream).** A slow consumer that lags more than 2 minutes will process duplicates if a publisher retries.
**Why it bites:** "Exactly-once" becomes "at-least-once" under load. Downstream state divergence.
**Acceptance:** Explicit dedup window config per stream (default 24h for financial events, 1h for notifications), documented in module.toml or platform-wide manifest.
**Files:** `platform/event-bus/src/stream_config.rs`, per-module `module.toml`.

### GAP-15: Database migration rollback safety is a pattern, not a standard
**What:** `modules/ap/tests/migration_safety_test.rs` exists. It's 1 file out of 26 modules. **No other module has a migration safety test**, and the AP one was even marked with an ignore attribute in bd-1vq9e (DarkCrane's "bonus fix").
**Why it bites:** Forward migrations ship freely. Rollback/recovery is untested. A bad migration in production has no tested rollback path.
**Acceptance:** Migration safety test pattern extracted to `tools/migration-safety-test/` as a reusable helper. Every proven module has a migration_safety_test that validates the last N migrations are reversible or has a documented "not reversible, here's the forward-only recovery path" note.
**Files:** new `tools/migration-safety-test/`, per-module `tests/migration_safety.rs`.

### GAP-16: Tenant DB bootstrap race on first request [CONFIRMED ROUND 4, DEPENDENCY NOTED ROUND 5]
**What:** `activate_tenant` (step 7) sets `status = 'active'` without polling per-module health. After activation, the control-plane publishes `tenant.provisioned` — each module handles it asynchronously. The first user request can arrive before all modules process the event.
**Round 5 dependency confirmed:** The Round 4 acceptance requires `/api/ready?tenant_id={id}` endpoint on each module. But `platform/health/src/lib.rs` implements `/api/ready` as a global readiness check (DB up, NATS up) with no `tenant_id` parameter. The tenant-aware ready endpoint doesn't exist yet.
**Two-part fix (ordered):**
1. **GAP-31 first:** Add `?tenant_id=` parameter to `/api/ready` that checks if the module has processed the tenant's provisioning event and is ready to serve tenant-scoped requests.
2. **GAP-16 second (depends on GAP-31):** `activate_tenant` (step 7) polls all 26 `GET /api/{module}/ready?tenant_id={id}` endpoints before setting `status = 'active'`.
**Acceptance:**
1. `platform/health/src/lib.rs` adds a `TenantReadinessCheck` trait. Each module implements it: returns true after `tenant.provisioned` event is processed.
2. `GET /api/ready?tenant_id={id}` returns `{ status: "up"|"warming", tenant: { id, status, modules_ready: N, modules_total: 26 } }`.
3. `activate_tenant` polls all 26 modules for `status: up` within 90s before setting `status = 'active'`.
4. If any module doesn't return `up` within 90s, mark tenant `degraded` (not active) with list of non-ready modules exposed in `/provisioning` status.
5. Integration test: provision a tenant, assert all module ready probes return `up`, then assert every module accepts a tenant-scoped request with 200.
**Files:** `platform/health/src/lib.rs` (add TenantReadinessCheck trait), `platform/control-plane/src/provisioning/steps.rs` (activate_tenant), per-module health route wiring.

### GAP-17: Bulk data import paths don't exist
**What:** A new customer typically brings: chart of accounts, opening balances, BOM structures, item master, vendor list, customer list, price book. **There are no bulk-import endpoints or tools for any of these.** A vertical onboarding = 10k lines of insert SQL hand-crafted per customer.
**Why it bites:** Onboarding time dominated by manual data wrangling. No idempotency on imports — if an import fails halfway, customer has inconsistent state.
**Acceptance:** Platform-level bulk-import bead: CSV/JSON format specs for each entity type (COA, items, BOMs, vendors, customers), validation endpoints, idempotent row-by-row import with clear error reporting. Per-module importers (AP vendor import, AR customer import, Inventory item import, BOM import).
**Files:** New `platform/bulk-import/` crate or per-module `/imports` endpoints. Initial scope: COA + items + vendors + customers (4 most-needed).

### GAP-18: Secret rotation story is undocumented
**What:** `JWT_PRIVATE_KEY_PEM`, `SERVICE_AUTH_SECRET`, per-module DB passwords. **Rotation procedure is not documented. `JwtVerifier::from_jwks_url` (bd-x2k12) supports JWKS rotation but nothing in 7D actually uses it.**
**Why it bites:** Compliance audit (SOC2, HIPAA) requires key rotation. First compliance audit = scramble.
**Acceptance:** `docs/operations/secret-rotation.md` with step-by-step runbooks for JWT key rotation (blue-green via JWKS), SERVICE_AUTH_SECRET rotation, per-module DB password rotation. One integration test validates JWT rotation: mint token with key-A, rotate to key-B, validate both work during overlap window, key-A expires cleanly.
**Files:** new `docs/operations/secret-rotation.md`, new e2e test.

### GAP-19: Prometheus metrics exist but SLOs and alerting rules don't
**What:** Every module exports Prometheus metrics (request duration, counters, gauges). `docs/monitoring/` has some alert examples. **No codified SLO definitions. No Prometheus alert rules committed. No runbook linking alerts to owner/action.**
**Why it bites:** Platform goes into degraded state, nothing pages anyone, customer finds out first.
**Acceptance:** `ops/slo.yaml` defines SLOs per critical endpoint (availability, latency p95, error rate). `ops/alerts/*.rules.yaml` codifies alert rules that trigger on SLO burn. `docs/operations/runbook.md` links each alert to an action.
**Files:** new `ops/slo.yaml`, `ops/alerts/`, `docs/operations/runbook.md`.

### GAP-20: Time zone handling is UTC-everywhere with no tenant locale
**What:** Every timestamp is UTC in the DB. Reports, aging, statements all use UTC. **A tenant in Pacific time sees "invoice dated 2026-04-11" because the UTC timestamp crossed midnight UTC while it was still April 10 locally.**
**Why it bites:** Off-by-one-day bugs in reports, aging buckets, statement periods, month-end close.
**Acceptance:** `platform/tenant-registry` stores `locale_tz` per tenant. Reporting queries accept tenant_tz as a parameter and cast UTC timestamps to tenant local time for date-bucket computations. Period close uses tenant-local midnight boundaries, not UTC.
**Files:** `platform/tenant-registry/db/migrations/*_add_tenant_locale.sql`, `modules/reporting/src/http/date_helpers.rs`, `modules/gl/src/domain/periods.rs`.

### GAP-21: Soft delete semantics are inconsistent
**What:** Some tables have `deleted_at`, some use `status='deleted'`, some rely on archive tables, some just DELETE. **No platform-wide convention. GDPR erasure request handling is not implemented.**
**Why it bites:** GDPR compliance gap. Customer deletion requests can't be satisfied consistently.
**Acceptance:** Platform doc `docs/architecture/SOFT-DELETE-STANDARD.md` defines the convention (deleted_at timestamp, retention class, purge job). Every new table must conform. GDPR erasure endpoint exists that tombstones tenant data across all modules with a documented retention window before hard delete.
**Files:** new `docs/architecture/SOFT-DELETE-STANDARD.md`, `platform/control-plane/src/gdpr_erasure.rs`.

### GAP-22: Cross-module transaction boundaries under partial failure [SCOPED ROUND 4]
**What:** bd-dhl7p's composite WO create = WO record + BOM resolution + numbering call + operations insert in "one transaction." But the numbering call is an HTTP call to the Numbering service. **If numbering returns success but the follow-up inserts fail, the numbering sequence is burned.** If numbering times out but the call succeeded, we might create a duplicate WO number on retry.
**Round 4 scoping:** Only ONE composite HTTP endpoint was found in the codebase: `POST /api/production/work-orders/create` (`modules/production/src/http/work_orders.rs:101 composite_create_work_order`). Shipping-receiving's bd-6pyqw added guard/event logic for outbound ship but did NOT add a composite HTTP endpoint — it's a direct `POST /api/shipping/shipments/{id}/ship` mutation with a single DB transaction. So the scope of this gap is specifically the production composite WO endpoint, not a multi-module audit.
**The production composite WO flow:**
1. Allocate WO number from Numbering service (HTTP call, no local transaction yet)
2. Begin Postgres transaction
3. Insert WO record with allocated number
4. If `bom_revision_id` given: fetch BOM from BOM service (second HTTP call, inside TX window)
5. Insert routing operations
6. Commit transaction
**Failure modes:** If step 3-6 fails, the Numbering sequence is burned (no void call). If BOM fetch (step 4) hangs, the transaction is held open for the timeout duration. If the caller retries with the same request, a new WO number is allocated (no idempotency key on the Numbering call).
**Why it bites:** Silent data inconsistency in composite flows. Exactly the class of bug bd-s56d3 is starting to explore.
**Acceptance:**
1. Document the compensating action: Numbering service must be idempotent by `(tenant_id, request_id)`. Call Numbering with a deterministic `request_id` derived from the WO creation idempotency key so retries return the same number.
2. OR: Void the allocated sequence number on failure by calling `DELETE /api/numbering/sequences/{id}` in the error path.
3. BOM fetch (step 4) is moved BEFORE the transaction starts — don't hold a TX open while waiting for HTTP.
4. Integration test: inject failure at step 3 (after Numbering call), verify no orphaned WO number is created on retry. Test: inject BOM fetch timeout — verify TX releases cleanly.
**Files:** `modules/production/src/http/work_orders.rs`, `modules/production/src/domain/work_orders.rs` (reorder BOM fetch before TX), new `e2e-tests/tests/composite_wo_failure_injection_e2e.rs`.

### GAP-23: Connection pool config is per-module and inconsistent
**What:** bd-z53ou fixed Payments pool starvation by adding `pool_acquire_timeout_secs=5` and `pool_max=30`. **Every other module relies on SDK defaults** (pool_max=10 per the SDK default). Under load, modules without explicit tuning will exhibit the same starvation pattern.
**Why it bites:** Same bug will recur module-by-module as load increases. We already paid the cost of finding it once.
**Acceptance:** SDK default pool config bumped based on observed real-world needs. Each module's `module.toml` explicitly declares pool_max/pool_min/acquire_timeout based on its workload class (write-heavy/read-heavy/mixed). Load test validates each class under synthetic load.
**Files:** `platform/platform-sdk/src/manifest/database.rs`, every `modules/*/module.toml`, new `tools/load-test/`.

### GAP-24: No cross-module API version compatibility matrix
**What:** Every module has its own semver. Platform-sdk is 0.1.0. Every REVISIONS.md entry says "breaking: No." **There is no documented compatibility matrix saying "SDK 0.1.x works with Security 1.6.x, AP 3.3.x, AR 6.3.x" — and when a vertical pulls latest from all modules, compat isn't guaranteed.**
**Why it bites:** A vertical pinning to specific versions gets no guidance on what else they need to pin. Upgrade paths are ad-hoc.
**Acceptance:** `docs/COMPATIBILITY-MATRIX.md` maintained per release (or per major version change) listing known-good module combinations. Release tag creates a snapshot of all module versions. Vertical can pin to a release tag and get a consistent set.
**Files:** new `docs/COMPATIBILITY-MATRIX.md`, git tag convention.

### GAP-25: Payments connection health — self-recovered, needs canary test [REVISED ROUND 4]
**What:** `7d-payments` was hung (health endpoint timing out) for several hours after bd-z53ou. Strike counter reached 240+.
**Round 4 update:** Payments self-recovered on 2026-04-12/13 with no manual intervention. Health is now green. Most likely H4 (stale binary — container restarted at some point) or H2 (NATS subscription unstuck on redeliver). No root cause formally diagnosed since it self-cleared.
**What remains:** No canary test that would detect pool starvation or health handler hang within 60s of onset. Next time this class of bug hits, detection is "customer notices service is down." No formal post-mortem.
**Why it bites:** Silent health endpoint hangs go undetected for hours. The Docker health-poller hit the strike threshold and fired a mail notification — but only after 20+ minutes of degradation.
**Hypotheses that remain open (for canary design):**
- H1: `publish_batch` connection leak on error path — canary should verify connections are released after errors.
- H2: NATS subscription hang at startup — canary should verify startup completes within 10s.
- H3: Health endpoint DB query hangs — canary should verify health endpoint responds in <1s even under load.
**Acceptance (P2 — not blocking):**
1. `e2e-tests/tests/payments_health_canary_e2e.rs`: 50 concurrent requests + assert health responds within 1s + assert pool_acquire_timeout fires fast-fail under exhaustion.
2. Docker healthcheck interval for payments set to 10s (not default 30s).
3. `docs/audits/payments-health-2026-04-12.md` — record hypotheses, what self-cleared, what remains unknown.
**Files:** `e2e-tests/tests/payments_health_canary_e2e.rs` (new), `infra/docker-compose.services.yml` (health check interval), `docs/audits/payments-health-2026-04-12.md` (new).

### GAP-26: Backup exists, but not automated or drill-verified [REVISED ROUND 4]
**What:** ~~No automated backup exists~~ — **Round 4 correction:** `scripts/backup_all.sh` (pg_dump-based, iterates all module DBs per tenant, writes compressed SQL to timestamped directory) and `scripts/dr_drill.sh` (validates backup integrity, restore capability, DB connectivity) both exist in the repo.

**What's actually missing:** (a) Neither script is scheduled — no cron, no CI job, no supervisord service runs `backup_all.sh` nightly. (b) `dr_drill.sh` has never been run against a real production backup (only local dev). (c) RPO/RTO targets not documented. (d) No Prometheus metric tracking "age of last successful backup" so ops can alert on missed backups.
**Why it bites:** Scripts exist but aren't running. No backup = any data loss is terminal, regardless of whether a backup script is in the repo.
**Acceptance:**
1. Nightly cron (or supervisord scheduled task) runs `scripts/backup_all.sh`. Verified: output is non-empty, no errors in `backup.log`.
2. Backup age Prometheus metric: `platform_backup_age_seconds{module}` updated by a sidecar. Alert fires if backup age > 26h.
3. Weekly restore drill in CI staging: `scripts/dr_drill.sh` runs against latest backup and exits 0. CI fails if drill fails.
4. `docs/operations/backup-restore.md` with RPO target (≤24h), RTO target (≤4h), and the specific commands to restore a specific tenant-module to a specific point in time.
**Files:** `scripts/backup_all.sh` (exists — schedule it), `scripts/dr_drill.sh` (exists — run in CI), new supervisord config or cron entry, new `docs/operations/backup-restore.md`.

### GAP-27: No graceful degradation policy for non-critical deps
**What:** Every module treats every dependency as required. If Numbering is down, composite WO create fails hard. If Notifications is down, AR posting fails hard (actually — it's via outbox, so it queues, but there's no explicit policy documented). **There is no concept of "critical deps" vs "best-effort deps" in any module.**
**Why it bites:** A single non-critical service blip cascades into customer-visible failures.
**Acceptance:** Each module declares its dep classifications (`critical` = fail hard if down, `degraded` = succeed but log, `best-effort` = fire-and-forget). SDK provides `ctx.critical_client::<T>()` vs `ctx.degraded_client::<T>()` that returns `Result<T, DegradedMode>`. Integration test: kill Notifications, verify AR still accepts invoice creation with a "notifications-degraded" warning header.
**Files:** `platform/platform-sdk/src/context.rs`, `platform/platform-sdk/src/manifest/dependencies.rs`, per-module `module.toml` dep classification.

### GAP-28: No circuit breakers or bulkheads
**What:** All HTTP service-to-service calls use plain reqwest with a timeout. **No circuit breaker** — a slow downstream service blocks every upstream caller until timeouts fire. **No bulkhead** — a single runaway endpoint can consume the entire connection pool of its caller.
**Why it bites:** Cascading failures. One slow service = platform-wide slowdown.
**Acceptance:** `PlatformClient` gets a circuit breaker (3 strikes → open for 30s → half-open probe). Per-target connection pool caps (bulkheads). Integration test: inject 5s latency into one endpoint, verify upstream circuit opens after 3 requests and returns a fast-fail without blocking other endpoints.
**Files:** `platform/platform-sdk/src/http_client.rs` (add circuit breaker layer), new e2e test.

### GAP-29: Audit crate is complete but not wired into any module [CONFIRMED ROUND 4]
**What:** `platform/audit` crate is a real, complete implementation — NOT a stub. It has `schema.rs` (AuditEvent + MutationClass + WriteAuditRequest), `writer.rs`, `policy.rs`, `outbox_bridge.rs`, `diff.rs`, `actor.rs`, and integration tests for all of them. DB migration `20260216000001_create_audit_log.sql` creates the `audit_events` table with `mutation_class` enum. 

**Round 4 finding:** `grep -rn "platform_audit\|use audit::\|audit::write\|audit::record" modules/ --include="*.rs"` returned ZERO results. No production module calls the audit crate. The crate has integration tests but **no consumer**. The lib.rs docs claim "The E2E test oracle validates that every mutation has exactly one audit record" — but that oracle doesn't exist yet.
**Why it bites:** First compliance audit (SOC2) finds every financial mutation unlogged. AP invoices, AR payments, GL journals, production WOs — zero audit trail.
**Acceptance:**
1. `docs/architecture/AUDIT-STANDARD.md` defines which mutation classes require audit records (AP: bills/payments; AR: invoices/payments/credit-notes; GL: journals/period-close; Production: WO state transitions; Inventory: adjustments; Control-plane: tenant lifecycle).
2. Each module in the audit-required list has `platform-audit` in its `Cargo.toml` as a dependency.
3. The Guard→Mutation→Outbox flow in each module's write endpoints calls `audit::writer::write_audit_event(pool, request)` as part of the transaction.
4. Integration test oracle: for each mutation endpoint in AP/AR/GL, assert that exactly one audit record with the correct `mutation_class`, `entity_id`, and `actor_id` exists after the call.
5. CI job: `grep -rn "platform_audit" modules/ --include="*.rs" | wc -l` must be >= 30 (rough proxy for audit wiring density — fails if nobody is wired).
**Files:** `docs/architecture/AUDIT-STANDARD.md` (new), `platform/audit/Cargo.toml` (exists), per-module `Cargo.toml` (add platform-audit dep), per-module mutation handlers (add `audit::writer::write_audit_event` call), new oracle integration tests.

### GAP-30: Reconciliation jobs don't exist [INVARIANTS ENUMERATED ROUND 4]
**What:** Financial data has internal invariants that should never be violated. **No scheduled reconciliation jobs validate these invariants.**
**Round 4 — specific invariants per module:**
- **AR:** `invoice.total_amount = sum(lines.unit_price * lines.quantity) + invoice.tax_amount` for all non-voided invoices. `sum(payment.amount WHERE invoice_id = X) <= invoice.total_amount` (overpayment is impossible).
- **AP:** Same invariant for bills. `aging_bucket_30 + aging_bucket_60 + aging_bucket_90 + aging_bucket_90plus = total_outstanding_payables` (aging must sum to total).
- **GL:** For each journal entry: `sum(debit_amounts) = sum(credit_amounts)` (double-entry invariant). For each closed period: `period_close_hash = SHA-256(entry_count, debit_sum, credit_sum, entry_hash_chain)` (tamper-detection).
- **Inventory:** `item.on_hand_qty = sum(receipt movements) - sum(issue movements)` per item-warehouse-tenant. Cannot be negative for non-lot-tracked items.
- **Production:** For completed WOs: `wo.actual_output_qty <= wo.planned_qty * 1.1` (10% overrun tolerance). Component issues: `sum(issued_qty) >= bom_required_qty` for "closed" status.
- **BOM:** Each BOM revision has exactly one `status` in {draft, released, superseded, obsolete}. A BOM cannot be released if it has lines with quantity = 0.
**Why it bites:** Drift from bugs, manual data correction, or race conditions goes undetected until a customer notices a financial discrepancy — which is the worst possible detection path.
**Acceptance:** Nightly reconciliation job per module that validates the invariants above and emits a Prometheus metric `platform_recon_violations_total{module, invariant}`. Any violation triggers an alert. Dashboard showing "last reconciliation: PASSED" per module with violation count.
**Files:** new `tools/reconciliation/` with a reconciliation runner binary per module, new `docs/architecture/RECONCILIATION-INVARIANTS.md` documenting all invariants and their query forms.

### GAP-31: /api/ready exists globally but not tenant-aware [CLARIFIED ROUND 5]
**What:** `platform/health/src/lib.rs` implements `/api/ready` — it checks DB up, NATS up, pool metrics. It IS distinct from `/api/health`. **What's missing:** It doesn't accept a `?tenant_id=` parameter. Under load, first requests after a rolling deploy can also time out while things warm up (connection pool fill, migration check).
**Round 5 finding:** The endpoint exists and works globally. The tenant-readiness gap was discovered while solving GAP-16: `activate_tenant` needs per-tenant readiness probes. The `/api/ready?tenant_id={id}` is a new feature that needs to be added.
**Why it bites:** (1) Rolling deploys cause latency spikes for first requests (pool not filled yet). (2) `activate_tenant` can't confirm per-tenant readiness, enabling the GAP-16 race condition.
**Acceptance:**
1. `/api/ready` already passes global checks (keep it).
2. Add optional `?tenant_id={uuid}` parameter: when provided, additionally checks that the module has processed the `tenant.provisioned` event for that tenant. Returns `{ global: "up", tenant: { id, status: "up"|"warming" } }`.
3. Every module's implementation of the `TenantReadinessCheck` trait stores a local flag set when the `tenant.provisioned` consumer fires. The `/api/ready?tenant_id={id}` handler checks this flag.
4. Deploy script (or rolling-update orchestration) waits for `/api/ready` → `up` before sending traffic to a new instance.
**Files:** `platform/health/src/lib.rs` (add TenantReadinessCheck, extend ready handler), per-module provisioning consumer (set ready flag on event), `scripts/dev/wait-for-ready.sh`.

### GAP-32: Local developer environment is a bring-your-own-dependencies story
**What:** README says "Start the data stack, start backend services, run tests." **In practice:** musl-cross toolchain, correct Rust target, NATS with auth, Postgres per module, env vars, cargo-slot setup, cross-watcher start. A new developer or contributor faces hours of setup.
**Why it bites:** Onboarding friction. Contributions limited to developers who already have the environment.
**Acceptance:** Single-command bring-up: `./scripts/dev/up.sh` that detects missing prereqs, runs everything needed, and exits with a green "ready" or a precise "missing: brew install musl-cross." `docs/CONTRIBUTING.md` with verified 15-minute first-PR flow.
**Files:** `scripts/dev/up.sh`, `scripts/dev/doctor.sh` (exists, integrate), `docs/CONTRIBUTING.md`.

### GAP-33: No structured logging standard
**What:** Modules use `tracing::info!` freely with mixed formats. Some log JSON, some plain text. `RUST_LOG` level is the only control. **No standard for "what goes in a log line" — required fields like tenant_id, request_id, actor_id are applied inconsistently.**
**Why it bites:** Log aggregation is noisy. Cross-module trace reconstruction from logs is hand-wavy. Compliance audit asks "show me all actions by user X across all modules" and the answer is "grep."
**Acceptance:** `docs/architecture/LOGGING-STANDARD.md` defines required fields per log level. SDK provides `ctx.log_span()` that auto-injects tenant/request/actor. Lint in CI that flags `tracing::info!` calls missing required fields in request handlers.
**Files:** `docs/architecture/LOGGING-STANDARD.md`, `platform/platform-sdk/src/logging.rs`, CI lint.

### GAP-34: Error budget / SLO burn tracking missing
**What:** Even once GAP-19 lands SLO definitions and alert rules, **there's no error-budget tracking** — the cumulative "how much availability have we burned this month vs the SLO target." Ops flies blind on release pacing: is it safe to ship a risky change today or have we already burned the budget?
**Why it bites:** Release decisions made on vibe instead of data.
**Acceptance:** Grafana dashboard showing SLO budget remaining per critical endpoint, with historical burn rate. Automated "release freeze recommended" signal when budget is below threshold.
**Files:** `ops/grafana/dashboards/slo-budget.json`, new Prometheus recording rules.

### GAP-35: No customer data export path
**What:** Customer leaves the platform. **What do they take with them?** Today: nothing. There's no bulk export, no data migration file format, no offboarding runbook.
**Why it bites:** GDPR data-portability requirement. Regulatory risk. Also: trust issue for prospective customers ("can we leave if it doesn't work out?").
**Acceptance:** `POST /api/control-plane/tenants/{id}/export` kicks off a job that generates a ZIP bundle with all tenant data in documented JSON/CSV formats per module. `docs/operations/tenant-offboarding.md` with legal retention timelines.
**Files:** new `platform/control-plane/src/export_job.rs`, per-module exporter trait.

### GAP-36: No dependency vulnerability scanning in CI
**What:** Rust deps are managed via Cargo.lock. **No automated scanning for known CVEs (cargo-audit, cargo-deny).** Docker base images not scanned (trivy, grype). Secret scanning on git history not configured (gitleaks).
**Why it bites:** First security audit finds CVEs. Supply chain attacks undetected.
**Acceptance:** CI job runs `cargo audit --deny warnings` on every PR. `cargo deny check` enforces license allowlist + banned crates. Weekly `trivy` scan of all container images with failing gate on HIGH/CRITICAL. `gitleaks` runs on every PR + history.
**Files:** `.github/workflows/security.yml`, `deny.toml`.

### GAP-37: Feature flag framework missing
**What:** New features land fully wired. **There is no feature flag system** for gradual rollout, per-tenant enablement, kill switches, or A/B testing. Rolling back a new feature = revert commit + deploy.
**Why it bites:** No safe rollout path for risky changes. No per-customer enablement for beta features. No kill switch when a feature misbehaves in production.
**Acceptance:** Simple feature flag crate with per-tenant + global flags, stored in tenant-registry. `ctx.feature_enabled("composite_wo_create", &claims)` pattern. Admin endpoint to flip flags. Integration test validates enabled/disabled paths.
**Files:** new `platform/feature-flags/`, tenant-registry migration for flags table.

### GAP-38: No noisy-neighbor isolation between tenants
**What:** Shared-database modules (all 26 core) share a single connection pool across all tenants. **A single abusive tenant can starve the pool** and degrade service for every other tenant.
**Why it bites:** Fair use violations are platform-wide problems.
**Acceptance:** Per-tenant connection budget within the shared pool. `ctx.pool_for_tenant(id)` returns a permit that respects the tenant's quota. Integration test: hammer one tenant with 1000 concurrent requests, verify other tenants still see normal latency.
**Files:** `platform/platform-sdk/src/tenant_quota.rs`, SDK pool wrapper.

## Priority matrix

| Gap | Priority | Rationale |
|-----|----------|-----------|
| GAP-01 | **P0** | We don't know if today's commits broke contract compat for downstream verticals |
| GAP-02 | **P1** | Dev cross-build image unreproducible (Dockerfile deleted) — not prod-facing |
| GAP-03 | **P0** | Active silent bug class; SageDesert found one, there may be more |
| GAP-26 | **P0** | No backup / restore means any data loss = terminal |
| GAP-06 | **P1** | SageDesert in flight; formalize + complete the full-cycle test |
| GAP-13 | **P1** | Contract drift is silent today — protects all future SDK releases |
| GAP-11 | **P1** | CI gate that prevents the same class of break as GAP-01 |
| GAP-04 | **P1** | Blocker for next vertical onboarding |
| GAP-05 | **P2** | GL close is implemented; AP/AR pre-validation and e2e proof needed |
| GAP-16 | **P1** | Tenant provisioning race is latent — first slow tenant finds it |
| GAP-22 | **P1** | Composite flow correctness under partial failure — known risk class |
| GAP-17 | **P1** | Onboarding velocity — every vertical hits this manually today |
| GAP-29 | **P1** | Audit trail gap is compliance-blocking for SOC2 |
| GAP-30 | **P1** | Reconciliation jobs catch silent financial drift before customers do |
| GAP-36 | **P1** | Dependency vuln scanning is compliance-blocking |
| GAP-27 | **P2** | Graceful degradation prevents cascading failures |
| GAP-28 | **P2** | Circuit breakers prevent same cascade class |
| GAP-31 | **P2** | Warm-up probe prevents rolling-deploy latency spikes |
| GAP-07 | **P2** | Compliance gate for aerospace; first paying customer is aerospace |
| GAP-10 | **P2** | Debugging multiplier — every other gap is harder to catch without this |
| GAP-14 | **P2** | Event dedup window is silent risk; becomes urgent under load |
| GAP-15 | **P2** | Migration safety is a pattern, not a standard |
| GAP-08 | **P2** | Defense in depth, not blocking |
| GAP-19 | **P2** | SLOs + alerts are table-stakes for operating the platform |
| GAP-23 | **P2** | Pool config inconsistency — same bug class as bd-z53ou will recur |
| GAP-25 | **P2** | Self-recovered; needs canary test to detect next time within 60s |
| GAP-33 | **P2** | Structured logging standard needed before observability really pays off |
| GAP-38 | **P2** | Noisy neighbor isolation — one bad tenant can take down everyone |
| GAP-18 | **P3** | Secret rotation — needed for compliance audit, not v1 |
| GAP-09 | **P3** | Documentation + opt-in flag; fast to close |
| GAP-20 | **P3** | Time zone bugs — real but manifest as off-by-one, not total break |
| GAP-21 | **P3** | GDPR erasure — first erasure request = scramble without this |
| GAP-24 | **P3** | Compat matrix — nice-to-have until a vertical asks for it |
| GAP-32 | **P3** | Dev env onboarding friction — limits contributor velocity |
| GAP-34 | **P3** | Error budget tracking — depends on GAP-19 first |
| GAP-35 | **P3** | Customer export — GDPR + trust story |
| GAP-37 | **P3** | Feature flag framework — deferred until a real risky rollout needs it |
| GAP-12 | **P4** | Defense in depth; not addressing a known bug |

## Bridge strategy

**Sequencing invariant:** Fix active outages → prevent future outages → close structural gaps → polish.

### Wave 1 (P0 — must land before anyone sleeps)
1. **GAP-01** — Run `proof_platform_completion.sh` against current main. Surface breakage from today's version bumps. Must complete before any vertical pulls latest.
2. **GAP-03** — Audit `inject_headers` and every other place that might use cached startup tokens. Add a canary test that fails if any cross-service call sees nil-UUID tenant.
3. **GAP-26** — Schedule `backup_all.sh` and validate `dr_drill.sh` against a real backup. Every day without automated backups is data-loss exposure for Fireproof customer.
4. **GAP-04 (RBAC only)** — `POST /api/control/tenants` has ZERO auth. Any JWT holder can create tenants. Add `RequirePermissionsLayer` with `PLATFORM_TENANTS_CREATE` permission immediately. This is a security gap, not a feature gap.

### Wave 2 (P1 — ships the vertical readiness story)
5. **GAP-02** — Recreate `infra/Dockerfile.runtime` (deleted by bd-d77cl.1). Dev cross-build breaks on next Docker cache clear.
6. **GAP-06** — Let bd-s56d3 finish, then build the full WO→ship→invoice e2e flow as the gold-standard integration test.
7. **GAP-13** — Contract drift gate in CI. Runs `openapi_dump` for every module, diffs against committed contracts.
8. **GAP-11** — Breaking-change gate runs on every PR, not just the completion script.
9. **GAP-04 (remainder)** — Per-module status in provisioning status endpoint + e2e test (depends on GAP-02 RBAC fix already landed).
10. **GAP-31** — `/api/ready?tenant_id=` parameter + TenantReadinessCheck trait. (Required before GAP-16 can be implemented.)
11. **GAP-16** — Provisioning health gate: `activate_tenant` polls per-module ready (depends on GAP-31).
12. **GAP-22** — Production composite WO: move BOM fetch before TX, add Numbering idempotency key, add failure injection test.
13. **GAP-17** — Bulk import for COA, items, vendors, customers (4 most-needed).
14. **GAP-29** — Audit crate wiring: instrument AP/AR/GL/Production with `platform_audit::writer` calls.

### Wave 3 (P2 — hardening)
13. **GAP-05** — AP/AR period pre-validation + e2e proof of GL rejection path.
14. **GAP-07** — ECO enforcement: block WO from consuming superseded BOM revs. Aerospace compliance.
15. **GAP-10** — Distributed tracing: span propagation across HTTP service calls with tenant_id in claims.
16. **GAP-14** — Explicit event dedup windows per stream.
17. **GAP-15** — Migration safety test pattern extracted and applied to every proven module.
18. **GAP-08** — Cross-service rate limiting activation.
19. **GAP-19** — SLOs + alert rules + runbook.
20. **GAP-23** — Pool config standardized per workload class.
21. **GAP-25** — Payments health canary test.
22. **GAP-30** — Reconciliation jobs for AR/AP/GL/Inventory/Production.
23. **GAP-36** — Dependency vulnerability scanning in CI.

### Wave 4 (P3-P4 — future)
20. **GAP-18** — Secret rotation runbooks.
21. **GAP-09** — Carrier adapter opt-in docs.
22. **GAP-20** — Tenant-local time zone for reports + periods.
23. **GAP-21** — GDPR erasure + soft delete standard.
24. **GAP-24** — Compatibility matrix.
25. **GAP-12** — Postgres RLS evaluation.

## Test philosophy

- **Real services, no mocks, no stubs** — every bead ships with a test that runs against real Postgres, real NATS, real dependencies.
- **Failure injection** — composite endpoints and cross-module flows must have tests that deliberately fail at each boundary and assert no data inconsistency.
- **Canary tests** — for silent bug classes (nil-tenant, pool starvation, dedup misses), add canary tests that detect the bug within 60s of onset in CI.
- **Full-cycle e2e** — at least one test per wave exercises the realistic customer path end-to-end, not just single endpoints.
- **No "pre-existing issue"** — any bug found during gap work gets a child bead and gets fixed in scope.

## Scope fences (explicit anti-goals for this plan)

These are NOT in scope for the bridge plan. They're real, but solving them is a separate initiative:

- **Frontend/UI** — TCP UI exists and has its own bead stream. Don't touch it here.
- **MRP / production scheduling** — Manufacturing roadmap explicitly excludes this.
- **Backflush / auto-component-issue** — Explicitly disallowed in v1.
- **Process manufacturing / recipe BOMs** — Out of scope.
- **Mobile apps** — Out of scope.
- **Cross-region deployment** — Future initiative.
- **NCR / CAPA lifecycle** — Phase C gives inspection + hold/release only.

## Success criteria for the bridge plan

The bridge plan is complete when:
1. All P0 gaps closed and verified with integration tests.
2. All P1 gaps have either (a) landed with tests or (b) explicit deferral with scope fence rationale.
3. `proof_platform_completion.sh` exits 0 against main.
4. A new vertical (hypothetical) can onboard by calling `POST /api/control-plane/tenants/{id}/provision` and receive a fully ready environment within 60s.
5. An integration test exists that runs the full WO→ship→invoice customer flow and validates state at every step.
6. Every silent bug class has a canary test that would have caught it.

## Refinement log

### Round 0 (initial) — 2026-04-11T02:15Z
Bridge plan drafted from reality check findings. 12 gaps. Surface-level acceptance criteria.

### Round 1 (ambition pass 1) — 2026-04-11T02:30Z
Added GAP-13 through GAP-24 (13 new gaps): contract drift, event dedup, migration safety, tenant bootstrap race, bulk import, secret rotation, SLOs, time zones, soft delete, composite txn boundaries, pool config, compat matrix, payments crash-loop. Added waves, scope fences, test philosophy, success criteria. 25 total gaps.

### Round 2 (ambition pass 2) — 2026-04-11T02:45Z
Added GAP-26 through GAP-38 (13 new gaps): backup/restore, graceful degradation, circuit breakers, audit trail, reconciliation, ready probe, dev env, logging standard, SLO budget, data export, vuln scanning, feature flags, noisy neighbor. 38 total gaps.

### Round 3 (refinement pass 1 — verification + acceptance sharpening) — 2026-04-11T03:10Z
Focus: verify P0 assumptions before committing to fixes. Sharpen acceptance criteria to be testable.

**Verifications performed:**
- **GAP-02:** Confirmed via Grep tool — 29 services use `image: 7d-runtime`, 0 have `command:` override. Bomb is real. Proposed specific 1-line Dockerfile fix using `$SERVICE_BINARY` env var.
- **GAP-25:** Direct curl to payments health times out (not empty-reply like BOM). Cross-watcher log shows only one restart. Payments has no `platform_client::<>` calls — NOT the same class as bd-g1fu1. Generated 4 hypotheses (conn leak, NATS hang, DB query stuck, stale binary). Split into diagnostic + fix beads.
- **GAP-04:** Designed concrete API shape, sync/async decision (async/poll), state machine (pending→migrating→seeding→warming→ready), RBAC permission, 6 acceptance tests.

**Acceptance criteria sharpened on:**
- GAP-01: Specified run mode (`--skip-perf --skip-e2e`), expected failures (bd-1vq9e response shapes), decision tree for each failure.
- GAP-02: Specific 1-line fix proposal, verification script.
- GAP-03: 5 concrete grep patterns, 10 canary test endpoints, CI integration.
- GAP-04: Full API design, state machine, RBAC, 6 test scenarios.
- GAP-25: Split into diagnostic bead (15min timebox) + fix bead.

**Open questions still to verify in future rounds:**
- GAP-05: What does "close a period" actually mean in the platform's accounting model? Need to read GL code before writing acceptance.
- GAP-07: What's the exact ECO state machine and which states should block WO creation?
- GAP-13: Does `openapi_dump` binary handle compile-failure gracefully (falls through vs hard-fails CI)?
- GAP-16: Can tenant activation actually wait for all 26 modules without a deadlock risk (activation worker is itself one of the modules)?
- GAP-22: List every composite endpoint in the codebase and classify by risk tier.
- GAP-26: What's the actual backup mechanism — pg_dump, pgBackRest, WAL-G?
- GAP-29: Does `platform/audit` crate have a real schema + API, or is it a stub waiting for instrumentation?
- GAP-30: What are the actual reconciliation invariants per module? Some modules are clearer than others.

### Round 4 (refinement pass 2 — gap correction + scope narrowing) — 2026-04-13
Focus: answer all 8 open questions from Round 3 via direct code investigation.

**Verifications performed (answering all Round 3 open questions):**
- **GAP-05:** GL period close is comprehensively implemented — handlers, service (validation/snapshot/execution), period_repo enforcement, and multiple tests. Gap corrected: real issue is AP/AR don't pre-validate period status before accepting entries. Priority downgraded P1→P2.
- **GAP-07:** `bom_revision_id` stored on WOs but never validated against BOM revision status. `eco_service/service.rs:131` sets revision `superseded`, but `production/work_orders.rs:219` has no guard. Gap confirmed. Added specific acceptance criteria including 422 error format.
- **GAP-13:** `openapi_dump` is a standalone compile-time binary — no DB/NATS needed. Compile failure = drift caught. CI job approach is: build binary (compile gate), run + diff. No graceful-failure handling needed.
- **GAP-16:** `activate_tenant` (step 7) sets status='active' without polling per-module health. After activation, publishes `tenant.provisioned` which is async — modules process it after activation. Race window confirmed. Added concrete acceptance: step 7 must poll all 26 `/api/ready?tenant_id={id}` endpoints before setting active.
- **GAP-22:** Only ONE composite HTTP endpoint found: `POST /api/production/work-orders/create`. Shipping-receiving's bd-6pyqw was guards/events, not a composite HTTP endpoint. Gap scoped down significantly. Added specific transaction failure mode analysis.
- **GAP-25:** Self-recovered (payments healthy as of ~2026-04-12). Downgraded P0→P2. Now focused on canary test only.
- **GAP-26:** `scripts/backup_all.sh` (pg_dump) and `scripts/dr_drill.sh` exist in repo. Gap corrected: not "no backup" but "backup not scheduled." Priority unchanged (P0 — every day without automated backup = data-loss exposure for Fireproof customer).
- **GAP-29:** `platform/audit` crate is real and complete — schema, writer, policy, outbox_bridge, diff, actor, 5 integration test files. But `grep` found ZERO callers in production modules. Gap confirmed and scoped: crate exists, nothing uses it.
- **GAP-30:** Enumerated specific reconciliation invariants per module: AR (header total = line sum + tax), AP (aging buckets sum to total payables), GL (debit = credit per journal, period hash verification), Inventory (on-hand = receipts - issues), Production (output qty, component issue completeness), BOM (revision status consistency).

**Corrections to priority matrix:**
- GAP-25: P0→P2 (self-recovered)
- GAP-05: P1→P2 (GL close implemented; only AP/AR guard + e2e proof remaining)
- GAP-26 promoted to Wave 1 (every day without automated backups is live data exposure for Fireproof customer)
- GAP-29 promoted to Wave 2 (audit crate exists but zero callers = compliance gap)
- GAP-36 added to Wave 3

**Open questions for Round 5 (pre-bead-generation verification):**
- GAP-02: IndigoHawk retracted bd-nw7th.1 with "supervisord is target state" — does this mean Dockerfile CMD should use supervisord, not $SERVICE_BINARY? Need to check current infra/Dockerfile.runtime vs the retraction note to determine if fix should be forward or revert.
- GAP-29: Does identity-auth use platform/audit? Check `modules/identity-auth/Cargo.toml`.
- GAP-04: What RBAC permission currently guards `POST /api/control/tenants`? Check middleware stack in create_tenant handler.
- GAP-36: Check if `cargo-audit` or `cargo-deny` is already configured anywhere (`.cargo/deny.toml`, `.cargo/audit.toml`).
- GAP-31: Is there already a `/api/ready` endpoint in any module beyond the basic DB check? Check `platform/health/src/`.
- GAP-27: Are there any modules that already declare dep classifications in module.toml? Would establish prior art.

### Round 5 (refinement pass 3 — pre-bead final verification) — 2026-04-13
Focus: answer all Round 4 open questions, resolve remaining unknowns before bead generation.

**Verifications performed:**
- **GAP-02:** `infra/Dockerfile.runtime` confirmed DELETED by bd-d77cl.1 ("delete obsolete dev image recipes"). `docker-compose.services.yml` uses `flywheel/rust-dev-runtime:2026-04` (external image — UNAFFECTED). Only `docker-compose.cross.yml` uses `image: 7d-runtime` (dev workflow). Blast radius corrected: dev workflow only, not production. `infra/supervisord.conf`, `infra/dev-entrypoint.sh`, `infra/watch-binary.sh` still exist — only the Dockerfile is missing. Fix: recreate it as a supervisord-based dev image. Priority lowered P0→P1.
- **GAP-29:** Identity-auth does NOT use platform/audit (checked `modules/identity-auth/Cargo.toml` — no audit dep). Zero callers confirmed across all modules.
- **GAP-04 RBAC:** `platform/control-plane/src/handlers/create_tenant.rs` has ZERO RBAC middleware. Any valid JWT can create tenants. Escalated to security gap — added as P0 Wave 1 item (separate from the rest of GAP-04 which stays P1).
- **GAP-36:** No `deny.toml`, no `audit.toml`, no `.cargo/deny` anywhere. Zero cargo security scanning configured. Gap confirmed clean-slate.
- **GAP-31:** `platform/health/src/lib.rs` has `/api/ready` but NO `?tenant_id` parameter. Global readiness only. Gap confirmed — adding tenant-aware ready endpoint is required before GAP-16 can be implemented. Dependency chain: GAP-31 → GAP-16.
- **GAP-27:** No modules declare dep criticality in `module.toml`. Clean slate design — no prior art to build on.

**Final priority matrix corrections:**
- GAP-02: P0→P1 (dev-only blast radius, not production)
- GAP-04 RBAC portion: promoted to Wave 1 P0 (security gap — unauthorized tenant creation)
- Wave 1 now: GAP-01 + GAP-03 + GAP-26 + GAP-04-RBAC
- Wave 2 sequencing: GAP-31 must precede GAP-16 (dependency confirmed)
- GAP-36 confirmed needing implementation (no existing config)

**Plan is now ready for bead generation.** All open questions answered, all acceptance criteria are testable, all file paths verified against codebase.
