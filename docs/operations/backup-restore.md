# Backup & Restore Operations

**Bead:** bd-3ybu7 (GAP-26)  
**RPO target:** 24 hours  
**RTO target:** 4 hours

---

## Overview

Nightly automated backups cover all 17 PostgreSQL databases across the platform.
Each database is dumped independently to a timestamped, gzip-compressed SQL file.

| Item | Value |
|------|-------|
| Schedule | 01:00 UTC daily (cron) |
| Retention | 14 days of rolling backups |
| Format | `pg_dump --format=plain` piped through `gzip -9` |
| Cron file | `infra/crontab.d/backup` |
| Backup script | `scripts/backup_all.sh` |
| Drill script | `scripts/dr_drill.sh` |

---

## Backup Layout

```
backups/
  YYYY-MM-DD_HH-MM-SS/
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
    manifest.json     ← row counts + metadata
    backup.log        ← execution log
```

---

## Running a Manual Backup

```bash
# Standard run (uses docker-compose defaults)
./scripts/backup_all.sh

# Specify output directory
./scripts/backup_all.sh /mnt/backups/2026-04-13

# Dry-run (print plan, no changes)
./scripts/backup_all.sh --dry-run
```

Exit 0 = all databases backed up successfully.  
Exit 1 = one or more databases were unreachable or dump failed (check backup.log).

---

## Running the DR Drill

The drill creates a fresh backup, verifies gzip integrity, validates the manifest,
runs a smoke test (row count comparison), and checks NATS + service health.

```bash
# Full drill (runs backup, integrity, smoke test)
./scripts/dr_drill.sh

# Dry-run
./scripts/dr_drill.sh --dry-run
```

Report written to `dr-reports/dr-drill-YYYY-MM-DD_HH-MM-SS.txt`.

---

## Restoring a Tenant-Module to a Specific Point in Time

### Step 1: Identify the backup

```bash
ls -lt backups/ | head -10    # most recent first
```

Choose the backup directory closest to the recovery point (e.g. `backups/2026-04-12_01-00-23`).

### Step 2: Restore a single module

```bash
# Restore AR database from a specific backup
BACKUP_DIR=backups/2026-04-12_01-00-23
MODULE=ar

# Decompress and restore (will DROP + recreate the database)
PGPASSWORD=<superuser_password> psql -h <host> -U postgres -c "DROP DATABASE IF EXISTS ar_db;"
PGPASSWORD=<superuser_password> psql -h <host> -U postgres -c "CREATE DATABASE ar_db OWNER ar_user;"
gunzip -c "${BACKUP_DIR}/${MODULE}.sql.gz" | \
  PGPASSWORD=<module_password> psql -h <host> -U ar_user -d ar_db
```

### Step 3: Restore all modules

```bash
# Restore all databases from a backup directory
./scripts/restore_all.sh backups/2026-04-12_01-00-23
```

### Step 4: Verify row counts

```bash
# Smoke test: compare row counts against manifest
./scripts/restore_all.sh backups/2026-04-12_01-00-23 --smoke-test
```

---

## Prometheus Monitoring

The backup sidecar publishes `platform_backup_last_success_seconds{module}` via
node_exporter's textfile collector (`/var/lib/prometheus/textfiles/backup.prom`).

**Alert rules:** `infra/monitoring/alerts/backup-age.yml`

| Alert | Threshold | Severity |
|-------|-----------|----------|
| `BackupStale` | age > 26h | critical |
| `BackupMetricMissing` | metric absent > 30m | warning |

Prometheus computes age as: `time() - platform_backup_last_success_seconds`

---

## Incident Response

### BackupStale fires

1. `ssh backup-host`
2. `tail -100 /var/log/7d-backup/nightly.log` — check last run output
3. `./scripts/backup_all.sh` — run manually and observe output
4. If databases unreachable: check Docker containers with `docker ps`
5. After successful backup: alert auto-resolves within 5 minutes

### Weekly CI drill fails

1. Open the `dr-drill-report` artifact in the failed GitHub Actions run
2. Look for `FAIL` lines in the drill output
3. Common causes:
   - Database restore failed: check `restore_all.sh` error output
   - gzip corrupt: re-run backup immediately; investigate disk issues
   - Manifest errors: check `backup_all.sh` log for the failed run

### Full platform data restore

1. Halt all services: stop application containers
2. Run `./scripts/restore_all.sh <BACKUP_DIR>` for the chosen backup
3. Verify with `./scripts/restore_all.sh <BACKUP_DIR> --smoke-test`
4. Restart services
5. RTO target: 4 hours from decision to live

---

## Retention Policy

Backups older than 14 days are pruned by the cron entry in `infra/crontab.d/backup`
at 01:05 UTC daily (5 minutes after backup starts).

To retain a backup indefinitely: move it out of `backups/` to a separate archive path
before the prune window runs.
