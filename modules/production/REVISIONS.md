# production — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_production.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.2.2 | 2026-03-31 | bd-tbnqm.2.1 | Add missing ConflictingIdempotencyKey match arm to 5 From<DomainError> for ApiError conversions (WorkcenterError, TimeEntryError, DowntimeError, ComponentIssueError, FgReceiptError). | Compilation failure: new error variant added without updating error_conversions.rs. | No |
| 2.2.1 | 2026-03-31 | bd-xs0ry.1 | Add optional idempotency_key field to 7 POST endpoints (component-issues, fg-receipt, workcenters, time-entries/start, time-entries/manual, downtime/start, routings). New production_idempotency_keys table. Duplicate key with matching hash returns cached result; different hash returns 409 Conflict. ON CONFLICT (event_id) DO NOTHING on outbox INSERT for defensive safety. | Double-submit creates duplicate outbox events causing downstream inventory consumers to double-process stock issues and FG receipts. | No |
| 2.1.2 | 2026-03-31 | bd-vnuvp.9 | Add tenant_id filter to routing_steps query (via routing_templates subquery) and operations COUNT query. Defense-in-depth tenant isolation on 2 queries. | P0 tenant isolation sweep: queries must filter by tenant_id to prevent cross-tenant data leakage. | No |
| 2.1.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 1.0.0 | 2026-03-28 | bd-gbbus | Initial proof. Workcenter CRUD and deactivation, work order lifecycle, routing creation/revision/release, operation initialization/start/complete with predecessor enforcement, timer and manual time entries, downtime tracking, component issue and finished-goods receipt flows, tenant-scoped queries, and outbox event publishing. 56 integration tests pass, clippy clean. | Production execution module complete and proven for shop-floor workflows. All promotion gates pass. | No |
| 1.0.1 | 2026-03-28 | bd-29c9i.1 | Add RequirePermissionsLayer to all /api/production/* routes: mutate routes (POST/PUT) require production.mutate, read routes (GET) require production.read. Operational endpoints (/healthz, /api/health, /api/ready, /api/version, /metrics) remain ungated. | Production was the only module without permission gating — security audit finding. | No |
| 1.0.2 | 2026-03-30 | bd-lgsgm.1 | Remove old routings.rs file that conflicted with refactored routings/ directory module. The routings/ directory contains the same code split into types.rs and repo.rs with ToSchema derives for OpenAPI. | Rust E0761 dual-module conflict prevented compilation after plug-and-play refactor left both file and directory. | No |
| 2.1.0 | 2026-03-31 | bd-3noyh | Standard response envelopes and OpenAPI spec. Top-level lists (workcenters, routings, active downtime) return PaginatedResponse with page/page_size/total_items/total_pages. Sub-collections (operations, time entries, routing steps, workcenter downtime) return { data: [...] }. All errors use ApiError with error, message, request_id, details. Error conversions in domain/error_conversions.rs. utoipa::path on all 28 handlers. /api/openapi.json endpoint. openapi_dump binary. Bearer JWT SecurityScheme. | Plug-and-play: consistent pagination, error formats, and machine-readable OpenAPI spec for consumers. | YES: List endpoints return { data: [...], pagination: {...} } instead of bare arrays. Error responses include request_id field. Consumers must update response parsing. |

## How to read this table

- **Version:** The version in the package file (`Cargo.toml` or `package.json`) after this change.
- **Date:** When the change was committed.
- **Bead:** The bead ID that tracked this work.
- **What Changed:** A concrete description of the change. Name specific endpoints, fields, events, or behaviors affected. Do not write "various improvements" or "minor fixes."
- **Why:** The reason the change was necessary. Reference the problem it solves or the requirement it fulfills.
- **Breaking?:** `No` if existing consumers are unaffected. `YES` if any consumer must change code to handle this version. If YES, include a brief migration note or reference a migration guide.

## Rules

- Add a new row for every version bump. One row per version.
- Do not edit old rows. If a previous change is reversed, add a new row explaining the reversal.
- The commit that bumps the version in the package file must also add the row here. Same commit.
- If the change is breaking (MAJOR version bump), the "Breaking?" column must describe what consumers need to change.
