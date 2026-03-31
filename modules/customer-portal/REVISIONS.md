# customer-portal — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-03-28 | bd-1xe02 | Initial promotion. External customer auth boundary with RS256 JWT, Argon2 password hashing, refresh token rotation, tenant-isolated portal users, document visibility via doc-mgmt distribution check, status feed with acknowledgments, outbox event emission for all auth lifecycle events. Proof script, clippy clean, 5 tests pass (1 unit + 4 real-DB integration). Security audit: no blocking findings. | Production readiness gate for first paying customer (Fireproof ERP). | No |
| 1.0.1 | 2026-03-28 | bd-29c9i.3 | Admin routes (/portal/admin/*) now require customer_portal.admin permission instead of party.mutate. | Security audit: party.mutate conflated party record management with portal user administration — two distinct privilege scopes. | No |
| 2.0.0 | 2026-03-30 | bd-c2dnv | Standard response envelopes. All error responses migrated from inline json!() tuples to platform ApiError with request_id on every error path. List endpoints (status feed, documents) now return PaginatedResponse with page/page_size query params. AuthResponse (login/refresh) preserved as-is — not wrapped in PaginatedResponse. TracingContext threaded through all handlers. | Plug-and-play wave 2: platform-wide response contract conformance. | YES — error shape changed from ad-hoc JSON to `{error, message, request_id}`; list responses wrapped in `{data, pagination}`. Consumers must update error parsing and list response handling. |
