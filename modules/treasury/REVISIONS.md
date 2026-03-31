# treasury - Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.1.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 1.0.0 | 2026-03-28 | bd-14lud | Initial proof baseline for treasury covering bank account management, transaction import, reconciliation, and cash position reporting. Added the module proof script used by the promotion gate. | Establish the first proven release after package tests and module proof passed end-to-end. | No |
| 1.0.1 | 2026-03-28 | bd-170ei | Replaced f64 arithmetic with rust_decimal::Decimal in parse_amount functions across parser.rs, chase.rs, and amex.rs. Eliminates IEEE 754 rounding errors when converting monetary strings to minor units. | f64 multiplication can silently round certain decimal values incorrectly (e.g. 1.005 * 100 = 100.49… → 100 instead of 101). Financial code requires exact decimal arithmetic. | No |
| 2.0.0 | 2026-03-31 | bd-tufn5 | All error responses now use platform-http-contracts ApiError with request_id from TracingContext. Removed per-handler ErrorBody/ReconErrorBody/ImportErrorBody types. Deleted admin_types.rs. list_accounts returns PaginatedResponse with page/page_size params. Added count_accounts and list_accounts_paginated to service layer. | Consistent error envelope and pagination across all platform modules. Breaking: list_accounts response shape changed from array to {data, pagination} envelope. | YES — consumers of GET /api/treasury/accounts must unwrap the `data` field from the paginated envelope. |
| 2.1.0 | 2026-03-31 | bd-tufn5 | Added OpenAPI 3.1 spec: 16 utoipa-annotated handlers, /api/openapi.json endpoint (15 paths, 31 schemas), openapi_dump binary, SecurityAddon for Bearer JWT. ToSchema/IntoParams derives on all API types. | Self-documenting API for client code generation and developer portal. | No |

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
