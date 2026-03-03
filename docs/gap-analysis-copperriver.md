# Production Gap Analysis — CopperRiver

> **Reviewer:** CopperRiver
> **Date:** 2026-03-03
> **Scope:** Full codebase scan against go-live readiness for first paying customer (Fireproof ERP, aerospace/defense)

---

## 1. Risk Table

| # | Risk | Severity | Must-Fix or Defer | Evidence |
|---|------|----------|-------------------|----------|
| R1 | **NATS has no authentication** — any container on the Docker network can publish/subscribe to the event bus without credentials | HIGH | **Must-fix** | `docker-compose.data.yml:8-11` — NATS command is `["-js", "-sd", "/data", "-m", "8222"]` with no auth config. No `nats.conf` file exists in the repo. |
| R2 | **No Alertmanager configured** — Prometheus alert rules exist but fire into the void; nobody gets notified of outages | HIGH | **Must-fix** | No `alertmanager.yml` or Alertmanager service anywhere in the repo. `docker-compose.monitoring.yml` defines only Prometheus and Grafana. `OBSERVABILITY-PRODUCTION.md` line 201 says "Configure an alert receiver" — still TODO. |
| R3 | **Database backups have no off-host destination** — `backup_all_dbs.sh` dumps to local VPS disk only | HIGH | **Must-fix** | `scripts/production/backup_ship.sh` requires `BACKUP_S3_BUCKET` or `BACKUP_SCP_HOST` env vars. Neither is documented as configured. Backups on the same disk as the databases = no protection against disk failure. |
| R4 | **Symmetric service-to-service auth (shared HMAC secret)** — any compromised service can impersonate any other | MEDIUM | Defer | `platform/security/src/service_auth.rs` — already tracked as M1 OPEN in `docs/security-audit-2026-02-25.md`. Blast radius is limited to the Docker bridge network which is firewalled. |
| R5 | **No TLS between services and Postgres** — all database connections are plaintext | MEDIUM | Defer | All `DATABASE_URL` values in `docker-compose.services.yml` and `docker-compose.data.yml` have no `sslmode` parameter. Postgres containers have no SSL certificates configured. Traffic stays on the `7d-platform` Docker bridge (not exposed to internet). |

---

## 2. Prioritized Punch List (Minimum Work to Ship)

### P1. Configure Alertmanager with a real notification channel

**Why it's critical:** Five sets of alert rules exist (`service-down.yml`, `payment-unknown.yml`, `invariant-failure.yml`, `latency-slo.yml`, `outbox-health.yml`) covering billing spine failures, payment stalls, invariant violations, and SLO breaches. None of them reach a human today. If the AR service goes down at 2am and a billing cycle stalls, nobody knows until the customer calls.

**Scope:**
- Create `infra/monitoring/alertmanager.yml` with at minimum an email or Slack receiver
- Add an Alertmanager container to `docker-compose.monitoring.yml`
- Add `alerting:` block to `infra/monitoring/prometheus.yml` pointing at Alertmanager
- Test by firing a synthetic alert

**Estimate:** Small bead, a few hours.

### P2. Configure off-host backup shipping

**Why it's critical:** The backup scripts are solid — `backup_all_dbs.sh` does per-database `pg_dump` with SHA-256 manifests, `install_backup_timer.sh` schedules daily runs, `backup_prune.sh` handles retention. But `backup_ship.sh` needs a destination. A VPS disk failure today = total data loss for an aerospace customer. That's lawsuit territory.

