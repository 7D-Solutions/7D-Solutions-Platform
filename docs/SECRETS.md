# Secrets Management

Production secrets are stored as individual files under `/etc/7d/production/secrets/`
on the VPS. Each file holds a single secret value, is owned by root, and is mode 0600.
Docker Compose mounts these files into containers via the `secrets:` mechanism, so
sensitive values never appear in `docker inspect`, process listings, or CI logs.

## How It Works

1. **Secret files** live at `/etc/7d/production/secrets/<name>` — one value per file.
2. **`docker-compose.production.yml`** declares each secret and maps it into the
   correct container at `/run/secrets/<ENV_VAR_NAME>`.
3. **Postgres containers** use the native `POSTGRES_PASSWORD_FILE` mechanism to read
   their password from the mounted secret file.
4. **Application services** (Rust binaries) use an **entrypoint wrapper**
   (`scripts/docker-secrets-entrypoint.sh`) that reads every file in `/run/secrets/`
   and exports it as an environment variable before exec-ing the binary. No Rust
   code changes are needed.
5. **Local dev is unaffected** — dev still uses `.env` with safe defaults. The
   production overlay is only included in production deploys.

## Quick Start (First Time)

On the production VPS, as root:

```bash
# 1. Generate all secret files with random values
sudo bash /opt/7d-platform/scripts/production/secrets_init.sh

# 2. Validate
sudo bash /opt/7d-platform/scripts/production/secrets_check.sh --format=dir

# 3. Deploy (the deploy script auto-detects the secrets directory)
bash scripts/production/deploy_stack.sh --manifest deploy/production/MODULE-MANIFEST.md
```

## Secret Files Reference

### Shared Secrets

| File | Description | Used by |
|------|-------------|---------|
| `jwt_private_key_pem` | Ed25519 private key (PEM) for JWT signing | auth |
| `jwt_public_key_pem` | Ed25519 public key (PEM) for JWT verification | auth, gl, party, maintenance |
| `jwt_secret` | BFF session encryption key (hex) | frontend BFF |
| `nats_auth_token` | NATS client password | NATS server |
| `nats_url` | Full `nats://platform:<token>@7d-nats:4222` URL | all backend services |
| `tilled_webhook_secret` | Tilled payment webhook signing secret | AR |
| `seed_admin_password` | Initial admin password for tenant bootstrap | provisioning scripts |

### Per-Service Database Secrets

Each database service has two secret files:

| File pattern | Contents |
|-------------|----------|
| `<prefix>_postgres_password` | Raw password (used by Postgres container via `POSTGRES_PASSWORD_FILE`) |
| `<prefix>_database_url` | Full `postgres://user:password@host:5432/dbname` URL (used by application service) |

Prefixes: `auth`, `ar`, `subscriptions`, `payments`, `notifications`, `gl`,
`projections`, `audit`, `tenant_registry`, `inventory`, `ap`, `treasury`,
`fixed_assets`, `consolidation`, `timekeeping`, `party`, `integrations`, `ttp`,
`pdf_editor`, `maintenance`, `shipping_receiving`, `numbering`, `doc_mgmt`,
`workflow`, `wc`.

Special: `control_plane_ar_database_url` — the control-plane service reads both
the tenant registry DB and the AR DB.

## Rotating Secrets

### Rotate a single database password

```bash
# 1. Generate new password and update secret files
sudo bash scripts/production/secrets_rotate.sh db auth

# 2. Update the Postgres role (the script prints this command)
docker exec 7d-auth-postgres psql -U auth_user -d auth_db \
  -c "ALTER ROLE auth_user WITH PASSWORD '<new-password>'"

# 3. Redeploy the affected service
docker compose -f docker-compose.services.yml -f docker-compose.production.yml \
  up -d auth-1 auth-2
```

### Rotate NATS token

```bash
# 1. Generate new token (updates nats_auth_token + nats_url)
sudo bash scripts/production/secrets_rotate.sh nats

# 2. Redeploy NATS and all backend services
docker compose -f docker-compose.data.yml -f docker-compose.production.yml up -d nats
docker compose -f docker-compose.services.yml -f docker-compose.production.yml up -d
```

### Rotate JWT keys

```bash
# 1. Generate new Ed25519 key pair
sudo bash scripts/production/secrets_rotate.sh jwt

# 2. Redeploy auth + JWT-verifying services
docker compose -f docker-compose.services.yml -f docker-compose.production.yml \
  up -d auth-1 auth-2 gl party maintenance
```

**Warning:** Rotating JWT keys invalidates all existing tokens. Users will need to
re-authenticate.

### Rotate all database passwords

```bash
sudo bash scripts/production/secrets_rotate.sh db all
# Then ALTER ROLE for each database and redeploy all services.
```

## Validating Secrets

```bash
# Validate directory format (auto-detected)
sudo bash scripts/production/secrets_check.sh

# Explicitly validate directory format
sudo bash scripts/production/secrets_check.sh --format=dir

# Validate old single-file format (backward compatible)
sudo bash scripts/production/secrets_check.sh --format=file /etc/7d/production/secrets.env
```

The check verifies:
- Directory exists, is root-owned, mode 0700
- All files are root-owned, mode 0600
- No `CHANGE_ME` placeholders or test/dev patterns
- All required secret files exist and are non-empty
- All `DATABASE_URL` files start with `postgres://`

## Directory Permissions

| Path | Owner | Mode |
|------|-------|------|
| `/etc/7d/production/secrets/` | `root:root` | `0700` |
| `/etc/7d/production/secrets/*` | `root:root` | `0600` |

## Migration from secrets.env

If you're migrating from the old single-file format (`/etc/7d/production/secrets.env`):

```bash
# 1. Generate the new directory structure
sudo bash scripts/production/secrets_init.sh

# 2. Verify
sudo bash scripts/production/secrets_check.sh --format=dir

# 3. Deploy — deploy_stack.sh auto-detects the directory and includes the overlay
bash scripts/production/deploy_stack.sh --manifest deploy/production/MODULE-MANIFEST.md

# 4. Once confirmed working, archive the old file
sudo mv /etc/7d/production/secrets.env /etc/7d/production/secrets.env.archived
```

The old `secrets.env` file can coexist with the new directory. `deploy_stack.sh`
prefers the directory format when it detects `/etc/7d/production/secrets/`.

## Architecture

```
Production VPS
├── /etc/7d/production/secrets/          (root:root 0700)
│   ├── jwt_private_key_pem              (root:root 0600)
│   ├── jwt_public_key_pem
│   ├── nats_auth_token
│   ├── nats_url
│   ├── auth_postgres_password
│   ├── auth_database_url
│   ├── ...                              (one file per secret)
│   └── seed_admin_password
│
├── docker-compose.production.yml        (declares secrets + entrypoint overrides)
└── scripts/
    ├── docker-secrets-entrypoint.sh     (reads /run/secrets/* → env vars → exec)
    └── production/
        ├── secrets_init.sh              (generate all secrets)
        ├── secrets_rotate.sh            (rotate individual secrets)
        └── secrets_check.sh             (validate before deploy)
```

Inside a container:
```
/run/secrets/
├── DATABASE_URL       ← mapped from <prefix>_database_url secret
├── NATS_URL           ← mapped from nats_url secret
├── JWT_PUBLIC_KEY     ← mapped from jwt_public_key_pem (if needed)
└── ...
```

The entrypoint reads each file and exports `DATABASE_URL=<contents>`, then exec's
the binary. The binary sees standard environment variables — zero code changes.
