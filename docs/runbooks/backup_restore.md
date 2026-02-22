# Backup & Restore Runbook

**Phase 48 — Production Hardening (last updated: bd-3len)**

## Purpose

Deterministic, idempotent backup and restore of all 7D Solutions Platform module databases.
Covers full-platform backup, full-platform restore, and smoke-restore verification.

## Databases Covered

| Module | Container | Port | DB |
|---|---|---|---|
| auth | 7d-auth-postgres | 5433 | auth_db |
| ar | 7d-ar-postgres | 5434 | ar_db |
| subscriptions | 7d-subscriptions-postgres | 5435 | subscriptions_db |
| payments | 7d-payments-postgres | 5436 | payments_db |
| notifications | 7d-notifications-postgres | 5437 | notifications_db |
| gl | 7d-gl-postgres | 5438 | gl_db |
| projections | 7d-projections-postgres | 5439 | projections_db |
| audit | 7d-audit-postgres | 5440 | audit_db |
| tenant_registry | 7d-tenant-registry-postgres | 5441 | tenant_registry_db |
| inventory | 7d-inventory-postgres | 5442 | inventory_db |
| ap | 7d-ap-postgres | 5443 | ap_db |
| treasury | 7d-treasury-postgres | 5444 | treasury_db |
| fixed_assets | 7d-fixed-assets-postgres | 5445 | fixed_assets_db |
| consolidation | 7d-consolidation-postgres | 5446 | consolidation_db |
| timekeeping | 7d-timekeeping-postgres | 5447 | timekeeping_db |
| party | 7d-party-postgres | 5448 | party_db |
| integrations | 7d-integrations-postgres | 5449 | integrations_db |
| ttp | 7d-ttp-postgres | 5451 | ttp_db |

## Prerequisites

- `pg_dump` and `psql` installed and on `PATH` (`brew install libpq` on macOS)
- Docker Compose stack running (or target databases reachable)
- Sufficient disk space in backup destination
- Python 3 (for manifest JSON parsing in restore script)

## 1. Backup Procedure

### Dry run (verify plan without writing anything)

```bash
bash scripts/backup_all.sh --dry-run
```

Output lists every database and the files that *would* be created.

### Full backup

```bash
# Default: creates ./backups/YYYY-MM-DD_HH-MM-SS/
bash scripts/backup_all.sh

# Custom target directory
bash scripts/backup_all.sh /mnt/nas/7d-backups/$(date +%Y-%m-%d)
```

### What gets created

```
backups/2026-02-19_02-00-00/
  auth.sql.gz
  ar.sql.gz
  subscriptions.sql.gz
  payments.sql.gz
  notifications.sql.gz
  gl.sql.gz
  projections.sql.gz
  audit.sql.gz
  tenant_registry.sql.gz
  inventory.sql.gz
  ap.sql.gz
  treasury.sql.gz
  fixed_assets.sql.gz
  consolidation.sql.gz
  timekeeping.sql.gz
  party.sql.gz
  integrations.sql.gz
  manifest.json          ← metadata + row counts per table
  backup.log             ← timestamped execution log
```

### Verify backup integrity

```bash
BACKUP_DIR="backups/2026-02-19_02-00-00"

# Check manifest
cat "${BACKUP_DIR}/manifest.json" | python3 -m json.tool | head -40

# Check for errors in log
grep -E "ERROR|WARN" "${BACKUP_DIR}/backup.log"

# Verify gzip integrity of all dumps
for f in "${BACKUP_DIR}"/*.sql.gz; do
  gunzip -t "$f" && echo "OK: $f" || echo "CORRUPT: $f"
done
```

### Automated daily backup (cron)

```cron
# Daily at 02:00 — runs from project root
0 2 * * * cd /app && bash scripts/backup_all.sh /backups/$(date +\%Y-\%m-\%d) >> /var/log/7d-backup.log 2>&1
```

## 2. Restore Procedure

> **WARNING**: Full restore drops and recreates each database. Stop application services first.

### Stop services

```bash
docker compose -f docker-compose.modules.yml down
```

### Restore all databases from a backup

```bash
BACKUP_DIR="backups/2026-02-19_02-00-00"
bash scripts/restore_all.sh "${BACKUP_DIR}"
```

The script will, for each database in the manifest:
1. Verify gzip integrity of the dump file
2. Drop the existing database (via superuser on port 5432)
3. Create a fresh database
4. Pipe the dump through `psql`
5. Run a smoke test comparing row counts against the manifest

### Environment variables for restore

By default, the restore script connects to each module's Postgres using the same defaults as docker-compose. Override with:

