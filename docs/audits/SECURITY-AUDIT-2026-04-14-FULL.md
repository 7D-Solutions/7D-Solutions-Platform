# Full Security Audit (Passes 1–4): 7D-Solutions Platform

Date: 2026-04-14  
Audit mode: **Static review + targeted manual validation** (no live penetration testing in this engagement)

## How to read this document

This file is the **single shareable “full report”** for 7D-Solutions Platform. It includes **verbatim** Pass 1–4 writeups (as they existed in-repo on 2026-04-14), plus a short deduplicated executive summary.

Source filenames (for traceability):

- Pass 1: `docs/audits/security-audit-2026-04-14-codex.md`
- Pass 2: `docs/audits/security-audit-2026-04-14-pass2-fresh.md`
- Pass 3: `docs/audits/security-audit-2026-04-14-pass3-fresh.md`
- Pass 4: `docs/audits/security-audit-2026-04-14-pass4-fresh.md`

## Executive summary (deduplicated)

**Top risks (deduped):**

- **CORS defaults / wildcard posture** in `platform-sdk` (misconfiguration-dependent; Pass 1 + Pass 4).
- **Outbound HTTP / integration surface** should have an explicit SSRF policy (Pass 2).
- **CI “known passwords”** are fine for ephemeral CI only if prod/staging parity cannot drift (Pass 3).

**Deduplicated severity roll-up (best-effort):**

- Critical: 0
- High: 0
- Medium: 3 (CORS misconfig class; outbound policy class; CI drift class)
- Low: 4 (SQL string-building maintenance risk; import blast radius; token scopes; etc.)

## Pass 1 (verbatim): `security-audit-2026-04-14-codex.md`

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

## Pass 2 (verbatim): `security-audit-2026-04-14-pass2-fresh.md`

# Security Audit — Pass 2 (Fresh Eyes): 7D-Solutions Platform

Date: 2026-04-14  
Lens: **Trust boundaries** — outbound HTTP, file/import surfaces, SSRF-adjacent patterns, and “defense in depth” regressions.

## Summary

- Total findings: 2
- Critical: 0 | High: 0 | Medium: 1 | Low: 1

## Findings

### [Medium] Outbound integration HTTP surface is broad (SSRF / abuse amplification review needed)
- **Signal:** Multiple modules use `reqwest` for outbound calls (integrations, AR webhooks, imports, admin utilities).
- **Why this matters:** Any endpoint that accepts URLs, redirects, or “connector configuration” from operators becomes a classic SSRF pivot unless strictly allowlisted and validated.
- **Recommendation:** Maintain an explicit outbound URL policy (scheme/host allowlist, block RFC1918/link-local/metadata IPs), centralize HTTP client construction, and add integration tests for forbidden destinations.

### [Low] Bulk import endpoints amplify integrity risk (CSV/JSON parsing + large payloads)
- **Example reviewed:** `modules/gl/src/http/imports.rs` — accepts JSON/CSV with a row cap and validates before writes (good), but still a high-impact blast radius if parser edge cases slip through.
- **Recommendation:** Fuzz CSV/JSON import parsers; enforce strict byte limits before parsing; keep “validate all then write” invariant covered by property tests.

## Pass 2 verification notes

- This pass intentionally did **not** repeat Pass 1 CORS findings; it focused on different classes of risk.
- No live SSRF harness was executed; this is a **static** risk framing plus targeted file review.

## Pass 3 (verbatim): `security-audit-2026-04-14-pass3-fresh.md`

# Security Audit — Pass 3 (Fresh Eyes): 7D-Solutions Platform

Date: 2026-04-14  
Lens: **CI/CD + operational realism** — how the repo behaves in automation, what defaults leak, and where “works in CI” becomes “works in prod” accidentally.

## Summary

- Total findings: 2
- Critical: 0 | High: 0 | Medium: 1 | Low: 1

## Findings

