# shipping-receiving-rs — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.2.1 | 2026-03-30 | bd-nhmgu | Add openapi_dump utility binary for offline spec generation. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 1.0.0 | 2026-03-28 | bd-eexq4 | Initial proof. All tests passing. | Module build complete and core logic validated via tests. | No |
| 2.0.0 | 2026-03-30 | bd-y3hq2 | Replace ErrorBody with ApiError. Add PaginatedResponse to list_shipments. All error responses include request_id via TracingContext. Query params changed from limit/offset to page/page_size. | Plug-and-play Wave 2: standard response envelopes. | YES — list_shipments returns `{"data":[],"pagination":{}}` instead of bare array. Error responses now include `request_id` field. Query params changed from `limit`/`offset` to `page`/`page_size`. |
| 2.1.0 | 2026-03-30 | bd-y3hq2 | Add OpenAPI spec via utoipa. All handlers annotated with `#[utoipa::path]`. All types derive ToSchema/IntoParams. `/api/openapi.json` route serves OpenAPI 3.0 spec. SecurityAddon for Bearer JWT. | Plug-and-play Wave 2: OpenAPI documentation. | No |
| 2.2.0 | 2026-03-30 | bd-y3hq2 | Migrate config.rs to ConfigValidator. NATS_URL uses require_when for conditional validation. | Plug-and-play Wave 2: startup improvements. | No |

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
