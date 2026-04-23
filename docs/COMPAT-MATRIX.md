# Compatibility Matrix

Every time a module or platform component version is bumped, the agent doing the bump must add a row here before the PR merges. The `compat-matrix-gate` CI job enforces this.

See [VERSIONING.md](VERSIONING.md#compatibility-matrix) for the full obligation.

| Module | Version | Min Frontend Version | Notes | Date |
|---|---|---|---|---|
| pdf-editor | 2.3.4 | PDF-Creation >= Phase 0 (schemaVersion field, DRAW fix) | bd-884lm + bd-bt4yr | 2026-04-23 |
| integrations-rs | 2.35.0 | No frontend constraint — backend-only QBO webhook token API | bd-mmnbp + bd-24qxb + bd-c6z0t | 2026-04-23 |
| control-plane | 1.7.0 | PDF-Creation: add proxy rule for /api/features (port 8091) | bd-p2jsi | 2026-04-23 |
| pdf-editor | 2.3.3 | PDF-Creation: not yet established | Initial seed row — Fireproof Bubble Phase 0 baseline | 2026-04-22 |
