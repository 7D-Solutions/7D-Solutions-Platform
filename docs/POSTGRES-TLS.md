# Postgres TLS Setup

All Postgres database connections use TLS encryption. This applies to both
local development and production deployments.

## How It Works

**Server side (Postgres containers):**
- Every Postgres container starts with `ssl=on` via command-line args
- A custom entrypoint (`infra/postgres/docker-entrypoint-tls.sh`) copies certs
  to the correct path with Postgres-required permissions (key file mode 600)
- `pg_hba.conf` requires TLS for all TCP connections (`hostssl` only, no `host`)
- Authentication method is `scram-sha-256` (PG16 default). Do NOT use `md5` — sqlx
  with `runtime-tokio-rustls` cannot negotiate md5 auth over TLS
- Unix socket connections (used by health checks) are allowed without TLS

**Client side (Rust services):**
- All `DATABASE_URL` values include `?sslmode=require`
- `sqlx` with `runtime-tokio-rustls` handles TLS negotiation automatically

## Local Development

### First-time setup

Generate dev certificates (self-signed CA + server cert covering all DB hostnames):

```bash
./infra/postgres/tls/generate-dev-certs.sh
```

This creates three files in `infra/postgres/tls/`:
- `ca.crt` — CA certificate
- `server.crt` — server certificate (valid for all `7d-*-postgres` hostnames + localhost)
- `server.key` — server private key

These files are gitignored. The script is idempotent — it skips generation if valid
certs already exist.

### After generating certs

Restart the data stack to pick up TLS:

```bash
docker compose -f docker-compose.data.yml down
docker compose -f docker-compose.data.yml up -d
```

Existing pgdata volumes do not need to be recreated. TLS is configured via
command-line args, not `postgresql.conf` inside the data directory.

### Verifying TLS is active

Connect to any database and check the SSL status:

```bash
docker exec -it 7d-auth-postgres psql -U auth_user -d auth_db -c "SELECT ssl, version FROM pg_stat_ssl WHERE pid = pg_backend_pid();"
```

Expected output: `ssl = t` (true).

## Production Deployment

### Certificate setup

Deploy CA-signed certificates to `infra/postgres/tls/` on the production host,
replacing the dev self-signed certs:

```
infra/postgres/tls/
  ca.crt       — CA certificate (your organization's CA or a trusted CA)
  server.crt   — Server certificate signed by the CA
  server.key   — Server private key (will be copied with mode 600 by entrypoint)
```

The server certificate must include SANs for all Postgres container hostnames
(e.g., `7d-auth-postgres`, `7d-ar-postgres`, etc.). A single wildcard or
multi-SAN cert works for all databases.

### sslmode for production clients

Production `DATABASE_URL` secrets (in `/etc/7d/production/secrets/`) should use
`sslmode=verify-full` for maximum security:

```
postgres://user:pass@7d-auth-postgres:5432/auth_db?sslmode=verify-full&sslrootcert=/etc/ssl/certs/7d-ca.crt
```

This requires the CA certificate to be available inside the service container.
Mount it via the production overlay or bake it into the service image.

### sslmode levels

| Mode | Encryption | Server cert validated | Hostname checked |
|------|-----------|----------------------|-----------------|
| `disable` | No | No | No |
| `require` | Yes | No | No |
| `verify-ca` | Yes | Yes (against CA) | No |
| `verify-full` | Yes | Yes (against CA) | Yes |

Dev uses `require` (encrypted but no CA validation — acceptable for self-signed).
Production should use `verify-full`.

## Certificate Rotation

1. Generate new cert/key pair signed by the same CA (or a new CA if rotating CAs)
2. Deploy new files to `infra/postgres/tls/` on the host
3. Restart Postgres containers — they re-read certs on startup
4. No service restart needed — `sqlx` reconnects automatically on next pool refresh

If rotating the CA:
1. Deploy new CA cert alongside the old one (both must be trusted during transition)
2. Update `sslrootcert` in service `DATABASE_URL` secrets to point to the new CA
3. Restart services to pick up the new CA trust
4. Remove the old CA cert

## Troubleshooting

**"SSL connection is required" error:**
The database is correctly enforcing TLS but the client isn't using it. Check that
the `DATABASE_URL` includes `?sslmode=require` (or stronger).

**"certificate verify failed" error:**
The client is using `verify-ca` or `verify-full` but can't validate the server cert.
Check that `sslrootcert` points to the correct CA certificate.

**Postgres won't start — "could not load server certificate":**
The cert files may be missing or have wrong permissions. Run
`./infra/postgres/tls/generate-dev-certs.sh` to regenerate dev certs.

**"password authentication failed" with TLS enabled:**
Check `infra/postgres/pg_hba.conf` — the auth method must be `scram-sha-256`, not
`md5`. sqlx 0.8 with `runtime-tokio-rustls` cannot negotiate md5 authentication
over TLS connections. After changing, recreate all Postgres containers.

**Tests fail with connection refused:**
Ensure the data stack is running with TLS-enabled containers:
`docker compose -f docker-compose.data.yml up -d`
