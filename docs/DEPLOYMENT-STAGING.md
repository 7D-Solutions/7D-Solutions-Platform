# Staging Deployment

This document is the single source of truth for provisioning a fresh staging VPS
and running the full 7D Solutions Platform stack on it.

## Overview

The platform uses four Docker Compose files that must be started in order:

| File | Contents | Start order |
|------|----------|-------------|
| `docker-compose.data.yml` | NATS + all Postgres databases | 1 |
| `docker-compose.platform.yml` | Auth service (×2), control-plane, auth-lb | 2 |
| `docker-compose.yml` (includes services.yml) | All backend modules (AR, GL, AP, …) | 3 |
| `docker-compose.frontend.yml` | TCP UI (Next.js) | 4 |

All four share the `7d-platform` Docker bridge network (external, created by bootstrap).

## VPS Specification

| Resource | Minimum | Recommended |
|----------|---------|-------------|
| vCPU | 4 | 8 |
| RAM | 16 GB | 32 GB |
| Disk | 80 GB SSD | 160 GB SSD |
| OS | Ubuntu 24.04 LTS | Ubuntu 24.04 LTS |

Provider-agnostic. Tested on Hetzner CX41, DigitalOcean c-4, Linode Dedicated 16.

## Staging Base URLs

| Service | Port | Path |
|---------|------|------|
| TCP UI | 3000 | `/` |
| Auth (load balanced) | 8080 | `/api/health` |
| Control Plane | 8091 | `/api/ready` |
| AR | 8086 | `/api/health` |
| Subscriptions | 8087 | `/api/health` |
| Payments | 8088 | `/api/health` |
| Notifications | 8089 | `/api/health` |
| GL | 8090 | `/api/health` |
| Inventory | 8092 | `/api/health` |
| AP | 8093 | `/api/health` |
| Treasury | 8094 | `/api/health` |
| Fixed Assets | 8095 | `/api/health` |
| Consolidation | 8096 | `/api/health` |
| Timekeeping | 8097 | `/api/health` |
| Party | 8098 | `/api/health` |
| Integrations | 8099 | `/api/health` |
| TTP | 8100 | `/api/health` |

## Environment Variable Contract

All variables are documented in `scripts/staging/env.example`.
Copy it to `scripts/staging/.env.staging` and populate before deploying.
**Never commit `.env.staging` to git.**

### Required secrets (no defaults — must be set)

| Variable | Description |
|----------|-------------|
| `JWT_PRIVATE_KEY_PEM` | Ed25519 private key for JWT signing |
| `JWT_PUBLIC_KEY_PEM` | Ed25519 public key |
| `JWT_SECRET` | BFF session secret (32+ random bytes) |
| `*_POSTGRES_PASSWORD` | One per database (19 total) |

### Optional with defaults

| Variable | Default | Notes |
|----------|---------|-------|
| `JWT_KID` | `auth-key-1` | Rotate when cycling keys |
| `RUST_LOG` | `info` | Set `debug` for troubleshooting |

### Secret injection approach

Secrets are **never stored in the repo**. Recommended approaches (choose one):

1. **Local file:** Populate `.env.staging` locally, upload via `scp` in `deploy_compose.sh`.
2. **VPS secrets manager:** Store in Hetzner Vault / DO Secrets / HashiCorp Vault;
   export into shell before running deploy.
3. **CI environment variables:** Set as masked variables in GitHub Actions; the
   staging workflow reads them and writes a `.env` file on the VPS.

## Provisioning a New VPS

### Step 1 — Create the instance (manual)

1. Create a VPS with the recommended spec at your cloud provider.
2. Enable SSH key authentication. Disable password login.
3. Configure firewall: allow only SSH (22) from your IP; ports 3000 and 8080–8100
   for service access (restrict to known IPs for staging).
4. Note the public IP or hostname.

### Step 2 — Configure `.env.staging`

```bash
cp scripts/staging/env.example scripts/staging/.env.staging
# Edit .env.staging — set STAGING_HOST, STAGING_USER, all passwords, JWT keys
```

### Step 3 — Bootstrap Docker on the VPS

```bash
ssh user@your-staging-host 'bash -s' < scripts/staging/ssh_bootstrap.sh
```

This installs Docker Engine, creates the `7d-platform` network, and creates all
named volumes. Idempotent — safe to run multiple times.

### Step 4 — Clone the repo on the VPS

```bash
ssh user@your-staging-host
git clone git@github.com:your-org/7d-platform.git /opt/7d-platform
exit
```

### Step 5 — Deploy

```bash
bash scripts/staging/deploy_compose.sh
```

This:
1. Pulls latest code on the VPS
2. Uploads `.env.staging` as `.env` on the VPS
3. Starts data stack → waits 15s → starts platform → backend → frontend
4. Waits 30s, then runs smoke checks against all `/api/health` endpoints

## Ongoing Deployments (Re-deploy)

```bash
bash scripts/staging/deploy_compose.sh
```

Docker Compose will rebuild only changed services. Existing volumes (database
data) are preserved.

## Smoke Checks Only

