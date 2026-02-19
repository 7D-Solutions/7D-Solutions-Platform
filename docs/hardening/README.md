# Hardening Phase (Phase 34)

This directory tracks the platform's ship-readiness evidence. Each gate must pass before production launch.

## Gate Areas

| Gate | Bead | Status |
|------|------|--------|
| Security sweep (request limits, timeouts, rate limits) | bd-3rti | pending |
| RBAC enforcement + deny-by-default | bd-2e03 | pending |
| Backup/restore tooling + automated verify | bd-tet1 | pending |
| Service SLO metrics sweep | bd-2o97 | pending |
| Auth hardening (JWT rotation, session limits) | bd-3cik | pending |
| TLS + cert pinning config | bd-2bte | pending |
| Secret scanning CI gate | bd-1q38 | pending |
| Dependency audit (cargo audit) | bd-3p4l | pending |
| Load test baseline (k6 or wrk) | bd-2lz8 | pending |
| DB connection pool tuning | bd-1lm4 | pending |
| Graceful shutdown + SIGTERM handling | bd-2s7k | pending |
| Health check endpoints (liveness + readiness) | bd-3n7r | pending |
| Structured log audit (no PII leakage) | bd-1v2x | pending |
| Go/No-Go decision report | bd-3g9f | pending |

## Evidence Location

- Runbooks: `docs/runbooks/`
- CI gates: `.github/workflows/hardening.yml`
- Benchmark reports: `tools/stabilization-gate/reports/`

## Definition of Done

All gates PASS, CI hardening workflow green, go/no-go report signed off.
