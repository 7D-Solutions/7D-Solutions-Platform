# control-plane — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.5.1 | 2026-04-10 | bd-505dg | Fix billing run integration test to use AppState::new() constructor (follow-up to 1.5.0 AppState refactor). | AppState struct literals broke after adding new fields in 1.5.0. | No |
| 1.5.0 | 2026-04-10 | bd-505dg | Add PLATFORM_TENANTS_CREATE permission (platform.tenants.create) to security crate. Wire RequirePermissionsLayer on POST /api/control/tenants with optional_claims_mw for JWT extraction. Integration test confirms 202 with permission, 403 without, 401 with no JWT. | SOC2/access-control gap — create-tenant was unauthenticated, any caller could provision a tenant. | No |
| 1.4.0 | 2026-04-10 | bd-k4c1h | Add `provisioning/worker.rs`: async per-module bundle worker. Each module provisioned independently (create DB → run migrations → seed → verify). Per-module status tracked in new `cp_tenant_module_status` table (pending/provisioning/ready/failed). `provision_tenant()` calls `seed_module_statuses()` before the 7-step sequence; `CREATE_TENANT_DATABASES` now drives all per-module work. Steps 3-5 (migrations, seed, verify) are idempotently skipped after the worker completes. `ModuleRegistry::from_configs()` added for test use. | Single 7-step sequence could not track partial failures per module — one failed module blocked all others. New worker provisions modules concurrently with independent retry paths. | No |
| 1.3.2 | 2026-04-11 | bd-d77cl | Add `/api/health` route as an alias for `/api/ready`. Same handler and response shape, different path. | Rust Service Container Spec (AgentCore `docs/rust-service-container-spec.md` §4) requires every HTTP service to serve `/api/health`. Control-plane has its own router (not platform-sdk ModuleBuilder) so the route had to be added manually. `/api/ready` preserved as a backwards-compat alias. | No |
| 1.3.1 | 2026-04-04 | bd-p5cnn | Fix PORT default 8092 to 8091 to match PLATFORM-SERVICE-CATALOG.md | Default port was wrong — control-plane should listen on 8091 per the service catalog. | No |
| 1.3.0 | 2026-04-02 | bd-fdvkw | Add `GET /api/service-catalog` endpoint. Returns module_code → base_url mappings from `cp_service_catalog` table. Replaces hardcoded env vars (AR_BASE_URL, TENANT_REGISTRY_URL, DOC_MGMT_BASE_URL) with a single queryable endpoint. New migration seeds all 26 platform modules. | Modules discover each other via hardcoded env vars — adding a module means updating env vars everywhere. A central catalog simplifies service discovery. | No |
| 1.2.1 | 2026-04-02 | bd-5a957 | Split `steps.rs` into `steps.rs` + `tracking.rs` to meet 500 LOC file size limit. No functional changes. | `steps.rs` was 543 LOC; platform requires <500 LOC per file. | No |
| 1.2.0 | 2026-04-02 | bd-5a957 | Add provisioning orchestrator. NATS consumer drives 7-step sequence (validate, create DBs, migrations, seed, verify connectivity, verify schemas, activate). Module registry loaded from env vars. Recovery poll for stuck tenants. New endpoints: `GET .../provisioning` (step status), `POST .../retry` (retry failed). Hook events at milestones for vertical participation. | Tenants created via API were never provisioned — stuck in `pending` forever. The orchestrator automates the full lifecycle. | No |
| 1.1.0 | 2026-04-02 | bd-cinhj | Wire provisioning outbox relay to NATS. New `outbox_relay` module polls `provisioning_outbox` for unpublished events and publishes to NATS. Relay is optional — only starts when `NATS_URL` env var is set. Added `event-bus` dependency and `NATS_URL` to docker-compose.services.yml. | Provisioning events written to outbox were dead code — no relay published them. Verticals listening for `tenant.provisioning_started` never received events. | No |
| 1.0.4 | 2026-03-06 | bd-ubp52 | Sanitize DB errors in all handlers (create_tenant, retention, platform_billing_run). Add DefaultBodyLimit (2MB) to router. | Security audit H3/M4: DB error details leaked in HTTP responses; no request body size limit. | No |
| 1.0.3 | 2026-02-25 | bd-2ivp | Added connection pool metrics (size, idle, active) to `/api/ready` response via `db_check_with_pool`. | Ops needs pool saturation visibility to detect connection exhaustion before it causes request timeouts. | No |
| 1.0.2 | 2026-02-25 | bd-289r | Fixed clippy warnings: removed empty lines after doc comments, simplified borrowed expressions. | Enable cargo clippy -D warnings in CI. | No |
| 1.0.1 | 2026-02-25 | bd-1uce | Added graceful shutdown with SIGTERM/SIGINT signal handling. Server now drains in-flight requests before closing DB pool on shutdown. | Zero-downtime deploys require graceful shutdown to avoid dropping in-flight requests. | No |
| 1.0.0 | 2026-02-21 | bd-qvbg | Initial proof. All 23 tests passing (unit + integration against real DB). Handles tenant create, platform billing run, retention policy, AR client, tenant-registry client. Proof command: `./scripts/proof_control_plane.sh` | Module build complete. Phase 44 Track B promotion. | — |

## How to read this table

- **Version:** The version in the package file (`Cargo.toml`) after this change.
- **Date:** When the change was committed.
- **Bead:** The bead ID that tracked this work.
- **What Changed:** A concrete description of the change. Name specific endpoints, fields, events, or behaviors affected.
- **Why:** The reason the change was necessary.
- **Breaking?:** `No` if existing consumers are unaffected. `YES` if any consumer must change code to handle this version.

## Rules

- Add a new row for every version bump. One row per version.
- Do not edit old rows. If a previous change is reversed, add a new row explaining the reversal.
- The commit that bumps the version in the package file must also add the row here. Same commit.
- If the change is breaking (MAJOR version bump), the "Breaking?" column must describe what consumers need to change.