```bash
bash scripts/staging/deploy_compose.sh --smoke-only
```

Curls `/api/health` (or `/api/ready` for control-plane) on every service and
reports pass/fail.

## Phase 43 Proof Gate

The proof gate is the authoritative staging health check. It runs three suites
in sequence and exits non-zero if any suite fails:

| Suite | What it checks |
|-------|---------------|
| `smoke` | `/healthz` + `/api/ready` for all services, TCP UI login page, key data endpoints |
| `isolation_check` | Cross-tenant denial assertions (tenant A/B cannot read each other's resources) |
| `payment_loop` | Invoice → Tilled webhook → AR posting + idempotency (replay must not duplicate) |

### Run locally against staging

```bash
# Required for all suites:
export STAGING_HOST=staging.7dsolutions.example.com

# Required for the payment loop suite:
export TILLED_WEBHOOK_SECRET=<your-tilled-webhook-secret>

# Optional — enables data content assertions in smoke (not just auth enforcement):
export SMOKE_STAFF_JWT=<staff-jwt>

bash scripts/staging/proof_gate.sh
```

Override log directory (default `/tmp/proof_gate_logs`):

```bash
PROOF_GATE_LOG_DIR=/tmp/my_gate_run bash scripts/staging/proof_gate.sh
```

Individual per-suite log files and a `proof_gate_report.txt` summary are written
to `PROOF_GATE_LOG_DIR`.

### Run in CI (promote.yml)

The proof gate runs automatically as part of `promote.yml` whenever a tag is
promoted to staging and `skip_smoke` is `false` (the default). Required GitHub
Secrets:

| Secret | Used by |
|--------|---------|
| `SMOKE_STAFF_JWT` | smoke suite — data content assertions |
| `TILLED_WEBHOOK_SECRET` | payment_loop suite — HMAC signature generation |

The proof gate logs are uploaded as a GitHub Actions artifact named
`proof-gate-logs-<tag>` (retained 90 days) and linked in the workflow summary.

## CI/CD Deploy (Immutable Images)

The CI path produces **immutable Docker images** tagged `{semver}-{git-sha7}` (e.g. `v0.5.0-abc1234`) — `latest` is never pushed.

**The manifest is the single source of truth.** `deploy_stack.sh` reads image tags exclusively from `deploy/staging/MODULE-MANIFEST.md`. No ad-hoc tag overrides are accepted in production.

### How it works

1. `git tag v0.5.0 && git push origin v0.5.0`
2. `release.yml` runs automatically: builds Rust binaries, builds + pushes Docker images, updates `deploy/staging/MODULE-MANIFEST.md` with resolved image tags, commits.
3. Gate 3 validates the manifest (all pinned tags exist in the registry).
4. Run `promote.yml` manually (Actions → Promote Artifacts → Run workflow → click **Run workflow**). The workflow calls `deploy_stack.sh` — no tag argument needed; it reads the manifest.

### Required GitHub Secrets (environment: `staging`)

| Secret | Description |
|--------|-------------|
| `STAGING_SSH_PRIVATE_KEY` | Private key matching the deploy user's `authorized_keys` on the VPS |
| `STAGING_HOST` | VPS hostname or IP |
| `STAGING_USER` | SSH user (e.g. `deploy`) |
| `STAGING_REPO_PATH` | Repo path on VPS (e.g. `/opt/7d-platform`) |
| `STAGING_SSH_PORT` | SSH port (default: `22`) |
| `DOCKER_USERNAME` | Registry username |
| `DOCKER_PASSWORD` | Registry password or token |

**Repository variable:** `IMAGE_REGISTRY` (default: `7dsolutions`)

### CLI deploy (emergency / local testing)

```bash
export STAGING_HOST=staging.7dsolutions.example.com
export STAGING_USER=deploy
export STAGING_REPO_PATH=/opt/7d-platform
export IMAGE_REGISTRY=7dsolutions

# Deploy exactly what the manifest declares — no tag argument
bash scripts/staging/deploy_stack.sh

# Dry-run to see what would happen
bash scripts/staging/deploy_stack.sh --dry-run

# Deploy a specific manifest (e.g. for testing a rollback manifest)
bash scripts/staging/deploy_stack.sh --manifest /path/to/MODULE-MANIFEST.md
```

> **Dev-only override (guarded):** To force a single tag across all services during local testing,
> set `DEPLOY_ALLOW_TAG_OVERRIDE=1` and pass `--tag-override <tag>`. This flag is rejected without
> the guard variable. Never use it in CI or against the real staging environment.

## Rollback (Manifest Selection)

Rollback = restore a prior `MODULE-MANIFEST.md` and re-deploy. Since the manifest is the deploy
contract, rolling back means deploying exactly what the prior manifest declared — all images are
immutable and still present in the registry.

Every successful deploy archives the manifest it used to the VPS at
`${STAGING_REPO_PATH}/.manifest-snapshots/`. The deployment log records the manifest hash and
snapshot filename.

### Step 1 — View deployment history and available snapshots

```bash
export STAGING_HOST=staging.7dsolutions.example.com
export STAGING_USER=deploy
export STAGING_REPO_PATH=/opt/7d-platform

bash scripts/staging/rollback_stack.sh --history
```

Sample output:
```
=== Staging deployment history ===
2026-02-20T14:00:00Z manifest_hash=a1b2c3d4e5f6 manifest_git_sha=def5678 snapshot=2026-02-20T14:00:00Z-MODULE-MANIFEST.md
2026-02-21T09:30:00Z manifest_hash=b2c3d4e5f6a1 manifest_git_sha=abc1234 snapshot=2026-02-21T09:30:00Z-MODULE-MANIFEST.md

=== Available manifest snapshots (/opt/7d-platform/.manifest-snapshots) ===
2026-02-21T09:30:00Z-MODULE-MANIFEST.md
2026-02-20T14:00:00Z-MODULE-MANIFEST.md
```

### Step 2 — Roll back

**Roll back to the immediately-preceding deployment (most common):**
```bash
bash scripts/staging/rollback_stack.sh --previous
```

**Roll back to a specific named snapshot:**
```bash
bash scripts/staging/rollback_stack.sh \
    --snapshot 2026-02-20T14:00:00Z-MODULE-MANIFEST.md
```

**Roll back to the manifest at a specific git commit:**
```bash
bash scripts/staging/rollback_stack.sh --manifest-sha def5678
```

Rollback runs manifest validation (Gate 3 pre-check) before deploying to confirm all images
in the rollback manifest still exist in the registry. If validation fails, the rollback is
aborted — inspect the registry before retrying.

Data volumes are preserved during rollback; only service containers are replaced.

### Step 3 — Commit the rollback manifest

After a successful rollback, the old manifest was used temporarily. Make the rollback permanent
by overwriting the current manifest with the prior one and committing:

```bash
# Copy the snapshot you rolled back to into the working tree
scp deploy@staging.7dsolutions.example.com:\
  /opt/7d-platform/.manifest-snapshots/2026-02-20T14:00:00Z-MODULE-MANIFEST.md \
  deploy/staging/MODULE-MANIFEST.md

# Review, then commit
git diff deploy/staging/MODULE-MANIFEST.md
git add deploy/staging/MODULE-MANIFEST.md
git commit -m "[rollback] Revert staging manifest to 2026-02-20 (def5678)"
git push
```

## Rollback (Source Build Path — legacy)

Use this path only if the CI image pipeline is unavailable.

1. SSH into the VPS: `ssh ${STAGING_USER}@${STAGING_HOST}`
2. `cd ${STAGING_REPO_PATH}`
3. `git checkout <previous-tag-or-sha>`
4. `bash scripts/staging/deploy_compose.sh` (rebuilds from source)

## Scripts Reference

| Script | Purpose |
|--------|---------|
| `scripts/staging/env.example` | Template for all env vars — copy to `.env.staging` |
| `scripts/staging/export_env.sh` | Source this to export `.env.staging` into current shell |
| `scripts/staging/provision_vps.sh` | Interactive guided walkthrough for first-time provisioning |
| `scripts/staging/ssh_bootstrap.sh` | Install Docker + create network/volumes on a fresh VPS |
| `scripts/staging/deploy_compose.sh` | Pull + build + start all four compose stacks (source build) |
| `scripts/staging/build_images.sh` | Build Docker images with immutable tags (CI step 1) |
| `scripts/staging/push_images.sh --confirm` | Push built images to registry (CI step 2) |
| `scripts/staging/list_versions.sh [--json]` | List resolved image tags for current HEAD |
| `scripts/staging/deploy_stack.sh` | Deploy manifest-pinned images to staging VPS (reads `deploy/staging/MODULE-MANIFEST.md`) |
| `scripts/staging/deploy_stack.sh --manifest <path>` | Deploy using an alternate manifest (e.g. rollback manifest) |
| `scripts/staging/rollback_stack.sh --previous` | Roll back to the manifest snapshot before the last deploy |
| `scripts/staging/rollback_stack.sh --snapshot <name>` | Roll back to a specific named manifest snapshot |
| `scripts/staging/rollback_stack.sh --manifest-sha <sha>` | Roll back to manifest at a prior git commit |
| `scripts/staging/rollback_stack.sh --history` | Show deployment log and available manifest snapshots |
| `scripts/staging/proof_gate.sh [--host H] [--secret S] [--jwt J]` | Phase 43 proof gate: smoke + isolation + payment loop |
| `scripts/staging/smoke.sh` | Health checks + data endpoints only |
| `scripts/staging/isolation_check.sh` | Multi-tenant denial assertions only |
| `scripts/staging/payment_loop.sh` | Payment money path + idempotency proof only |

## Platform-Local Parity

Staging intentionally mirrors local dev:
- Same Docker Compose files, same service names, same internal hostnames
- Same `7d-platform` network name
- Same port assignments
- Differences: real secrets, no `dev` env labels (labels are informational only)

Do **not** create staging-specific compose overrides unless absolutely necessary.
If a difference is needed, extend the base compose file with a
`docker-compose.staging.yml` override and document it here.
