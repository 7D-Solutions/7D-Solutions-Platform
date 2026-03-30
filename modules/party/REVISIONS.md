# party — Revision History

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
| Proof | (Gate 1) | `scripts/proof_party.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-03-28 | bd-3l9cl | Initial proof. Party master data CRUD (customer/vendor/both), app-scoped isolation, deactivation, duplicate-name guard, address management, contact management, external ref linking, admin endpoints, outbox event publishing, bench binary. 17 unit tests pass, clippy clean. | Party module complete and proven. All gates pass. | No |
| 1.0.1 | 2026-03-30 | bd-ziqa6 | Split `party/service.rs` into `party/{create,query,update}` plus validation helpers, and reworked `contact_service` into dedicated query/mutation/guards modules so each file stays below 500 LOC while re-exporting the same APIs. | Followed CopperRiver's review direction to shrink oversized domain files while keeping the surface stable for downstream consumers. | No |
| 2.0.0 | 2026-03-30 | bd-ww1k4 | `GET /parties` and `GET /parties/search` return `PaginatedResponse<Party>` (`{data, pagination}`). Sub-collection lists (`/contacts`, `/addresses`, `/primary-contacts`) return `{data:[...]}` wrapper. All errors use `ApiError` from `platform-http-contracts`. `list_parties` takes `page`/`page_size` and returns `(Vec<Party>, i64)`. `search_parties` returns `(Vec<Party>, i64)`. Added `From<PartyError> for ApiError`, `utoipa::ToSchema` on `Party`, `DataResponse<T>` wrapper. | Standardize response envelopes — Party is the most-referenced module and must match the contract pattern used by Inventory and other proven modules. | YES: List/search return envelope objects instead of bare arrays. Consumers read `.data` for items, `.pagination` for metadata. Errors use `ApiError` shape. Service callers destructure `(vec, total)` tuple from `list_parties`/`search_parties`. |
| 2.1.0 | 2026-03-30 | bd-djp16 | Full OpenAPI spec via utoipa 5.x. `#[utoipa::path]` on all 19 endpoints (7 party, 7 contact, 5 address). `ToSchema` on all domain types (`PartyCompany`, `PartyIndividual`, `ExternalRef`, `PartyView`, `Contact`, `Address`, all request/response types). `IntoParams` on `ListPartiesQuery` and `SearchQuery`. Bearer JWT `SecurityScheme`. Serves `/api/openapi.json`. | Consumers were reverse-engineering the API from source. Machine-readable spec enables TS client generation and matches the pattern proven in Inventory. | No |

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
