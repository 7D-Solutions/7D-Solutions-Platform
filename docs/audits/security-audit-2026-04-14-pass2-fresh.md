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
