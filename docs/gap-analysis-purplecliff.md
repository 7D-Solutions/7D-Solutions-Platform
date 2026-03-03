# Production Gap Analysis — PurpleCliff

> **Reviewer:** PurpleCliff
> **Date:** 2026-03-03
> **Scope:** Full codebase scan — Docker Compose, CI, security audits, deployment scripts, monitoring, backup, NATS, Dockerfiles
> **Context:** First paying customer (aerospace/defense manufacturing — Fireproof ERP)

---

## 1. Risk Table

| # | Risk | Severity | Must-Fix or Defer | Evidence |
|---|------|----------|-------------------|----------|
| R1 | **No alert delivery — Prometheus fires into the void** | HIGH | **MUST-FIX** | `infra/monitoring/prometheus.yml` has no `alertmanager_url` configured. `docker-compose.monitoring.yml` has no Alertmanager container. Alert rules exist in `infra/monitoring/alerts/*.yml` but nobody receives them. The entire alert system is decorative. |
| R2 | **NATS has zero authentication** | HIGH | **MUST-FIX** | `docker-compose.data.yml:10` — NATS started with just `-js -sd /data -m 8222`. No auth config, no ACLs, no TLS. Any container on the `7d-platform` Docker network can publish or subscribe to any subject. A compromised service could tamper with the entire event bus. For aerospace/defense, event integrity is non-negotiable. |
| R3 | **7 databases missing from backup and bootstrap** | HIGH | **MUST-FIX** | `scripts/production/backup_all_dbs.sh` covers 18 databases. Missing: maintenance, pdf-editor, shipping-receiving, numbering, doc-mgmt, workflow, workforce-competence. `scripts/production/ssh_bootstrap.sh` is also missing these 7 volume declarations. If any of these modules hold production data, it won't be backed up or survive a VPS rebuild. |
| R4 | **No inter-service TLS — all traffic is plaintext** | MEDIUM | Defer (ship-and-harden) | All HTTP between services is plain HTTP. NATS is plain TCP. Database connections use `sslmode=disable` (visible in `scripts/proofs_runbook.sh` URL construction). Docker bridge network isolation + UFW mitigates external exposure, but aerospace/defense compliance frameworks (NIST SP 800-171, DFARS) may require encryption in transit even on internal networks. |
| R5 | **Service-to-service auth uses shared HMAC secret** | MEDIUM | Defer (ship-and-harden) | `platform/security/src/service_auth.rs` — single `SERVICE_AUTH_SECRET` shared across all services. Already identified as M1 in `docs/security-audit-2026-02-25.md`, status OPEN. Compromise of one service exposes the shared secret to impersonate any other. Docker network isolation mitigates but doesn't eliminate. |

---

## 2. Prioritized Punch List (Minimum Work to Ship)

### Item 1: Wire Up Alertmanager (MUST-FIX, ~2 hours)

**What:** Add an Alertmanager container to `docker-compose.monitoring.yml` and configure `alertmanager_url` in `infra/monitoring/prometheus.yml`. Configure at least one notification channel (email or Slack webhook).

**Why:** Without alert delivery, the comprehensive alert rules (service-down, payment-unknown, invariant-failure, latency-SLO) are useless. You won't know if the billing spine goes down until a customer calls. This is the single highest-leverage fix — the rules already exist, they just need somewhere to send notifications.

**Scope:**
- Add `alertmanager` service to `docker-compose.monitoring.yml`
- Create `infra/monitoring/alertmanager.yml` with at least one receiver
- Add `alerting.alertmanagers` block to `infra/monitoring/prometheus.yml`
- Test by triggering a synthetic alert

### Item 2: Add NATS Authentication (MUST-FIX, ~3 hours)

**What:** Create a NATS server config file with user/password auth (at minimum) and mount it into the NATS container. Update all service `NATS_URL` env vars to include credentials.

**Why:** NATS is the nervous system of the platform. Every event (billing cycles, payment confirmations, inventory changes) flows through it. Without auth, any container on the network — including a compromised service or a debugging tool left running — can publish fake events or consume sensitive data. For an aerospace/defense customer, this is an audit finding waiting to happen.

**Scope:**
- Create `infra/nats/nats.conf` with auth block (user/password or token)
- Update `docker-compose.data.yml` to mount the config and use `--config`
- Add `NATS_USER` and `NATS_PASSWORD` to `scripts/production/env.example` and `secrets_check.sh`
- Update all service `NATS_URL` entries in `docker-compose.services.yml` to `nats://user:pass@7d-nats:4222`

### Item 3: Complete Backup Coverage (MUST-FIX, ~1 hour)