**Scope:**
- Pick an S3-compatible provider (DigitalOcean Spaces, Backblaze B2, AWS S3)
- Set `BACKUP_S3_BUCKET`, `BACKUP_S3_ENDPOINT_URL`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` in `/etc/7d/production/secrets.env`
- Run `backup_ship.sh` manually and verify files arrive
- Run `install_backup_timer.sh` to activate the daily pipeline
- Run a restore drill from the off-host copy

**Estimate:** Small bead, a few hours (mostly ops configuration, not code).

### P3. Enable NATS authentication

**Why it's critical:** NATS is the event bus carrying all inter-service events — billing cycles, payment notifications, GL postings, tenant lifecycle events. With no auth, any process on the Docker network can publish fake events. For aerospace, this breaks audit trail integrity. An attacker who gains access to one container (even a low-privilege one) could publish fabricated events that cascade through the system.

**Scope:**
- Create a `nats.conf` with at minimum a shared auth token (quick) or per-service credentials via accounts (better)
- Mount it into the NATS container via `docker-compose.data.yml`
- Update all services' `NATS_URL` to include the token: `nats://token@7d-nats:4222`
- Verify all services reconnect and events flow
- Add token to production secrets contract

**Estimate:** Small bead, a few hours.

### P4. Verify backup restore on a separate instance

**Why it's critical:** Having backups is necessary but not sufficient. Aerospace compliance requires proven restore capability. `scripts/production/restore_drill.sh` and `scripts/drills/jetstream_restore_drill.sh` exist, but the brief marks "DLQ replay drill automation" as in-progress. A restore drill against a fresh VPS (or at minimum a separate Docker Compose instance) proves the backups actually work.

**Scope:**
- Run `restore_drill.sh` with the output from `backup_all_dbs.sh`
- Verify data integrity post-restore (row counts, key records)
- Run the JetStream restore drill
- Document the result as a proof artifact

**Estimate:** Small bead, a few hours.

---

## 3. Surprises (Good and Bad)

### Good surprises

- **Security audit was thorough and all critical findings are resolved.** The C1 tenant isolation bypass was found across all 18 modules and fixed — every module now derives tenant identity from JWT `VerifiedClaims`. This is uncommonly disciplined for a platform this size. The SQL injection audit found zero vulnerabilities across all production SQL.

- **The CI pipeline is surprisingly mature.** 14 CI gates including cross-module boundary guard, migration versioning check, contract breaking-change gate, event metadata lint, no-panic lint on critical paths, REVISIONS.md completeness lint, and file size enforcement. This is production-grade CI.

- **Backup scripts are well-engineered.** Multi-stage pipeline (dump → ship → prune) with SHA-256 manifests, systemd timer or cron fallback, idempotent operation, and clear error reporting. They just need an off-host destination.

- **Production deployment is manifest-governed.** Immutable image tags (no `:latest`), proof gates (smoke + isolation + payment verification + rollback rehearsal), GitHub Actions environment protection rules, manifest-vs-running diff validation. This is a mature deployment pipeline.

- **Dockerfiles use multi-stage builds with cargo-chef.** Fast rebuilds, minimal runtime images (debian-slim with only ca-certificates, libssl3, curl). Good practice.

### Bad surprises

- **DLQ migration check is warning-only.** In `ci.yml:546-549`, the check for DLQ tables says `# TODO: Change to exit 1 once all modules have DLQ migrations`. Event-producing modules without a `failed_events` table will silently lose failed events. For aerospace where every event must be traceable, this should be a hard gate before go-live.

- **CI E2E tests reference legacy compose files.** The `e2e-happy-path` and other E2E CI jobs use `docker-compose.infrastructure.yml` and `docker-compose.modules.yml`, which the main `docker-compose.yml` header documents as "superseded by the data/services/frontend split." If the legacy files drift from the current structure, CI tests could pass locally but fail in CI or vice versa.

- **No Alertmanager at all.** The observability infrastructure (Prometheus, Grafana, dashboards, 5 alert rule files) is genuinely good — but the critical last mile of actually delivering alerts to humans is completely missing. This is a notable gap because everything else in the monitoring setup is production-quality.

---

## Summary

The platform is in strong shape. The security posture, test coverage, CI pipeline, and deployment tooling are all well above the bar for a first customer. The three must-fix items (Alertmanager, off-host backups, NATS auth) are all small, well-scoped work items that can ship in a day. Nothing here is a fundamental architecture issue — these are ops configuration gaps, not code problems.
