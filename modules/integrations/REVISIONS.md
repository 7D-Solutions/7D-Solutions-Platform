# integrations — Revision History

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
| Proof | (Gate 1) | `scripts/proof_integrations.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-03-28 | bd-1rdqj | Initial proof. External refs CRUD with outbox events. Webhook ingest (Stripe, GitHub, QuickBooks, internal). QBO CDC/webhook normalization with realm→tenant resolution. OAuth connection management with encrypted tokens. Outbox relay with retry/DLQ. EDI transactions. File jobs. Outbound webhooks with delivery logging. 145 integrated tests against real Postgres. | Platform integrations layer ready for production. Handles external system connections, webhook routing, and event publishing. | No |
| 1.0.1 | 2026-03-29 | bd-ym43b | Add `POST /api/integrations/qbo/invoice/{invoice_id}/update` endpoint for sparse-updating QBO invoice shipping fields (ShipDate, TrackingNum, ShipMethodRef). Uses platform OAuth connection, handles SyncToken concurrency via QboClient retry loop. Gated by `integrations.mutate` permission. | Huber Power Phase 1 write-back requires outbound QBO invoice updates with shipping data. | No |
| 2.0.0 | 2026-03-30 | bd-hmoua | Migrate all HTTP handlers from ErrorBody to ApiError (platform-http-contracts). Wrap 3 list endpoints (list_by_entity, list_connector_types, list_connectors) in PaginatedResponse envelopes. Add platform-http-contracts and utoipa dependencies. Remove unused imports. | Plug-and-play standardization: uniform error shapes and paginated list responses across all platform modules. | YES: Error responses change shape from `{"error":"..."}` to `{"status":N,"code":"...","message":"..."}`. List endpoints now return `{"items":[...],"page":N,"page_size":N,"total":N}` instead of bare arrays. |

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