### [Medium] CI workflows embed many fixed database passwords (`postgres`, module-specific passes)
- **Signal:** `.github/workflows/ci.yml` sets `POSTGRES_PASSWORD`, `PGPASSWORD`, and multiple `*_POSTGRES_PASSWORD` values to predictable literals for CI services.
- **Why this matters:** This is appropriate for ephemeral CI **only if** it cannot leak into production-like environments via copy/paste, templated deploys, or “compose parity” drift.
- **Recommendation:** Add an explicit guardrail doc + lint: “CI-only literals must never appear in production compose”; separate CI compose from prod/staging compose; rotate any shared dev passwords periodically.

### [Low] Secret references are mostly GitHub Actions `secrets.*` (good), but token scopes matter
- **Signal:** Workflow references `secrets.CRATE_REGISTRY_TOKEN`, Docker registry creds, etc.
- **Why this matters:** Token compromise impact depends on least-privilege scoping and rotation cadence, not just “not plaintext in repo”.
- **Recommendation:** Document minimum scopes per token, enforce rotation dates, and verify registry tokens are not reused across environments.

## Pass 3 verification notes

- This pass is intentionally **not** about application logic; it’s about the “everything around the code” attack surface.

## Pass 4 (verbatim): `security-audit-2026-04-14-pass4-fresh.md`

# Security Audit — Pass 4 (Audit-only “Convergence Read”): 7D-Solutions Platform

Date: 2026-04-14  
Goal: Re-check the **highest-risk prior finding** with a stricter “would this actually ship insecurely?” lens — **no code changes**, audit documentation only.

## What Pass 4 is (in audit mode)

This mirrors the **Verify / fresh re-read** step from `multi-pass-bug-hunting`, but **without** the fix/rescan/test loop: we only tighten exploitability reasoning and identify **deployment preconditions** that turn a “warning” into an incident.

## Re-read target: default CORS fallback behavior

### Prior concern (from Pass 2)

`platform-sdk` builds a permissive CORS layer when `CORS_ORIGINS` is unset (defaults to `*`) and only warns when `ENV != development`.

### Convergence read (tighter exploitability model)

```35:107:/Users/james/Projects/7D-Solutions Platform/platform/platform-sdk/src/startup_helpers.rs
pub(crate) fn build_cors_layer(manifest: &Manifest) -> CorsLayer {
    let env_val = std::env::var("ENV").unwrap_or_else(|_| "development".to_string());
    // ...
    let cors_env = std::env::var("CORS_ORIGINS").unwrap_or_else(|_| "*".to_string());
    // ...
    if is_wildcard && env_val != "development" {
        tracing::warn!(/* ... */);
    }
    let layer = if is_wildcard {
        CorsLayer::new().allow_origin(AllowOrigin::any())
    } else {
        let parsed: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        CorsLayer::new().allow_origin(parsed)
    };

    layer
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
        .allow_credentials(false)
}
```

### Updated assessment

- **Severity (audit): Medium → “Medium (misconfiguration-dependent)”**
- **Why not automatically Critical:** `allow_credentials(false)` reduces the classic “any origin + credentialed cookies” browser exploit class.
- **What still makes it real:** Wildcard origins broaden **browser-based attack surface** for APIs that rely on other controls (cookies not involved, but CSRF-like cross-site posting patterns, token exfiltration via XSS elsewhere, etc.). Also, operational mistakes (`ENV` unset in prod) can silently widen exposure.

### Concrete verification questions (for a security engineer agent)

- Is `ENV` guaranteed set in all production manifests (not just “usually”)?
- Are any browser clients relying on credentialed flows despite `allow_credentials(false)` being false (double-check any custom header auth patterns)?
- Is there an org-wide policy test that fails CI if `CORS_ORIGINS` is `*` outside dev?

## Pass 4 output

- **No new criticals found** on this convergence read; the primary risk remains **misconfiguration + missing guardrails**, not an obvious “always exploitable” code bug from this snippet alone.
