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
