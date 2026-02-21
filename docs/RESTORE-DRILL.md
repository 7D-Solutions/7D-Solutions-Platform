# Restore Drill

Single source of truth for running the 7D Platform backup restore drill.

A restore drill proves that the backup system is actually recoverable.
**Backups are only real if restore is proven.** Run this drill after any
infrastructure change and at minimum quarterly (SOC 2 requirement).

## RPO / RTO Targets

| Tier | Databases | RPO | RTO |
|------|-----------|-----|-----|
| **Platform** | Auth, Tenant Registry, Audit | 1 hour | 2 hours |
| **Critical** | GL, AR, AP, Payments, Treasury | 1 hour | 4 hours |
| **High** | Subscriptions, Inventory, Fixed Assets, Consolidation | 1 hour | 4 hours |
| **Standard** | Notifications, Projections, Timekeeping, Party, Integrations, TTP | 4 hours | 8 hours |

**RPO** = Recovery Point Objective (maximum acceptable data loss interval).
**RTO** = Recovery Time Objective (maximum acceptable restore duration).

These targets are achieved by:
- Hourly automated backups on production (bd-34t2 / `install_backup_timer.sh`)
- Off-host shipping to S3 or SCP remote immediately after each backup
- Tested restore procedure (this drill) with known timing benchmarks

## Restore Order

Always restore in this sequence to satisfy cross-service dependencies:

1. **Platform tier first** — Auth, Tenant Registry, Audit
   (all application services require Auth; Tenant Registry is queried at startup)
2. **Financial tier** — GL, AR, AP, Payments, Treasury
   (GL double-entry must be restored before any module that posts journal entries)
3. **Remaining modules** — Subscriptions, Inventory, Fixed Assets, Consolidation,
   Notifications, Projections, Timekeeping, Party, Integrations, TTP

`restore_drill.sh` follows this order automatically.

## Running the Drill

### Prerequisites

- Docker running locally (or on the target host)
- A backup directory produced by `backup_all_dbs.sh`
- Credentials available (from `/etc/7d/production/secrets.env` or environment)

### Quick run (latest backup)

```bash
bash scripts/production/restore_drill.sh
```

### Specify a backup directory

```bash
bash scripts/production/restore_drill.sh \
  --backup-dir /var/backups/7d-platform/2026-02-21_02-00-00
```

### Dry run (print plan without executing)

```bash
bash scripts/production/restore_drill.sh --dry-run
```

### Keep containers for inspection

```bash
bash scripts/production/restore_drill.sh --no-cleanup
# Inspect:
docker exec 7d-drill-postgres psql -U drill_su -d auth_db -c "\dt"
# Teardown manually:
docker rm -f 7d-drill-postgres && docker network rm 7d-drill-net
```

## What the Drill Does

1. **Verifies backup integrity** — SHA-256 checksums from MANIFEST.txt are checked
   before any restore begins. A corrupted dump fails fast.

2. **Provisions a clean restore target** — Spins up an isolated Docker container
   (`7d-drill-postgres`) on a private network (`7d-drill-net`). This is the
   "clean slate" equivalent of provisioning a fresh VPS Postgres instance.

3. **Restores in dependency order** — Each database dump (`.sql.gz`) is decompressed
   and piped into `psql` running inside the container. The module's DB user and
   database are created first (DROP/CREATE) so there is no residual state.

4. **Runs health_audit.sh** — Connects to every restored database via Docker exec,
   runs `SELECT 1` to confirm connectivity, and counts tables to confirm the schema
   was loaded. A database with 0 tables is flagged as a warning.

5. **Reports timing** — Each database restore time is logged. Cumulative drill
   time is printed at the end. Compare against your RTO targets.

6. **Cleans up** — The drill container and network are removed on exit unless
   `--no-cleanup` is specified.

## Expected Timings

Benchmarks vary by backup size and hardware. Rough estimates for a complete
18-database restore on a Hetzner CX41 (4 vCPU / 8 GB RAM):

| Phase | Expected duration |
|-------|-----------------|
| Manifest verification | < 5 seconds |
| Container start (Postgres 16) | 5–15 seconds |
| Per-database restore (small DB) | 2–15 seconds |
| Per-database restore (large DB with years of data) | 1–10 minutes |
| health_audit.sh | < 10 seconds |
| **Total (fresh install, small data)** | **2–5 minutes** |
| **Total (production data volume)** | **15–60 minutes** |

If total restore time exceeds the 4-hour RTO for critical databases, investigate:
- Whether backup dumps can be parallelized
- Whether Postgres instance sizing needs improvement
- Whether WAL streaming replication (lower RTO) is warranted

## Running health_audit.sh Standalone

After a restore drill or production deploy, audit database health directly:

```bash
# Check against drill container (left running with --no-cleanup):
./scripts/production/health_audit.sh --drill

# Check against live production containers (auto-detected):
./scripts/production/health_audit.sh
```

