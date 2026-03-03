# Production Deployment

This document is the single source of truth for provisioning a fresh production VPS
and running the full 7D Solutions Platform stack on it.

## Invariant

Production must remain **compositionally identical to staging**:
- Same Docker Compose files and service names
- Same `7d-platform` Docker bridge network
- Same port assignments (service-to-service)
- Same named volumes

Differences are limited to host hardening (UFW, fail2ban, SSH hardening, auditd,
unattended upgrades) and secrets management. Do not introduce production-only
Compose overrides unless absolutely necessary; document any overrides here.

## VPS Specification

| Resource | Minimum | Recommended |
|----------|---------|-------------|
| vCPU | 4 | 8 |
| RAM | 16 GB | 32 GB |
| Disk | 80 GB SSD | 160 GB SSD |
| OS | Ubuntu 24.04 LTS | Ubuntu 24.04 LTS |

Provider-agnostic. Tested on Hetzner CX41, DigitalOcean c-4, Linode Dedicated 16.

## Compose Stack Order

| File | Contents | Start order |
|------|----------|-------------|
| `docker-compose.data.yml` | NATS + all Postgres databases | 1 |
| `docker-compose.platform.yml` | Auth service (×2), control-plane, auth-lb | 2 |
| `docker-compose.yml` (includes services.yml) | All backend modules | 3 |
| `docker-compose.frontend.yml` | TCP UI (Next.js) | 4 |

All four share the `7d-platform` Docker bridge network (external, created by bootstrap).

## Production Service Access

In production, **all external traffic goes through a reverse proxy (nginx) at ports 80/443**.
Service ports (3000, 8080–8100) are not accessible from the internet. UFW blocks them.

| Service | Internal port | External path (nginx proxy) |
|---------|---------------|-----------------------------|
| TCP UI | 3000 | `https://app.7dsolutions.com/` |
| Auth | 8080 | `https://app.7dsolutions.com/api/auth/` |
| Control Plane | 8091 | `https://app.7dsolutions.com/api/control-plane/` |
| AR | 8086 | internal only |
| Subscriptions | 8087 | internal only |
| Payments | 8088 | internal only |
| Notifications | 8089 | internal only |
| GL | 8090 | internal only |
| Inventory | 8092 | internal only |
| AP | 8093 | internal only |
| Treasury | 8094 | internal only |
| Fixed Assets | 8104 | internal only |
| Consolidation | 8105 | internal only |
| Timekeeping | 8097 | internal only |
| Party | 8098 | internal only |
| Integrations | 8099 | internal only |
| TTP | 8100 | internal only |
| Maintenance | 8101 | internal only |
| PDF Editor | 8102 | internal only |
| Shipping-Receiving | 8103 | internal only |

## Host Security Posture

After `ssh_bootstrap.sh` runs, the host has:

| Control | Setting |
|---------|---------|
| SSH password auth | Disabled |
| SSH root login | Disabled |
| SSH max auth tries | 3 |
| SSH login grace time | 30 s |
| UFW inbound | Deny all except SSH / 80 / 443 |
| fail2ban | SSH jail, 3 tries, 1 h ban |
| Unattended upgrades | Security-only, no auto-reboot |
| auditd | Auth, passwd, sudoers, SSH config, repo directory |

## Provisioning a New VPS

### Step 1 — Create the instance (manual)

1. Create a VPS with the recommended spec at your cloud provider.
2. Upload your SSH public key during VPS creation (do not enable password auth).
3. Note the public IP or hostname.

### Step 2 — Configure `.env.production`

```bash
cp scripts/production/env.example scripts/production/.env.production
# Edit .env.production — set PROD_HOST, PROD_INITIAL_USER, PROD_DEPLOY_USER,
# PROD_DEPLOY_KEY, PROD_REPO_PATH, PROD_SSH_PORT
```

### Step 3 — Run the provisioning script

```bash
bash scripts/production/provision_vps.sh
```

This guided script:
1. Verifies initial SSH access
2. Creates the `deploy` user and installs your SSH public key
3. Runs `ssh_bootstrap.sh` on the VPS (UFW, fail2ban, SSH hardening, Docker, volumes)
4. Prompts you to clone the repo manually
5. Prints next steps for secrets and first deploy

### Step 4 — Populate secrets

