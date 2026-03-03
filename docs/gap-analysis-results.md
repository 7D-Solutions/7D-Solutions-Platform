# Production Gap Analysis Results

> **Date:** 2026-03-03
> **Scope:** 7D Solutions Platform — first customer deployment readiness (Fireproof ERP, aerospace/defense)
> **Method:** Direct codebase inspection of Docker Compose files, deployment scripts, security audits, observability config, versioning docs, and service catalog

---

## 1. Risk Table

| # | Risk | Severity | Verdict | Evidence |
|---|------|----------|---------|----------|
| R1 | **No Postgres backup automation in production** | CRITICAL | **Must-fix** | `scripts/production/backup_all_dbs.sh` and `install_backup_timer.sh` exist but the brief confirms Phase 65 backup/restore is still "in progress." No cron timer is installed by `provision_vps.sh`. A single disk failure with no running backup job loses all customer financial data. Aerospace customers will ask about RPO/RTO on day one. |
| R2 | **NATS has zero authentication or TLS** | HIGH | **Must-fix** | `docker-compose.data.yml` runs NATS with `["-js", "-sd", "/data", "-m", "8222"]` — no `--auth`, no `--tls*` flags, no `nats.conf` mounted. Any process on the Docker network can publish/subscribe to any subject. The monitoring port (8222) is also exposed. In production, a compromised container can forge events across every module. |
| R3 | **Symmetric service-to-service auth (shared HMAC secret)** | HIGH | **Ship-and-harden** | `platform/security/src/service_auth.rs` uses a single `SERVICE_AUTH_SECRET` for HMAC-SHA256. Compromise of any one service leaks the secret and allows impersonation of all others. Noted as M1-OPEN in the security audit. Acceptable for launch behind UFW + Docker network, but must migrate to per-service asymmetric keys before adding external integrations. |
| R4 | **No nginx rate-limiting on the auth load balancer** | MEDIUM | **Ship-and-harden** | `nginx/auth-nginx.conf` has no `limit_req_zone` or `limit_req` directives. The auth service has its own application-level rate limits (5 login/min/email, 10 lockout threshold), so the risk is mitigated but not eliminated — a volumetric attack against registration or token refresh endpoints could still exhaust auth-instance resources before the app-level limiter engages. |
| R5 | **E2E spoofed-header rejection test still pending** | MEDIUM | **Must-fix** | Security audit item P1 (`bd-3mwl`): "E2E test proving spoofed headers are ignored when JWT is present" is listed as PENDING. The code fix is done (all 18 modules derive tenant from `VerifiedClaims`), but for aerospace traceability the proof artifact must exist before go-live. |

---

## 2. Prioritized Punch List (Minimum Work to Ship)

### P1. Install and verify the production Postgres backup timer
**Scope:** 1–2 hours, one agent

The script `scripts/production/install_backup_timer.sh` exists. The work is:

1. Add a call to `install_backup_timer.sh` at the end of `provision_vps.sh` (or document it as a mandatory post-provision step in `DEPLOYMENT-PRODUCTION.md`).
2. Run a backup cycle on the staging VPS, confirm `.sql.gz` files and `manifest.json` are written with correct row counts.
3. Run `restore_all.sh` against a scratch Postgres to prove the backups are restorable.
4. Verify the prune policy (`backup_prune.sh`) runs and retains the configured window (30 days local).

**Why it's a blocker:** Financial data loss in aerospace is a contract-terminating event. RPO must be demonstrably bounded.

### P2. Enable NATS authentication in production
**Scope:** 2–3 hours, one agent

1. Create a `nats-server.conf` with token or NKey auth (token is simplest for single-cluster). Add a `NATS_AUTH_TOKEN` to the production secrets contract (`secrets.env`, `secrets_check.sh`).
2. Mount the config file in `docker-compose.data.yml`: `command: ["-c", "/etc/nats/nats.conf"]`.
3. Update every service's `NATS_URL` env var to include the token: `nats://token@7d-nats:4222`.
4. Add the token to `scripts/production/env.example` and the secrets check required-variable list.
5. Run `proofs_runbook.sh` to confirm all 33 crates still pass with the new auth URL.

**Why it's a blocker:** Without auth, any container on the Docker network can inject events into the billing spine. For an ERP handling invoices and payments, this is unacceptable.

### P3. Write and run the spoofed-header E2E test
**Scope:** 1–2 hours, one agent