If no containers are running, the script exits 0 with a notice — it does not
fail on empty environments.

## Restore Sequence — Step by Step (Manual Procedure)

Use this when `restore_drill.sh` cannot be run (e.g., target VPS with no Docker CLI access to run the script itself).

### Step 1 — Stop application services

```bash
docker compose -f docker-compose.yml down
docker compose -f docker-compose.platform.yml down
```

### Step 2 — Identify the backup

```bash
ls -1td /var/backups/7d-platform/????-??-??_??-??-?? | head -5
# Pick the most recent clean backup (check MANIFEST.txt for checksum pass)
BACKUP_DIR=/var/backups/7d-platform/2026-02-21_02-00-00
```

### Step 3 — Restore globals (roles)

```bash
gunzip -c "${BACKUP_DIR}/globals.sql.gz" | \
  docker exec -i -e PGPASSWORD="${AUTH_POSTGRES_PASSWORD}" 7d-auth-postgres \
  psql -U "${AUTH_POSTGRES_USER}" -d postgres
```

### Step 4 — Restore each database (platform tier first)

For each module, in order: auth → tenant_registry → audit → gl → ar → ap →
payments → treasury → subscriptions → inventory → fixed_assets → consolidation
→ notifications → projections → timekeeping → party → integrations → ttp

```bash
# Pattern (repeat for each module):
DB=auth_db
CONTAINER=7d-auth-postgres
USER="${AUTH_POSTGRES_USER}"
PASS="${AUTH_POSTGRES_PASSWORD}"

# Drop and recreate (superuser access via auth container)
docker exec -i -e PGPASSWORD="${AUTH_POSTGRES_PASSWORD}" 7d-auth-postgres \
  psql -U "${AUTH_POSTGRES_USER}" -d postgres \
  -c "DROP DATABASE IF EXISTS \"${DB}\";" \
  -c "CREATE DATABASE \"${DB}\" OWNER \"${USER}\";"

# Restore
gunzip -c "${BACKUP_DIR}/${DB}.sql.gz" | \
  docker exec -i -e PGPASSWORD="${PASS}" "${CONTAINER}" \
  psql -U "${USER}" -d "${DB}"
```

### Step 5 — Verify restore

```bash
./scripts/production/health_audit.sh
```

### Step 6 — Restart services

```bash
docker compose -f docker-compose.data.yml up -d
docker compose -f docker-compose.platform.yml up -d
docker compose -f docker-compose.yml up -d
docker compose -f docker-compose.frontend.yml up -d
```

### Step 7 — Run smoke suite

```bash
bash scripts/production/smoke.sh
```

## Drill Cadence

| Trigger | Frequency |
|---------|-----------|
| Scheduled drill | Quarterly (minimum) |
| After infrastructure changes | Within 1 week |
| After backup system changes | Immediately |
| After major schema migrations | Within 1 week |

Document each drill result in `docs/ops/DRILL-LOG.md` (create if it does not exist):

```
## 2026-02-21 Restore Drill
- Operator: MaroonHarbor
- Backup source: 2026-02-21_02-00-00
- Total elapsed: 4m 32s
- Result: PASS (18/18 databases)
- Notes: —
```

## Troubleshooting

### `Dump file not found: /path/to/auth_db.sql.gz`

The backup run named files by the `*_POSTGRES_DB` variable value. Confirm the
variable is exported: `echo $AUTH_POSTGRES_DB`. If the backup was created with
different credentials, update the env vars to match.

### `connection failed` in health_audit.sh

The database user may not have been created before the restore, or the password
differs. In drill mode, the restore creates the user from credentials in env.
In production mode, confirm the module's Postgres container is running:
```bash
docker ps --filter name=7d-auth-postgres
```

### Checksum mismatch in MANIFEST.txt

The dump file was corrupted in transit or during storage. Fall back to the
previous day's backup. Investigate storage integrity on the backup host.

### Restore succeeds but 0 tables in health_audit.sh

The dump was created from an empty database (service not yet seeded). This is
normal for fresh environments. Verify the production dump was taken from a live
stack, not an empty initialization.

## Scripts Reference

| Script | Purpose |
|--------|---------|
| `scripts/production/restore_drill.sh` | Run end-to-end restore drill |
| `scripts/production/health_audit.sh` | Audit database accessibility post-restore |
| `scripts/production/backup_all_dbs.sh` | Create the backup that this drill restores |
| `scripts/production/backup_ship.sh` | Ship backup off-host (S3 or SCP) |
| `scripts/production/smoke.sh` | HTTP-level smoke suite (requires live stack) |
| `docs/runbooks/disaster_recovery.md` | Full DR runbook with triage procedures |

## Changelog

- **2026-02-21**: Initial restore drill (bd-1xdz, P45-130)
