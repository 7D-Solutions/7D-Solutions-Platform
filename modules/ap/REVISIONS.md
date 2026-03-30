# ap — Revision History

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
| Proof | (Gate 1) | `scripts/proof_ap.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-03-28 | bd-3bwil | Initial proof. Vendor CRUD, bill lifecycle (open/matched/approved/partially_paid/paid/voided), PO management (draft/approved/cancelled/closed), 2-way and 3-way match engine, append-only payment allocations with row-locking, payment runs (build/execute with idempotent disbursement), payment terms (Net-N, discount schedules), receipt-link ingestion, AP aging reports (current/30/60/90+ buckets), tax quoting/commit/void via tax-core, outbox atomicity, tenant isolation, event envelope publishing, DLQ replay drill binary. 229 unit tests, 58 integration tests (real Postgres). | AP module complete and proven against real database. All gates pass. | No |
| 1.0.1 | 2026-03-30 | bd-ctx1n | Split 11 oversize source files under 500 LOC. `po/service.rs` split into `po/queries.rs` (reads) + `po/service_tests.rs` (tests). Same pattern applied to all other oversize files. | Enforce 500 LOC file-size limit for CI compliance. Internal refactor only — no API or behavior changes. | No |

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
