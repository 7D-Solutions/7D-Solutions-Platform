# workflow — Revision History

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
| Proof | (Gate 1) | `scripts/proof_workflow.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.1.2 | 2026-03-31 | bd-vnuvp.7 | Add tenant_id filter to all escalation queries: fire_timer guard/rule/count/update, arm_timer/arm_timer_with_due_at existing checks, get_timer, tick_inner cancel. Removed unused global tick() that bypassed tenant isolation. | Tenant isolation bug: 4+ queries on escalation_timers and escalation_rules could cross tenant boundaries. | No |
| 2.1.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 2.1.0 | 2026-03-30 | bd-2oyhr | OpenAPI via utoipa: 8 annotated handlers, /api/openapi.json endpoint (6 paths, 13 schemas), openapi_dump binary, ToSchema derives on all API types, IntoParams on query params. | Consumers can generate typed clients from the served spec. | No |
| 2.0.0 | 2026-03-30 | bd-2oyhr | Standard response envelopes: list_definitions and list_instances return PaginatedResponse with pagination metadata. All errors use ApiError with request_id from TracingContext. Replaced ErrorBody with platform-http-contracts ApiError. Added count queries for proper pagination totals. | Breaking: consumers must handle new paginated envelope on list endpoints and new ApiError shape on errors. | YES: list endpoints return `{"data":[...],"pagination":{...}}` instead of bare arrays. Errors return `{"error":"...","message":"...","request_id":"..."}`. |
| 1.0.0 | 2026-03-28 | bd-3eb22 | Initial proof. Workflow definition CRUD, instance lifecycle (create/advance/complete/cancel), step routing (sequential/parallel/conditional), decision recording, state machine transitions, admin endpoints, event publishing. 14 unit tests pass, clippy clean. | Workflow orchestration module complete and proven. All gates pass. | No |

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