Create `/etc/7d/production/secrets.env` on the VPS and run the validation script.
See [Environment Contract](#environment-contract) for the full procedure.

### Step 5 — First production deploy

```bash
# Via GitHub Actions (recommended):
# Actions → Promote Artifacts → Run workflow → select environment: production → enter tag

# Or via CLI (emergency only):
export PROD_HOST=prod.7dsolutions.example.com
export PROD_USER=deploy
export PROD_REPO_PATH=/opt/7d-platform
export IMAGE_REGISTRY=ghcr.io/7d-solutions

# Update deploy/production/MODULE-MANIFEST.md with real image tags, then:
bash scripts/production/manifest_validate.sh
bash scripts/production/deploy_stack.sh --manifest deploy/production/MODULE-MANIFEST.md
```

## Environment Contract

Application secrets are stored on the VPS at a root-owned location. They are
**never committed to the repository** and never appear in CI logs.

### Secrets directory on the VPS

| Property | Value |
|----------|-------|
| Path | `/etc/7d/production/secrets.env` |
| Owner | `root:root` |
| Mode | `0600` |
| Readable by | root only (the deploy user uses `sudo` to load via `export_env.sh`) |

### Creating the secrets file (first-time, on the VPS)

```bash
# SSH into the VPS as deploy user
ssh -p 22 deploy@prod.7dsolutions.example.com

# Create the secrets directory and file as root
sudo mkdir -p /etc/7d/production
sudo install -m 0600 -o root /dev/null /etc/7d/production/secrets.env

# Populate — paste contents adapted from scripts/production/env.example
# (provisioning vars are not needed here; only DB passwords, JWT keys, JWT_SECRET)
sudo nano /etc/7d/production/secrets.env
```

### Loading secrets before a deploy

On the VPS, source `export_env.sh` with `sudo -E` so the deploy user's shell
receives the exported variables:

```bash
sudo -E bash -c 'source /opt/7d-platform/scripts/production/export_env.sh && \
  bash /opt/7d-platform/scripts/production/deploy_stack.sh \
    --manifest /opt/7d-platform/deploy/production/MODULE-MANIFEST.md'
```

### Validating secrets before deploying

```bash
# Run the secrets check (must be root or sudo to read the 0600 file)
sudo bash scripts/production/secrets_check.sh
```

The script asserts:
- File exists at `/etc/7d/production/secrets.env`
- Owner is `root` (uid 0)
- Mode is `0600`
- No `CHANGE_ME` placeholders remain
- No test/development secret patterns
- All 30 required variables are present and non-empty

Exit code 0 = ready to deploy. Non-zero = fix the reported issues first.

### Required secrets (full list)

| Variable | Description |
|----------|-------------|
| `JWT_PRIVATE_KEY_PEM` | Ed25519 private key for JWT signing |
| `JWT_PUBLIC_KEY_PEM` | Ed25519 public key |
| `JWT_KID` | Key ID (e.g. `production-key-1`) |
| `JWT_SECRET` | BFF session secret (32+ random bytes, hex) |
| `AUTH_POSTGRES_PASSWORD` | auth service DB |
| `AR_POSTGRES_PASSWORD` | AR module DB |
| `SUBSCRIPTIONS_POSTGRES_PASSWORD` | Subscriptions module DB |
| `PAYMENTS_POSTGRES_PASSWORD` | Payments module DB |
| `NOTIFICATIONS_POSTGRES_PASSWORD` | Notifications module DB |
| `GL_POSTGRES_PASSWORD` | GL module DB |
| `PROJECTIONS_POSTGRES_PASSWORD` | Projections DB |
| `AUDIT_POSTGRES_PASSWORD` | Audit DB |
| `TENANT_REGISTRY_POSTGRES_PASSWORD` | Tenant registry DB |
| `INVENTORY_POSTGRES_PASSWORD` | Inventory module DB |
| `AP_POSTGRES_PASSWORD` | AP module DB |
| `TREASURY_POSTGRES_PASSWORD` | Treasury module DB |
| `FIXED_ASSETS_POSTGRES_PASSWORD` | Fixed Assets module DB |
| `CONSOLIDATION_POSTGRES_PASSWORD` | Consolidation module DB |
| `TIMEKEEPING_POSTGRES_PASSWORD` | Timekeeping module DB |
| `PARTY_POSTGRES_PASSWORD` | Party module DB |
| `INTEGRATIONS_POSTGRES_PASSWORD` | Integrations module DB |
| `TTP_POSTGRES_PASSWORD` | TTP module DB |
| `MAINTENANCE_POSTGRES_PASSWORD` | Maintenance module DB |
| `PDF_EDITOR_POSTGRES_PASSWORD` | PDF Editor module DB |
| `SHIPPING_RECEIVING_POSTGRES_PASSWORD` | Shipping-Receiving module DB |
| `NUMBERING_POSTGRES_PASSWORD` | Numbering module DB |
| `DOC_MGMT_POSTGRES_PASSWORD` | Doc Management module DB |
| `WORKFLOW_POSTGRES_PASSWORD` | Workflow module DB |
| `WC_POSTGRES_PASSWORD` | Workforce Competence module DB |
| `SEED_ADMIN_PASSWORD` | Initial admin password for new tenant seed (see below) |

Variable names and DB usernames are documented in full in `scripts/production/env.example`.

### Secure Tenant Bootstrap

When a new tenant is provisioned, `seed_identity_module` creates an `admin@<tenant_id>.local`
credential in the auth database. It reads the password from `SEED_ADMIN_PASSWORD` at runtime and
**refuses to seed** if:

- The variable is unset or empty
- The value matches a known-bad default (`changeme123`, `password`, `admin`, etc.)

**To set `SEED_ADMIN_PASSWORD` on the VPS:**

```bash
# Generate a strong random password (do this offline or on the VPS)
openssl rand -base64 24

# Append to the secrets file (as root)
sudo sh -c 'echo "SEED_ADMIN_PASSWORD=<generated-password>" >> /etc/7d/production/secrets.env'
sudo chmod 0600 /etc/7d/production/secrets.env
```

**Rotation:** If you need to rotate the seed password (for future tenant provisions), update
`SEED_ADMIN_PASSWORD` in `/etc/7d/production/secrets.env`. Previously seeded credentials are
unaffected (they store bcrypt hashes and are not re-seeded on retry).

**Never commit `.env.production` or `secrets.env` to git.** Both are listed in `.gitignore`.

## Production Manifest

The file `deploy/production/MODULE-MANIFEST.md` is the **only authoritative source of image tags for production**. The deploy script reads this file; no ad-hoc tag overrides are permitted.

### Updating the manifest

After a successful staging proof gate and promotion approval:

```bash
# 1. Edit deploy/production/MODULE-MANIFEST.md — update version, SHA, full image tag columns.
# 2. Validate all images exist in the registry:
bash scripts/production/manifest_validate.sh

# 3. Detect any drift between manifest and what is running on production:
export PROD_HOST=prod.7dsolutions.example.com
export PROD_USER=deploy
bash scripts/production/manifest_diff.sh
```

### manifest_validate.sh

Checks that every non-pending image tag in the manifest can be found via `docker manifest inspect`.

```bash
bash scripts/production/manifest_validate.sh
# or, to also fail on pending (unresolved) entries:
bash scripts/production/manifest_validate.sh --strict
```

### manifest_diff.sh

SSHes into the production VPS and compares what is **actually running** against the manifest.
Exits non-zero on any mismatch or container not running.

```bash
export PROD_HOST=prod.7dsolutions.example.com
export PROD_USER=deploy
bash scripts/production/manifest_diff.sh deploy/production/MODULE-MANIFEST.md
```

## Ongoing Deploys

Production deployments are manifest-governed. Update `deploy/production/MODULE-MANIFEST.md`
with the promoted image tags, validate, then deploy.

### GitHub Actions Operator Flow (recommended)

Production deploys are gated by GitHub Actions environment protection rules
(configured in **Settings → Environments → production**). Any required reviewers
must approve before the job runs.

**Step-by-step:**

1. Update `deploy/production/MODULE-MANIFEST.md` with the promoted image tags and
   commit + push to main.
2. Go to **Actions → Promote Artifacts → Run workflow**.
3. Set inputs:
   - **deploy_target:** `production`
   - **tag:** _(leave blank — production reads from the manifest)_
4. Click **Run workflow**.
5. The job enters a pending state. Required reviewers (configured on the
   `production` environment) must approve before execution begins.
6. After approval, the workflow runs three gates automatically:
   - **Gate 3 (pre-deploy):** `manifest_validate.sh` — asserts all manifest
     image tags exist in the registry.
   - **Deploy:** `deploy_stack.sh --manifest` — pulls and restarts containers;
     smoke checks run via SSH from inside the VPS (ports are firewalled).
   - **Gate 3 (post-deploy):** `manifest_diff.sh` — compares running containers
     against the manifest; exits non-zero on any mismatch.
7. On success, a deployment record artifact is uploaded and retained for 365 days.

Secrets (`PROD_SSH_PRIVATE_KEY`, `PROD_HOST`, `PROD_USER`, etc.) are injected from
the `production` environment secret store and are never echoed in logs.

### CLI (emergency only)

```bash
export PROD_HOST=prod.7dsolutions.example.com
export PROD_USER=deploy
export PROD_REPO_PATH=/opt/7d-platform
export IMAGE_REGISTRY=7dsolutions

# Validate images exist before deploying:
bash scripts/production/manifest_validate.sh

# Deploy from manifest:
bash scripts/production/deploy_stack.sh --manifest deploy/production/MODULE-MANIFEST.md

# Verify running images match manifest:
bash scripts/production/manifest_diff.sh
```

Docker Compose replaces only changed containers. Postgres volumes are preserved.

## First Tenant and Admin Bootstrap

After the first successful production deploy, provision the initial platform admin account
and real production tenants. All tenant provisioning uses supported API flows (no direct
DB edits); platform admin RBAC setup uses the documented seed script which calls the
auth HTTP API for registration.

### Quick reference

```bash
export PROD_HOST=prod.7dsolutions.example.com

# 1. Provision 2 initial production tenants and validate:
bash scripts/production/provision_tenants.sh --host "$PROD_HOST"

# 2. Bootstrap the platform admin account (replace with a strong password):
ADMIN_PASSWORD="$(openssl rand -base64 18)" \
bash scripts/production/provision_tenants.sh \
  --host "$PROD_HOST" \
  --with-admin \
  --admin-email admin@7dsolutions.app

# 3. Confirm services + data visibility:
bash scripts/production/smoke.sh --host "$PROD_HOST"
PROD_HOST="$PROD_HOST" bash scripts/production/isolation_check.sh
```

### What provision_tenants.sh does

1. **Plan catalog check** — asserts `GET /api/ttp/plans?status=active` returns ≥ 1 plan.
   If empty, migrations have not run; run them on the VPS first (see below).
2. **Provision Tenant A** — `POST /api/control/tenants` with `product_code=starter`,
   idempotency key `prod-initial-tenant-a-starter-v1` (safe to replay).
3. **Provision Tenant B** — `POST /api/control/tenants` with `product_code=professional`,
   idempotency key `prod-initial-tenant-b-professional-v1` (safe to replay).
4. **Validate tenant list** — both tenants visible in `GET /api/tenants`.
5. **Validate summaries** — `GET /api/control/tenants/:id/summary` returns 200.
6. **Bootstrap admin** _(optional `--with-admin`)_ — runs `scripts/seed-platform-admin.sh`
   on the VPS for the given email and password; creates platform_admin RBAC binding.
7. **Final plan confirmation** — asserts plans still visible after provisioning.

### Ensure migrations have run

If `cp_plans` is empty on first access, run migrations inside the control-plane container
on the VPS:

```bash
ssh deploy@prod.7dsolutions.example.com \
  "docker compose -f /opt/7d-platform/docker-compose.platform.yml exec control-plane \
    sh -c 'sqlx migrate run --database-url \$DATABASE_URL \
           --source /opt/7d-platform/platform/tenant-registry/db/migrations'"
```

Migrations are idempotent — safe to run multiple times.

### Idempotency

`provision_tenants.sh` uses stable idempotency keys. Re-running after a partial failure
re-uses existing tenant records (status 200 replay) without creating duplicates.

### Rotating the admin password

The platform admin password is stored as a bcrypt hash in the auth database.
To rotate it, remove the credential row and re-run `--with-admin` with the new password:

```bash
# On the VPS (as deploy user):
docker exec -i 7d-auth-postgres psql -U auth_user -d auth_db \
  -c "DELETE FROM credentials WHERE email = 'admin@7dsolutions.app'"

# Then re-run provision_tenants.sh --with-admin with the new password.
```

Never store the plaintext admin password in the repository, scripts, or CI logs.

## Production Proof Gate

Every production deploy runs `scripts/production/proof_gate.sh` as the single
authoritative green-light for production readiness. The gate must pass before the
workflow marks the deploy complete.

```bash
# Run the full proof gate (CI sets these env vars from secrets):
export PROD_HOST=prod.7dsolutions.example.com
export PROD_USER=deploy
export TILLED_WEBHOOK_SECRET=<secret>
export SMOKE_STAFF_JWT=<jwt>          # optional — enables data assertions in smoke suite
bash scripts/production/proof_gate.sh
```

### Proof gate suites

| Suite | What it proves |
|-------|---------------|
| `smoke` | All `/healthz` + `/api/ready` endpoints pass; data endpoints return 200 with JWT |
| `isolation_check` | 12 cross-tenant API denial assertions — tenant A cannot read tenant B's data |
| `payment_verify` | Full money path: customer → invoice → Tilled webhook → paid status, idempotency (livemode=false) |
| `rollback_rehearsal` | SSH connectivity + deployment log readable — rollback infrastructure confirmed |

Per-suite logs are uploaded as a GitHub Actions artifact
(`production-proof-gate-logs-<run-id>`, retained 90 days).

### Blocking behaviour

`promote.yml` runs `proof_gate.sh` after deploy and before the Playwright checks.
A non-zero exit code from any suite fails the step and blocks the workflow.

## Rollback

Rollback = redeploy a prior immutable tag. Production-specific rollback script keeps the
deployment log on the production VPS (`.production-deployments`).

```bash
# Show deployment history on production VPS:
export PROD_HOST=prod.7dsolutions.example.com
export PROD_USER=deploy
bash scripts/production/rollback_stack.sh --history

# Roll back to a specific prior tag:
bash scripts/production/rollback_stack.sh --tag v1.0.0-abc1234

# Roll back to the tag before the current one (automatic):
bash scripts/production/rollback_stack.sh --previous
```

After rollback, update `deploy/production/MODULE-MANIFEST.md` to reflect the rolled-back
tag and commit the change so the manifest stays in sync with what is running.

Data volumes are never touched during rollback; only service containers are replaced.

### Rollback rehearsal

Every proof gate run executes a **rollback rehearsal** — a read-only SSH operation that:
1. Reads `.production-deployments` on the VPS to confirm the deployment log is accessible.
2. Validates that SSH connectivity required for rollback is working.
3. Prints the exact rollback command for operators.

This is not an actual rollback. It proves that when you *need* to roll back, the
infrastructure is reachable and the prior tag is recorded.

**Expected output from a successful rollback rehearsal:**
```
=== Deployment history (last 10 entries) ===
2026-02-21T12:00:00Z tag=v1.0.0-abc1234 registry=7dsolutions

Rollback preflight: PASSED

To roll back production, run one of:
  bash scripts/production/rollback_stack.sh --previous
  bash scripts/production/rollback_stack.sh --tag <prior-tag>

Rollback rehearsal PROVEN: SSH connectivity and deployment log confirmed.
```

If the rehearsal fails (SSH unreachable, log missing), fix connectivity before
deploying. A deploy without a working rollback path is not safe.

## SSH Access

After provisioning, only key-based SSH is accepted:

```bash
ssh -p 22 deploy@prod.7dsolutions.example.com
```

Root login is disabled. All admin commands use `sudo` as the `deploy` user.

To rotate the deploy SSH key:
1. Add the new public key to `~deploy/.ssh/authorized_keys` on the VPS.
2. Verify access with the new key.
3. Remove the old key from `authorized_keys`.
4. Update the CI GitHub Secret `PROD_SSH_PRIVATE_KEY`.

## GitHub Secrets (environment: `production`)

| Secret | Description |
|--------|-------------|
| `PROD_SSH_PRIVATE_KEY` | Private key matching deploy user's `authorized_keys` |
| `PROD_HOST` | VPS hostname or IP |
| `PROD_USER` | Deploy user (`deploy`) |
| `PROD_REPO_PATH` | Repo path on VPS (`/opt/7d-platform`) |
| `PROD_SSH_PORT` | SSH port (default `22`) |
| `DOCKER_USERNAME` | Registry username |
| `DOCKER_PASSWORD` | Registry password or token |

Application secrets (DB passwords, JWT keys) are separate production environment secrets.
See `bd-1itw` for the full production secrets contract.

## Scripts Reference

| Script | Purpose |
|--------|---------|
| `scripts/production/env.example` | Template for all production variables (provisioning + app secrets) |
| `scripts/production/export_env.sh` | Source this on the VPS to export `/etc/7d/production/secrets.env` |
| `scripts/production/secrets_check.sh` | Validate secrets file before deploying (run as root/sudo) |
| `scripts/production/provision_vps.sh` | Guided walkthrough for first-time provisioning |
| `scripts/production/ssh_bootstrap.sh` | Harden host and install Docker on a fresh VPS |
| `scripts/production/proof_gate.sh` | Single authoritative proof gate: smoke + isolation + payment verify + rollback rehearsal |
| `scripts/production/manifest_validate.sh` | Assert all manifest image tags exist in the registry |
| `scripts/production/manifest_diff.sh` | Compare manifest vs what is actually running on the production VPS |
| `scripts/production/deploy_stack.sh --manifest <file>` | Deploy from `deploy/production/MODULE-MANIFEST.md` |
| `scripts/production/deploy_stack.sh --tag <tag>` | Deploy a specific tag directly (emergency bypass — use manifest normally) |
| `scripts/production/rollback_stack.sh --tag <tag>` | Roll back to a specific prior tag |
| `scripts/production/rollback_stack.sh --previous` | Roll back to the tag before the current one |
| `scripts/production/rollback_stack.sh --history` | Show deployment log on the production VPS |
| `scripts/production/backup_all_dbs.sh` | Dump all 25 Postgres databases to local backup storage |
| `scripts/production/backup_all_dbs.sh --dry-run` | List all configured databases without running backups |
| `scripts/production/backup_ship.sh` | Ship latest backup to off-host storage (S3 or SCP) |

## Backups

### Local backups

`backup_all_dbs.sh` dumps all 25 Postgres databases to timestamped directories under
`/var/backups/7d-platform/`. Each run produces one `.sql.gz` per database, a globals
dump, and a SHA-256 manifest.

```bash
# On the VPS (as deploy user):
sudo -E bash /opt/7d-platform/scripts/production/backup_all_dbs.sh
```

Schedule via cron (e.g. daily at 02:00 UTC):

```bash
sudo crontab -e
# Add:
0 2 * * * bash /opt/7d-platform/scripts/production/backup_all_dbs.sh >> /var/log/7d-backup.log 2>&1
```

### Off-host backup shipping

After a local backup completes, `backup_ship.sh` copies the latest backup run to
off-host storage. Two methods are supported: S3-compatible object stores and SCP.

#### S3 shipping

Set these environment variables in `/etc/7d/production/secrets.env`:

```bash
BACKUP_SHIP_METHOD=s3
BACKUP_S3_BUCKET=your-backup-bucket
BACKUP_S3_PREFIX=backups/7d-platform          # optional, this is the default
AWS_ACCESS_KEY_ID=AKIA...
AWS_SECRET_ACCESS_KEY=...
AWS_DEFAULT_REGION=us-east-1                   # optional, this is the default
# For S3-compatible APIs (DigitalOcean Spaces, Backblaze B2, MinIO):
BACKUP_S3_ENDPOINT_URL=https://nyc3.digitaloceanspaces.com
```

#### SCP shipping

```bash
BACKUP_SHIP_METHOD=scp
BACKUP_SCP_HOST=backup.example.com
BACKUP_SCP_USER=backup                        # optional, this is the default
BACKUP_SCP_PORT=22                             # optional, this is the default
BACKUP_SCP_PATH=/var/backups/7d-platform       # optional, this is the default
BACKUP_SCP_KEY=~/.ssh/id_ed25519               # optional, this is the default
```

#### Running the ship

```bash
# Ship the latest local backup:
sudo -E bash /opt/7d-platform/scripts/production/backup_ship.sh

# Ship a specific backup run:
sudo -E bash /opt/7d-platform/scripts/production/backup_ship.sh \
  --backup-dir /var/backups/7d-platform/2026-02-21_02-00-01
```

#### Automated schedule

Chain backup and ship in cron so every dump is immediately shipped off-host:

```bash
sudo crontab -e
# Add:
0 2 * * * bash /opt/7d-platform/scripts/production/backup_all_dbs.sh >> /var/log/7d-backup.log 2>&1 && bash /opt/7d-platform/scripts/production/backup_ship.sh >> /var/log/7d-backup-ship.log 2>&1
```

### Local backup retention

Old backups are retained indefinitely by default. To prune, remove timestamped
directories under `/var/backups/7d-platform/` older than your retention window.
Off-host storage lifecycle (S3 lifecycle rules or remote cron) should be configured
separately at the storage provider.

## Staging Parity

Production intentionally mirrors staging:
- Same Docker Compose files, same service names, same internal hostnames
- Same `7d-platform` network name and port assignments
- Same named volumes

**Differences:**
- Host hardened (UFW, fail2ban, SSH drop-in, auditd, unattended-upgrades)
- External traffic through reverse proxy at 80/443 (service ports not publicly accessible)
- Real production secrets (never shared with staging)
- CI environment: `production` (separate from `staging`)

Do **not** create production-only Compose overrides unless absolutely necessary.
If a difference is needed, use a `docker-compose.production.yml` override and document it here.