```bash
# Superuser connection (for DROP/CREATE DATABASE)
export SUPERUSER_POSTGRES_HOST=localhost
export SUPERUSER_POSTGRES_PORT=5432
export SUPERUSER_POSTGRES_USER=postgres
export SUPERUSER_POSTGRES_PASSWORD=postgres

# Per-module overrides (e.g. if AR moved to a different host)
export AR_POSTGRES_HOST=db-ar.prod.internal
export AR_POSTGRES_PORT=5432
export AR_POSTGRES_USER=ar_user
export AR_POSTGRES_PASSWORD=secret
```

### Restart services

```bash
docker compose -f docker-compose.modules.yml up -d
```

## 3. Smoke-Restore Verification

Run verification against live databases without restoring (confirms existing data matches a known good backup):

```bash
BACKUP_DIR="backups/2026-02-19_02-00-00"
bash scripts/restore_all.sh "${BACKUP_DIR}" --smoke-test
```

This compares row counts for every table in every database against the manifest values. Use it to:
- Confirm a restore succeeded
- Detect unexpected data loss after an incident
- Validate nightly backup row counts match expectations

### Expected output

```
[02:15:33] INFO  manifest: backups/2026-02-19_02-00-00/manifest.json
[02:15:33] INFO  ar: running smoke test
[02:15:33] PASS  ar.invoices: 1042 rows OK
[02:15:33] PASS  ar.ar_customers: 87 rows OK
...
[02:15:45] PASS  gl: all table counts match manifest

=== RESTORE OK: all databases verified against manifest ===
```

Exit code 0 = all checks passed. Non-zero = failures (see output).

## 4. Single-Database Restore

To restore one module without affecting others:

```bash
BACKUP_DIR="backups/2026-02-19_02-00-00"
MODULE="ar"
DB="ar_db"
PGPASSWORD=postgres psql -h localhost -p 5432 -U postgres \
  -c "DROP DATABASE IF EXISTS \"${DB}\";" \
  -c "CREATE DATABASE \"${DB}\" OWNER ar_user;"

gunzip -c "${BACKUP_DIR}/${MODULE}.sql.gz" | \
  PGPASSWORD=ar_pass psql -h localhost -p 5434 -U ar_user -d "${DB}"
```

Then run smoke test for that module:

```bash
bash scripts/restore_all.sh "${BACKUP_DIR}" --smoke-test
```

## 5. Retention Policy

| Destination | Retention | Notes |
|---|---|---|
| Local `./backups/` | 30 days | Dev / staging |
| Remote object store (S3/GCS) | 90 days | Production |
| Financial data (GL, AR, AP) | 7 years | SOX compliance |

### Prune old local backups

```bash
# Delete backups older than 30 days
find ./backups -maxdepth 1 -type d -mtime +30 -exec rm -rf {} +
```

## 6. Troubleshooting

### Backup script reports "database not reachable"

```bash
# Check Docker is running
docker ps | grep postgres

# Check connectivity manually
pg_isready -h localhost -p 5434 -U ar_user -d ar_db
```

### Restore fails with "role does not exist"

The dump includes `--no-owner` so role errors should not occur. If they do:

```bash
gunzip -c backup.sql.gz | grep "^CREATE ROLE"
# Create the missing role manually, then retry restore
```

### Duplicate key error during restore

The database was not empty. The script drops and recreates — ensure `DROP DATABASE` succeeded. Check superuser credentials:

```bash
PGPASSWORD=postgres psql -h localhost -p 5432 -U postgres -c "\l"
```

### Smoke test row count mismatch

Row counts after restore do not match manifest. Possible causes:
1. Restore was incomplete (check restore log for errors)
2. Manifest is stale (re-run backup to generate fresh baseline)
3. Data was modified between backup and restore verification

Investigate with:

```bash
# Check actual count vs manifest
python3 -c "
import json
data = json.load(open('backups/YYYY-MM-DD/manifest.json'))
for db in data['databases']:
    if db['name'] == 'ar':
        for t in db['tables']:
            print(t['table'], t['count'])
"
```

### Corrupt gzip

```bash
gunzip -t backups/YYYY-MM-DD/ar.sql.gz
# If corrupt, restore from previous day's backup
```

## References

- `scripts/backup_all.sh` — backup script
- `scripts/restore_all.sh` — restore + smoke-verify script
- `docker-compose.infrastructure.yml` — database port/credential defaults
- `docs/ops/BACKUP-RESTORE-RUNBOOK.md` — legacy runbook (Phase 16)
- PostgreSQL pg_dump docs: https://www.postgresql.org/docs/current/app-pgdump.html

## Changelog

- **2026-02-22**: Phase 48 — add TTP database to coverage table (bd-3len)
- **2026-02-19**: Phase 34 — scripted backup/restore with automated smoke test (bd-tet1)
