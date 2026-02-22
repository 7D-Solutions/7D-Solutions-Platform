# identity-auth — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-02-14 | bd-266g | Initial proof. JWT auth, RBAC, API key management, rate limiting, session management, tenant isolation, NATS event publishing, contract validation. | Module build complete. All E2E tests passing. | — |
| 1.1.0† | 2026-02-19 | bd-oht8 | Added RBAC data model: `migration 003_rbac.sql` creates `permissions` (global), `roles` (tenant-scoped), `role_permissions` junction, and `user_role_bindings` tables with indexes. Added `db/rbac.rs` with typed CRUD queries including `effective_permissions_for_user()` for JWT claims integration. | Phase 35 IAM/RBAC: needed persistent role and permission storage before JWT claims could embed entitlements. | No |
| 1.1.0† | 2026-02-19 | bd-3jvp | Expanded `AccessClaims` with `roles`, `perms`, `actor_type`, `app_id` (optional), and `ver` fields. Added `JwtVerifier`, `VerifiedClaims`, and `ActorType` to the `security` crate for real RS256 verification (replacing placeholder). Login and refresh handlers now resolve RBAC roles/perms from DB and embed them in access tokens. Actor types: `user`, `service`, `system` — aligned with `EventEnvelope` metadata. | JWT tokens were missing role/permission claims needed for authz decisions at module boundaries. | No |
| 1.1.0† | 2026-02-19 | bd-3lfe | Replaced in-memory semaphore with DB-backed seat leases: `migration 004_add_session_leases.sql` adds `session_leases` table (tenant_id, user_id, session_id FK, last_seen_at, revoked_at). Login enforces `active_seats < max_concurrent_sessions` atomically via advisory xact lock + count + insert in one TX. Refresh rotates lease to new token; logout revokes it. `MAX_CONCURRENT_SESSIONS` env var (default 5). | Phase 40: seat limits must survive pod restarts and work correctly under horizontal scaling. | No |
| 1.1.0† | 2026-02-19 | bd-2obz | Added `TenantRegistryClient` in `src/clients/tenant_registry.rs`: fetches `concurrent_user_limit` via HTTP GET `/api/tenants/{id}/entitlements` from control-plane with per-tenant TTL cache (DashMap, 60 s default). Fail-closed policy: deny login when no cached value and fetch fails. Grace period (300 s): stale cache used during outage. Added 3 new entitlement observability metrics. `TENANT_REGISTRY_URL` and `ENTITLEMENT_TTL_SECS` config vars. | Seat limit must come from tenant entitlements in tenant-registry rather than a static config value. | No |
| 1.1.0† | 2026-02-19 | bd-1l0u | Added tenant status gating to login and session refresh. Status rules: `trial`/`active` → allow; `suspended`/`deleted` → deny all logins; `past_due` → deny new logins only (existing sessions may refresh). Status fetched via `TenantRegistryClient`; metrics added for each denial path. | Suspended/deleted tenants were able to authenticate. | No |
| 1.1.0 | 2026-02-20 | bd-2a3q | Added `/healthz` (liveness) and `/api/ready` (standardized readiness JSON) endpoints. Existing `/health/live` and `/health/ready` kept for backward compatibility. New `health` crate dependency. | Platform-wide health endpoint standardization (Phase 42). This commit bumped the version to 1.1.0, formally covering all † entries above. | No |
| 1.2.1 | 2026-02-22 | bd-26ro | Remove unused `old_priv_pem` binding in `jwks_includes_both_keys_during_overlap` unit test. Test-only change; no runtime behaviour altered. | Compiler warning cleanup — unused variable introduced in 1.2.0. | No |
| 1.2.0 | 2026-02-22 | bd-2bc5 | Zero-downtime JWT key rotation overlap window. `JwtKeys::with_prev_key(prev_public_pem, prev_kid)` attaches a retiring RSA key; `decode_access_token()` tries the current key first then falls back to the previous key. `to_jwks()` now serves both keys so remote verifiers can fetch the new key ID before old tokens expire. `Config` reads `JWT_PREV_PUBLIC_KEY_PEM` and `JWT_PREV_KID` from env; `main.rs` calls `with_prev_key()` when both vars are set. Rotation is purely env-var-driven: set prev vars → rolling restart → wait one TTL cycle → clear prev vars → rolling restart. | P48-040: key rotation rehearsal required that the platform can rotate JWT keys without forcing a logout of all in-flight users. | No |

> **† Retroactive entries:** These five changes were committed on 2026-02-19 while the module was still at 1.0.0 and before the Gate 1 pre-commit hook was installed. No intermediate image was tagged. They are recorded here under 1.1.0 as required by the versioning standard (docs/VERSIONING.md). Backfilled by bd-mtpw.

## How to read this table

- **Version:** The version in `Cargo.toml` after this change.
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
