# control-plane — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
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