1. Create an E2E test (`e2e-tests/tests/tenant_isolation_spoofed_header.rs` or add to existing isolation tests).
2. Test flow: authenticate as Tenant A, issue requests with `X-Tenant-Id: <Tenant-B-UUID>` and `X-App-Id: <Tenant-B-UUID>` headers, assert all responses use Tenant A's data (from JWT claims).
3. Cover at minimum: AR, GL, Payments, Subscriptions, TTP (the billing spine).
4. Add to the `proofs_runbook.sh` or the production proof gate so the evidence is captured automatically.

**Why it's a blocker:** The security audit's C1 fix is the single most important remediation in the platform's history. The proof must be executable and repeatable, not just a code review claim.

### P4. Configure alert receiver (Alertmanager notification channel)
**Scope:** 30 minutes, one agent

1. Edit `infra/monitoring/alertmanager.yml` to configure a real receiver (email or Slack webhook — whatever the ops team monitors).
2. Test-fire a test alert and confirm delivery.
3. Document the receiver in `docs/OBSERVABILITY-PRODUCTION.md`.

**Why it matters:** Alert rules exist (`service-down.yml`, `payment-unknown.yml`, `invariant-failure.yml`, `latency-slo.yml`) but Alertmanager has no receiver configured. Without this, alerts fire into a void. Not a hard blocker — you could ship without it and rely on manual `smoke.sh` checks — but configuring it takes 30 minutes and dramatically reduces mean-time-to-detect.

### P5. Complete the DLQ replay drill and outbox backlog alerting (Phase 65 in-progress items)
**Scope:** 2–3 hours, one agent

The brief states DLQ replay drill automation and outbox backlog alerting are "in progress." These are the event-bus durability proof points.

1. Finish the DLQ replay drill script and run it end-to-end.
2. Add an outbox-backlog Prometheus alert rule (similar pattern to `payment-unknown.yml`) that fires when `outbox_pending_count` exceeds a threshold for >5 minutes.
3. Add the outbox metric to a Grafana dashboard panel.

**Why it matters:** In an event-driven architecture, a stalled outbox means mutations happened but downstream consumers never learned about them. For an ERP where AR posts to GL via events, a silent outbox stall means the books are wrong.

---

## 3. Surprises

### Good surprises

- **Security audit thoroughness.** The C1 tenant-isolation sweep across all 18 modules is remarkably thorough — the verification sweep (`bd-ia5y`) went back and found 5 additional modules with residual violations after the initial fix. This is exactly the kind of rigor aerospace auditors want to see.
- **SQL injection audit is clean.** Zero vulnerabilities across 18 modules and all tooling. The `platform/projections/src/validate.rs` allowlist-plus-regex defense is well-designed.
- **Production deployment tooling is mature for this stage.** `manifest_validate.sh`, `manifest_diff.sh`, `rollback_stack.sh`, `proof_gate.sh`, and the GitHub Actions promote workflow with environment protection rules — this is a lot of deployment safety for a platform that hasn't shipped yet. The rollback rehearsal in the proof gate is an especially smart touch.
- **Health contract is consistent.** Every service implements `/healthz` and `/api/ready` with the same JSON shape, latency checks, and status semantics. The shared `platform/health` crate enforces this.
- **Dockerfiles use multi-stage builds with cargo-chef.** Dependency caching, minimal runtime images (`debian:bookworm-slim`), and built-in healthchecks. No bloat.

### Bad surprises

- **All 20+ Postgres databases default to trivial passwords** (`auth_pass`, `ar_pass`, `gl_pass`, etc.) in `docker-compose.data.yml`. The production secrets contract (`secrets_check.sh`) validates that real passwords are set on the VPS, but the defaults are baked into the compose file. If anyone ever runs the dev compose files on a network-accessible host without overriding the env vars, every database is wide open. This isn't a production blocker (the VPS uses `secrets.env`), but it's worth a bold comment in the data compose file and a CI lint that rejects known-weak defaults in non-dev environments.
- **NATS monitoring port (8222) is published to the host** in `docker-compose.data.yml` (`"8222:8222"`). In production, UFW blocks it, but the compose file itself doesn't restrict the bind address. If someone port-forwards or runs without UFW, the NATS monitoring API (which shows stream configs, consumer state, and message counts) is exposed. Bind to `127.0.0.1:8222:8222` in the compose file for defense-in-depth.
- **The control-plane service has hardcoded credentials** in `docker-compose.services.yml` (line 136: `postgres://tenant_registry_user:tenant_registry_pass@...`). Unlike every other service, it doesn't use env-var substitution with defaults. This means the production `secrets.env` variables for tenant_registry won't be picked up by the control-plane unless someone also fixes this line. This is a real bug — verify before first deploy.