**What:** Add the 7 missing databases to `scripts/production/backup_all_dbs.sh` and the 7 missing volumes to `scripts/production/ssh_bootstrap.sh`.

**Why:** If any of these modules go into production use (and several — maintenance, pdf-editor, shipping-receiving — are in the services compose and will be running), their data won't survive a disaster. The backup pipeline is well-built but incomplete.

**Scope:**
- Add to `backup_all_dbs.sh` DB_CONFIGS array: maintenance, pdf-editor, shipping-receiving, numbering, doc-mgmt, workflow, workforce-competence
- Add corresponding env var names to `scripts/production/env.example` and `secrets_check.sh`
- Add 7 missing volumes to `ssh_bootstrap.sh` VOLUMES array
- Run `install_backup_timer.sh --dry-run` to verify

### Item 4: Alertmanager Route for First Customer (Ship-and-Harden, ~1 hour)

**What:** Configure a dedicated alert route for billing-spine alerts (AR, Payments, Subscriptions, TTP) that pages immediately, vs a slower route for non-critical modules.

**Why:** Not all alerts are equal. A billing service going down for the first paying customer is a P0. The timekeeping module being slow is a P2. The alert rules already distinguish billing-spine vs module severity (1 min vs 5 min thresholds in `service-down.yml`). The Alertmanager routing should match.

**Scope:** Part of Item 1 — just needs routing config in the alertmanager.yml.

---

## 3. Surprises (Good and Bad)

### Good Surprises

- **Security posture is strong.** The tenant isolation fix (C1) across all 18 modules is thorough — every module now derives tenant from `VerifiedClaims`. The SQL injection audit found zero vulnerabilities. RBAC is enforced on all mutation routes. This is unusually mature for a pre-production platform.

- **Production deployment tooling is comprehensive.** Manifest-driven deploys, proof gates (smoke + isolation + payment verification + rollback rehearsal), secrets validation, VPS hardening script with UFW/fail2ban/auditd — this is better than many shipped products. The `deploy/production/MODULE-MANIFEST.md` pattern with `manifest_diff.sh` is excellent.

- **The proofs runbook is real.** `scripts/proofs_runbook.sh` actually runs integrated tests against real Postgres and real NATS. 33/33 crates passing with zero mocks is a strong foundation. The CI pipeline has 20+ jobs including contract tests, cross-module boundary guards, event metadata linting, and file size enforcement.

- **Backup infrastructure exists and is well-designed.** Daily scheduled backups with dump → ship → prune pipeline, SHA-256 manifests, off-host storage support. Just needs the missing databases added.

- **Dockerfiles use proper multi-stage builds.** The `cargo-chef` pattern (plan → cook → build → slim runtime) is correct and produces minimal images. The runtime stage only includes the binary, migrations, and schemas.

### Bad Surprises

- **Alertmanager is completely absent.** The monitoring stack has Prometheus with good scrape configs, Grafana with 8 dashboards, and 4 alert rule files — but no Alertmanager. The whole alerting pipeline ends in a dead end. This is the most dangerous gap because it creates a false sense of security: the rules look comprehensive, but nobody would ever be notified.

- **CI only compiles 6 of ~18 modules.** The `ci.yml` workflow has `cargo check` jobs for auth, AR, subscriptions, payments, notifications, and GL. The other 12 modules (AP, treasury, fixed-assets, consolidation, timekeeping, party, integrations, TTP, maintenance, pdf-editor, shipping-receiving, inventory) are not built in CI. A compile-breaking change to any of these would only be caught locally.

- **E2E tests in CI reference legacy compose files.** The `e2e-happy-path` job uses `docker-compose.infrastructure.yml` and `docker-compose.modules.yml`, which are explicitly described as "Legacy files... superseded by the data/services/frontend split" in the main `docker-compose.yml` header. These may not match the current service definitions.

- **NATS monitoring port 8222 is exposed to host.** In `docker-compose.data.yml:13`, NATS exposes port 8222 to `0.0.0.0`. In production, UFW would block this, but in staging or any environment where UFW isn't active, the NATS monitoring API (which shows all stream data) is accessible from the network. Should bind to `127.0.0.1:8222:8222`.

---

## Summary

The platform is in remarkably good shape for a first customer deployment. The security audit remediation is complete, the test coverage is real (not mocked), and the deployment tooling is production-grade. Three items are genuine blockers:

1. **Wire up Alertmanager** — you have great alert rules but no delivery mechanism
2. **Add NATS auth** — the event bus is the spine of the platform and it's completely open
3. **Complete backup coverage** — 7 databases are not being backed up

Everything else (inter-service TLS, asymmetric service auth, CI coverage gaps) can ship-and-harden. The foundation is solid.
