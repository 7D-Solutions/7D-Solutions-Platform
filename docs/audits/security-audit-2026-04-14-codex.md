# Security Audit Report: 7D-Solutions Platform

Date: 2026-04-14  
Scope: Static code audit of core source paths with targeted pattern scans (secrets exposure, SQL construction, CORS/authn posture, transport controls) and manual validation of high-signal matches.

## Summary

- Total findings: 3
- Critical: 0 | High: 0 | Medium: 1 | Low: 2

## Findings

### [Medium] Wildcard CORS remains possible outside strict production gating
- **Location:** `platform/platform-sdk/src/startup_helpers.rs`
- **Issue:** If `CORS_ORIGINS` is unset, fallback is `*`; non-development only logs a warning.
- **Why this matters:** Mis-set or missing `ENV` in production-like deployments can silently permit cross-origin requests broader than intended.
- **Recommendation:** Fail closed by default (`deny` without explicit origins), or hard fail whenever wildcard is requested unless an explicit `ALLOW_WILDCARD_CORS=true` break-glass flag is set.

### [Low] Dynamic SQL string assembly for inbox filtering
- **Location:** `modules/notifications/src/inbox/repo.rs`
- **Issue:** Query text is built via `format!` with a generated `where_clause`.
- **Why this matters:** Current implementation appears constrained to internal boolean flags + bound category parameter, but string-built SQL increases long-term injection risk if future filters include user-controlled fragments.
- **Recommendation:** Move to fixed query templates or structured query builder branches; keep all variable values bound parameters.

### [Low] Dynamic SQL for QBO entity query composition
- **Location:** `modules/integrations/src/domain/qbo/sync.rs`
- **Issue:** `SELECT * FROM {entity_type}` is assembled as a string.
- **Why this matters:** `entity_type` is currently sourced from internal constants (`CDC_ENTITIES`), so exploitability is low now; still a defense-in-depth concern if this source ever becomes externalized.
- **Recommendation:** Enforce allowlist at compile-time and validate before query construction (explicit match over known values) to preserve safety under future refactors.

## Strengths Observed

- No validated hardcoded production secrets in audited core paths.
- Strong multi-module pattern rejecting `CORS_ORIGINS=*` in production config paths.
- Good use of bound SQL parameters in most data paths.

## Suggested Next Steps

1. Harden SDK CORS fallback to fail-closed behavior.
2. Add lint/check rule banning new `sqlx::query(&format!(...))` in non-test code unless allowlisted.
3. Run dependency-level scan in CI (`cargo audit` via your sanctioned cargo wrapper) and attach results to this report.

## Audit Notes

- This report prioritizes high-confidence static issues; it does not include runtime penetration testing.
- Test fixtures/docs/example values were de-prioritized unless they could leak into runtime behavior.
