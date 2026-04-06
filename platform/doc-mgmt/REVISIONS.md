# doc-mgmt — Revision History

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
| Proof | (Gate 1) | `scripts/proof_doc_mgmt.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.1.0 | 2026-04-06 | bd-4l4mw | Add entity-agnostic attachment endpoints: POST /api/attachments (presigned PUT URL), GET /api/attachments/{id} (presigned GET URL), GET /api/attachments?entity_type=X&entity_id=Y (list), DELETE /api/attachments/{id} (soft delete). New `attachments` table (migration 006). AppState gains `blob: Arc<BlobStorageClient>`. blob-storage from_env now accepts BLOB_BUCKET alias and defaults BLOB_REGION to us-east-1. | Verticals need to attach photos and files to any entity without file bytes flowing through the service. | No |
| 1.0.1 | 2026-04-01 | bd-manm4 | Add utoipa dependency, openapi module with ApiDoc struct, and openapi_dump binary for standalone spec generation. Info-only spec (handler annotations pending). | OpenAPI spec hygiene — all modules must emit complete specs for client codegen. | No |
| 1.0.0 | 2026-03-28 | bd-2j13q | Initial proof. Document metadata storage, template rendering with deterministic hashing, document lifecycle management, tenant-scoped queries. 5 unit tests pass, clippy clean. | Document management module complete and proven. All gates pass. | No |

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
