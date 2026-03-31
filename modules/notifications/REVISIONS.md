# Notifications — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.0.0 | 2026-03-30 | bd-dhozf | Standard response envelopes. All list endpoints (deliveries, inbox, DLQ) wrapped in PaginatedResponse. All error responses migrated from custom ErrorResponse/ErrorBody to ApiError. Templates handlers converted from tuple error returns to ApiError. Added count_receipts repo function for delivery pagination. Added ToSchema derive to DeliveryReceipt. Enabled utoipa chrono+uuid features. | Plug-and-play envelope standard — consistent paginated responses and error shapes across all endpoints. | YES — list endpoints now return `{data, pagination}` envelope instead of bare arrays. Error responses use `ApiError` shape (`{status, error, message, request_id}`). Consumers update response parsing accordingly. |
| 1.0.0 | 2026-03-28 | bd-4ym3v | Initial proof. All unit tests passing (32/32). Clippy clean. Proof script created. Covers: scheduled dispatch with retry, DLQ replay/abandon, inbox CRUD, broadcast fan-out, escalation rules, template rendering, event consumers (invoice issued, payment succeeded/failed, low stock), outbox publishing, Prometheus metrics. | Module build complete — event-driven notification delivery ready for production. | — |

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
