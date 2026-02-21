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
| Fixed Assets | 8095 | internal only |
| Consolidation | 8096 | internal only |
| Timekeeping | 8097 | internal only |
| Party | 8098 | internal only |
| Integrations | 8099 | internal only |
| TTP | 8100 | internal only |

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

bash scripts/staging/deploy_stack.sh --tag v1.0.0-abc1234
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
  bash /opt/7d-platform/scripts/staging/deploy_stack.sh --tag <tag>'
```

Or in an explicit two-step:

```bash
# As root (or via sudo), export into a sub-shell:
sudo bash -c 'source /opt/7d-platform/scripts/production/export_env.sh && \
  env | grep -E "POSTGRES|JWT" > /run/7d-prod-env && chmod 0600 /run/7d-prod-env'

# Then deploy reads /run/7d-prod-env (cleaned up by deploy_stack.sh)
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
- All 21 required variables are present and non-empty

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

Variable names and DB usernames are documented in full in `scripts/production/env.example`.

**Never commit `.env.production` or `secrets.env` to git.** Both are listed in `.gitignore`.

## Ongoing Deploys

Deployment process is identical to staging: promote an immutable image tag.

```bash
# Via GitHub Actions (recommended):
# Actions → Promote Artifacts → Run workflow → environment: production → tag

# CLI (emergency):
bash scripts/staging/deploy_stack.sh --tag v1.0.1-def5678
```

Docker Compose replaces only changed containers. Postgres volumes are preserved.

## Rollback

Rollback = promote a prior tag.

```bash
# Via GitHub Actions (recommended):
# Actions → Promote Artifacts → Run workflow → environment: production → prior tag

# CLI:
bash scripts/staging/rollback_stack.sh --tag v1.0.0-abc1234
```

View deployment history on the VPS:

```bash
bash scripts/staging/rollback_stack.sh --history
```

Data volumes are never touched during rollback; only service containers are replaced.

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
| `scripts/staging/deploy_stack.sh --tag <tag>` | Deploy a pinned image tag (works for both staging and prod) |
| `scripts/staging/rollback_stack.sh --tag <tag>` | Roll back to a specific prior tag |
| `scripts/staging/rollback_stack.sh --history` | Show deployment log on VPS |

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
