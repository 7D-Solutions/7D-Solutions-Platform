# workforce-competence — Revision History

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
| Proof | (Gate 1) | `scripts/proof_workforce_competence.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-03-28 | bd-2ca0f | Initial proof. Competence registration, acceptance authority grant/revoke, qualification tracking, training record management, idempotent operations, admin endpoints, event publishing. 5 unit tests pass, clippy clean. | Workforce competence module complete and proven. All gates pass. | No |
| 1.0.1 | 2026-03-30 | bd-d9iyz | Split service.rs into service/core.rs (commands) + service/queries.rs (reads). Split acceptance_authority.rs into acceptance_authority/grants.rs (grant+revoke) + acceptance_authority/checks.rs (authorization queries). Shared types in mod.rs with pub use re-exports. | Two files exceeded 500 LOC; split by separation of concerns (commands vs queries). | No |
| 2.0.0 | 2026-03-31 | bd-qyo4m | All 7 handlers migrated from inline json!() errors to ApiError with request_id on every error path. extract_tenant returns ApiError. error_conversions.rs maps ServiceError and GuardError to ApiError. Added platform-http-contracts dependency. | Plug-and-play standard response envelopes for consistent error format across platform. | YES: Error responses now return `{"error":"...","message":"...","request_id":"..."}` instead of ad-hoc JSON. Consumers parsing error bodies must update. |
| 2.1.0 | 2026-03-31 | bd-qyo4m | Added utoipa OpenAPI annotations to all 7 handlers. ToSchema derives on all request/response types. IntoParams on query param structs. SecurityAddon with Bearer JWT. /api/openapi.json endpoint serving ApiDoc spec. | Plug-and-play OpenAPI spec generation for consistent API documentation across platform. | No |
| 2.1.1 | 2026-03-31 | bd-nmykb.1 | Fixed missing ToSchema derives on all public model types (ArtifactType, CompetenceArtifact, RegisterArtifactRequest, OperatorCompetence, AssignCompetenceRequest, AuthorizationResult, AcceptanceAuthority, GrantAuthorityRequest, RevokeAuthorityRequest, AcceptanceAuthorityResult). Added missing #[utoipa::path] on all 7 handlers. Added IntoParams on query param structs. | Build failure: v2.1.0 OpenApi derive referenced types/paths without required trait implementations. | No |

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
