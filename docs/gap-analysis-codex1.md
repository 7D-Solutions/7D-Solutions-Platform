# Production Gap Analysis (Codex 1)

## 1) Top 5 Risks for First Customer

| Risk | Severity | Must-fix before go-live? | Evidence from codebase |
|---|---|---|---|
| **Release provenance drift: manifest-pinned deploy path is not actually wired to runtime compose services** | **Critical** | **Must-fix** | `scripts/production/deploy_stack.sh` writes per-service `*_IMAGE` vars to `.deploy.env` (`scripts/production/deploy_stack.sh:194-207`) but `docker-compose.platform.yml` and `docker-compose.services.yml` define services with `build:` and no corresponding `image: ${...}` consumption (`docker-compose.platform.yml:9,57,108`; `docker-compose.services.yml:9,56,129,...,866`). This can run stale/local images while appearing “promoted.” |
| **Production manifest still unresolved (pending tags), so deploy reproducibility is not yet real** | **High** | **Must-fix** | Production manifest entries still have `Git SHA` = `—` and `{sha}` placeholders (`deploy/production/MODULE-MANIFEST.md:12-17`). Validator explicitly skips pending entries unless strict (`scripts/production/manifest_validate.sh:7-8,75-89,113-116`). |
| **Large host attack surface + weak default secret fallbacks in compose** | **Critical** | **Must-fix** | Data stack publishes NATS and every Postgres DB port to host (`docker-compose.data.yml:11-13,37-38,63-64,91-92,...`). Service stack publishes many module API ports (`docker-compose.services.yml:133-134,182-183,269-270,312-313,...,876-877`). Many DB URLs/defaults include fallback passwords like `*_pass` (`docker-compose.services.yml:175,219,263,...`; `docker-compose.data.yml:42,68,96,...`). Docs rely on UFW as outer control (`docs/DEPLOYMENT-PRODUCTION.md:42-44,78`). |
| **Service-to-service trust is still shared-secret impersonation model** | **High** | **Ship-and-harden (short horizon)** | Security audit flags this as open (`docs/security-audit-2026-02-25.md:71-78`). Implementation confirms HMAC with shared `SERVICE_AUTH_SECRET` (`platform/security/src/service_auth.rs:3-4,137-143`). A single compromised service can mint tokens as peers. |
| **Detection/telemetry blind spot on control-plane latency; proof runbook emphasizes no-auth liveness over security behaviors** | **Medium** | **Ship-and-harden** | Control-plane metrics gap documented: no HTTP latency histogram / `/metrics` for SLO alerting (`docs/ALERT-THRESHOLDS.md:124-140`). Proof runbook captures health/ready/metrics “(no auth)” (`scripts/proofs_runbook.sh:5`) and focuses on service/container checks, not authz/adversarial scenarios. |

## 2) Must-Fix vs Ship-and-Harden

- **Blockers (must-fix):**
1. Wire manifest-pinned images into compose runtime (eliminate effective source-build drift in prod path).
2. Resolve production manifest placeholders to real immutable tags/SHAs; enforce strict validation.
3. Reduce exposed ports and remove insecure compose fallbacks for secrets/passwords.

- **Can ship with hardened follow-up (time-boxed):**
1. Replace shared-secret service auth with asymmetric service identity (or at minimum scoped per-service keys + audience checks as interim).
2. Close control-plane observability gap so billing/control-path latency regressions are alertable.

## 3) Minimum Additional Work to Ship (3-5 items)

1. **Make deploys truly immutable (P0, hours):** add `image:` fields for all prod-deployed services (or prod override file), consume `.deploy.env` `*_IMAGE` vars, and fail deploy if unresolved placeholders remain.
2. **Enforce strict manifest readiness (P0, <1 hour):** run `manifest_validate.sh --strict` in promotion/deploy path; block deploy when any pending entry exists.
3. **Close external exposure by default (P0, hours):** bind internal-only ports to `127.0.0.1` or remove host port mappings for module/DB services; keep public entry only through reverse proxy.
4. **Remove insecure secret defaults (P0, hours):** delete `:-*_pass` fallbacks in compose for production paths; fail fast at startup when required secrets are missing/placeholder.
5. **Control-plane SLO instrumentation (P1, hours):** add HTTP duration/error metrics and `/metrics` endpoint parity with other critical services.

## 4) Surprises (Good and Bad)

- **Good:** Tenant-isolation and RBAC issues from the February sweep are documented as resolved across modules (`docs/security-audit-2026-02-25.md`), and SQL injection audit reports no exploitable paths (`docs/sql-injection-audit-2026-02-25.md`).
- **Good:** Proof/hardening artifacts are substantial (real Postgres/NATS runs, stabilization baseline, runbooks).
- **Bad:** There is a material mismatch between release/deployment doctrine (immutable promotion, no rebuild drift) and current compose/deploy wiring (`docs/ops/RELEASE-PROMOTION.md:25-30` vs compose `build:` usage).
