# Production Gap Analysis (Codex Review)

Date: 2026-03-03
Scope reviewed: compose files, env/config, production deploy scripts, security/sql audits, health/observability/deployment docs, Dockerfiles.

## 1) Top 5 Risks (with must-fix vs defer)

| Risk | Severity | Must-fix or Defer | Evidence from codebase |
|---|---|---|---|
| Secrets are committed in repo (`.env`) including payment credentials and JWT private key material. | Critical | Must-fix before first customer traffic | `.env` contains Tilled secret/account values and full PEM private key/public key at [`.env`](/Users/james/Projects/7D-Solutions Platform/.env:3), [`.env`](/Users/james/Projects/7D-Solutions Platform/.env:8), [`.env`](/Users/james/Projects/7D-Solutions Platform/.env:36). |
| Production release is not actually immutable/pinned in runtime compose path. Manifest entries are still placeholders and deploy scripts skip pending entries. | Critical | Must-fix | Production manifest rows are unresolved placeholders (`{sha}` / `—`) at [`deploy/production/MODULE-MANIFEST.md`](/Users/james/Projects/7D-Solutions%20Platform/deploy/production/MODULE-MANIFEST.md:12). `manifest_validate.sh` explicitly skips pending entries unless strict at [`scripts/production/manifest_validate.sh`](/Users/james/Projects/7D-Solutions%20Platform/scripts/production/manifest_validate.sh:74). Runtime compose files are `build:`-based, not image-pinned (`docker-compose.platform.yml`, `docker-compose.services.yml`, `docker-compose.modules.yml`). |
| Internal service-to-service auth uses one shared symmetric secret, allowing cross-service impersonation if one service is compromised. | High | Must-fix | HMAC signing with `SERVICE_AUTH_SECRET` in [`platform/security/src/service_auth.rs`](/Users/james/Projects/7D-Solutions%20Platform/platform/security/src/service_auth.rs:3) and [`platform/security/src/service_auth.rs`](/Users/james/Projects/7D-Solutions%20Platform/platform/security/src/service_auth.rs:139). Also listed as open in prior audit: [`docs/security-audit-2026-02-25.md`](/Users/james/Projects/7D-Solutions%20Platform/docs/security-audit-2026-02-25.md:91). |
| Default network exposure is broad: many app services and DB/NATS ports are host-published; a firewall/proxy misstep exposes internal plane directly. | High | Must-fix | Service ports published broadly at [`docker-compose.services.yml`](/Users/james/Projects/7D-Solutions%20Platform/docker-compose.services.yml:133), [`docker-compose.services.yml`](/Users/james/Projects/7D-Solutions%20Platform/docker-compose.services.yml:183), [`docker-compose.services.yml`](/Users/james/Projects/7D-Solutions%20Platform/docker-compose.services.yml:790). Data layer ports exposed at [`docker-compose.data.yml`](/Users/james/Projects/7D-Solutions%20Platform/docker-compose.data.yml:12), [`docker-compose.data.yml`](/Users/james/Projects/7D-Solutions%20Platform/docker-compose.data.yml:38), [`docker-compose.data.yml`](/Users/james/Projects/7D-Solutions%20Platform/docker-compose.data.yml:64). |
| Containers run as root by default across service Dockerfiles (no `USER` drop). This increases blast radius on runtime compromise. | Medium | Ship-and-harden (first hardening sprint) | Workspace/deploy Dockerfiles lack `USER` directives (e.g. [`platform/identity-auth/Dockerfile.workspace`](/Users/james/Projects/7D-Solutions%20Platform/platform/identity-auth/Dockerfile.workspace:1), [`modules/ar/deploy/Dockerfile.workspace`](/Users/james/Projects/7D-Solutions%20Platform/modules/ar/deploy/Dockerfile.workspace:1), [`modules/payments/Dockerfile.workspace`](/Users/james/Projects/7D-Solutions%20Platform/modules/payments/Dockerfile.workspace:1)). |

## 2) Prioritized minimum punch list (3-5 items)

1. **P0: Remove committed secrets and rotate now**
Scope: purge `.env` secrets from git history for customer-facing branches, rotate JWT keypair + Tilled keys, replace repo `.env` with `.env.example` placeholders, enforce pre-commit/CI secret scan.

2. **P0: Make production deploy truly immutable**
Scope: complete image build/push for all production services, replace `{sha}` placeholders in `deploy/production/MODULE-MANIFEST.md`, fail deploy on any pending entry, and ensure compose uses pinned `image:` refs (not local `build:`) in production path.

3. **P0: Reduce exposed attack surface in compose defaults**
Scope: remove host port mappings for internal-only services/DB/NATS in production files (or bind to `127.0.0.1` where needed), keep only reverse-proxy ingress ports externally reachable.

4. **P1: Replace shared symmetric service auth with asymmetric workload identity**
Scope: move `SERVICE_AUTH_SECRET` HMAC model to per-service keypairs (or mTLS + signed JWT), enforce issuer/audience/service binding, rotate keys per service.

5. **P1: Drop container privileges**
Scope: add dedicated non-root user in runtime stages for platform/module Dockerfiles, set ownership for app dirs, run with read-only rootfs where feasible.

## 3) What surprised me

- Good surprise: C1 tenant-isolation and H1 RBAC issues from the prior audit are documented as remediated across modules, and SQL injection audit quality is strong.
- Bad surprise: despite strong docs claiming manifest-pinned production, current manifest is still placeholder-heavy and compose definitions are still build-centric, so release immutability is not yet actually enforced end-to-end.
- Bad surprise: real-looking credentials/key material are currently committed in `.env`, which is the fastest way to create an avoidable incident.
