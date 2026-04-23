# Platform Backup Resilience

**Bead:** bd-o5znl  
**Status:** Decision — ready for implementation  
**Author:** BrightSparrow  
**Date:** 2026-04-23

---

## Problem Statement

Each of the 20 PostgreSQL instances in the platform runs independent backups with no offsite storage, no immutability, and no tested restore procedure. A ransomware event or host destruction can wipe all backups simultaneously. This document makes architectural decisions and defines the implementation sequence.

---

## Decision Summary

| Question | Decision |
|----------|----------|
| Storage target | GCS (same vendor as Secret Manager — no new IAM surface) |
| Backup method | WAL-G with continuous WAL archiving (enables PITR) |
| Immutability | GCS Object Lock in WORM mode on a dedicated bucket |
| Retention | 7 daily / 4 weekly / 12 monthly (GFS scheme) |
| Scope | All 20 module databases + tenant-registry (highest priority) |
| Deployment | Single backup-orchestrator container; WAL archiving configured per postgres |
| Alerting | NATS event on backup failure → notifications module |
| Restore drill | Quarterly: restore to isolated container, run smoke tests |

---

## 1. Storage Target: GCS

**Decision: GCS bucket in a separate GCP project.**

Rationale:
- Platform already uses GCP Secret Manager. GCS adds no new vendor or auth surface.
- Placing the bucket in a **separate GCP project** means a compromised application service account cannot access backup storage — different project, different IAM binding.
- GCS Object Lock (WORM) is available on GCS but not on all S3-compatible services we evaluated.

The bucket name convention: `7d-platform-backups-<env>` (e.g., `7d-platform-backups-prod`).

Object Lock must be enabled **at bucket creation** — it cannot be retrofitted. All implementation work must start with bucket creation before any backup runs.

---

## 2. Backup Method: WAL-G

**Decision: WAL-G with continuous WAL archiving. pg_dump is not sufficient.**

Why WAL-G over pg_dump:
- pg_dump produces point-in-time snapshots. If the last snapshot was 23 hours ago, up to 23 hours of data is lost on restore.
- WAL-G archives PostgreSQL write-ahead logs continuously (every completed WAL segment, typically every 16 MB or 5 minutes). This enables Point-In-Time Recovery (PITR) — restore to any moment, not just snapshot intervals.
- WAL-G handles base backups (equivalent to pg_dump) plus the WAL stream, combining both into a coherent restore chain.

What this requires:
- Each postgres container must have `wal_level = replica` and `archive_mode = on` set, plus `archive_command` pointing to WAL-G.
- WAL-G configuration is passed via environment variables: `WALG_GS_PREFIX`, `GOOGLE_APPLICATION_CREDENTIALS`.
- WAL-G can be installed as a sidecar or run inside each postgres container via a shared volume.

**Deployment choice for WAL-G:** Each postgres container gets WAL-G installed and runs its own `archive_command`. A single backup-orchestrator container schedules full base backups (`wal-g backup-push`) across all databases via cron and handles GCS authentication. The per-postgres WAL archiving is autonomous — it does not depend on the orchestrator container being healthy.

---

## 3. Immutability: GCS Object Lock (WORM)

**Decision: Retention lock in Compliance mode, 30-day minimum retention.**

GCS Object Lock in **Compliance mode** prevents deletion by any principal, including project owners and the backup service account. Once an object is written with a retention policy, no one can delete it until the retention period expires.

Configuration:
- Bucket-level default retention: 30 days (covers all daily backups in the retention window)
- Objects get per-object retention set at write time matching the backup tier (daily = 7 days, weekly = 28 days, monthly = 365 days)
- The service account used by WAL-G is granted `storage.objectCreator` only — it cannot delete or modify objects

This means even if application credentials are fully compromised, backups cannot be wiped.

---

## 4. Retention Policy

**Decision: GFS (Grandfather-Father-Son) — 7 daily / 4 weekly / 12 monthly.**

| Tier | Count | Retention per object |
|------|-------|----------------------|
| Daily base backup | 7 | 7 days |
| Weekly base backup (Sunday) | 4 | 28 days |
| Monthly base backup (1st of month) | 12 | 365 days |
| WAL segments | continuous | 7 days |

WAL-G enforces this via `wal-g delete retain FULL 7 --before-time <date>` on a scheduled basis. The orchestrator container runs the delete job after each successful backup cycle. GCS Object Lock provides the floor — the delete job cannot remove objects whose retention has not expired.

---

## 5. Scope: All 20 Module Databases

All databases in `docker-compose.infrastructure.yml`:

| Database | Priority | Notes |
|----------|----------|-------|
| `tenant-registry-postgres` | **CRITICAL** | Cross-cutting tenant identity; loss here affects all verticals |
| `auth-postgres` | HIGH | Session and credential data |
| `gl-postgres` | HIGH | Ledger — financial record |
| `ar-postgres` | HIGH | Invoicing and receivables |
| `ap-postgres` | HIGH | Payables |
| `payments-postgres` | HIGH | Payment records |
| `subscriptions-postgres` | HIGH | Billing agreements |
| `treasury-postgres` | HIGH | Cash management |
| `inventory-postgres` | MEDIUM | Stock ledger |
| `party-postgres` | MEDIUM | Customer/supplier master |
| `fixed-assets-postgres` | MEDIUM | Asset register |
| `consolidation-postgres` | MEDIUM | Multi-entity rollup |
| `integrations-postgres` | MEDIUM | Carrier/external credentials |
| `production-postgres` | MEDIUM | Work order state |
| `timekeeping-postgres` | MEDIUM | Labor records |
| `maintenance-postgres` | MEDIUM | Asset maintenance history |
| `ttp-postgres` | MEDIUM | Waste hauling operational data |
| `notifications-postgres` | LOW | Event log; reconstructible from NATS |
| `projections-postgres` | LOW | Read-model; rebuildable from source events |
| `audit-postgres` | LOW | Append-only audit log; can be reconstructed but retention matters |
| `pdf-editor-postgres` | LOW | Document metadata |

Implementation order: tenant-registry and financial DBs first (CRITICAL/HIGH), then the rest in a second batch.

---

## 6. Deployment: Backup Orchestrator Container

**Decision: Single `backup-orchestrator` Docker Compose service, plus WAL-G archive_command in each postgres container.**

Architecture:
```
┌─────────────────────────────────────────────────┐
│  docker-compose.infrastructure.yml              │
│                                                 │
│  ┌──────────────────────────────────────────┐  │
│  │ backup-orchestrator                       │  │
│  │  - Runs WAL-G base backup cron (daily)   │  │
│  │  - Runs WAL-G delete/retain cron (daily) │  │
│  │  - Publishes NATS events on failure      │  │
│  │  - Mounts GCP service account JSON       │  │
│  └──────────────────────────────────────────┘  │
│                                                 │
│  ┌──────────────┐  ┌──────────────┐  ...       │
│  │ gl-postgres  │  │ ar-postgres  │            │
│  │ + WAL-G      │  │ + WAL-G      │            │
│  │ archive_cmd  │  │ archive_cmd  │            │
│  └──────────────┘  └──────────────┘            │
└─────────────────────────────────────────────────┘
                         │
                  GCS (separate project)
                  7d-platform-backups-prod
```

The backup-orchestrator is a minimal Alpine container with:
- WAL-G binary
- `supercronic` for cron scheduling (no cron daemon needed)
- GCP service account credentials from Secret Manager
- Shell scripts for each backup operation

Each postgres container is extended with:
- `WALG_GS_PREFIX=gs://7d-platform-backups-prod/<db-name>`
- `archive_mode = on` and `archive_command = 'wal-g wal-push %p'` in postgresql.conf
- WAL-G binary mounted or installed in the postgres image layer

**Why not systemd on the host:** Docker Compose is the operational unit of this platform. Introducing systemd units on the host creates a second deployment mechanism to maintain and monitor. All backup concerns stay inside the compose stack.

---

## 7. Alerting on Backup Failure

**Decision: NATS publish on failure → notifications module handles delivery.**

Each backup script exits with a non-zero code on failure. The orchestrator container's wrapper detects this and publishes a NATS event:

```json
{
  "event_type": "backup.failed",
  "source": "backup-orchestrator",
  "database": "gl_db",
  "backup_type": "base|wal",
  "error": "...",
  "timestamp": "2026-04-23T..."
}
```

The notifications module subscribes to `backup.failed` and routes to the on-call channel (email or Slack, configured by the tenant).

Additionally: a **heartbeat** event `backup.completed` is published after each successful cycle. A separate alerting rule fires if no heartbeat arrives within 25 hours — this catches silent failures (container crashed, network partition to GCS).

---

## 8. Restore Drill Procedure

**Decision: Quarterly drill. Automated script, manual sign-off.**

Drill procedure (run in staging or an isolated environment):

```bash
# 1. Identify the restore target
wal-g backup-list --detail  # pick latest FULL backup

# 2. Spin up an isolated postgres container
docker run -d --name restore-test -e POSTGRES_PASSWORD=... postgres:16-alpine

# 3. Restore base backup
docker exec restore-test wal-g backup-fetch /var/lib/postgresql/data LATEST

# 4. Replay WAL to a specific point in time (or current)
# Set recovery.conf / recovery parameters

# 5. Start postgres in recovery mode, wait for promotion

# 6. Run smoke tests against the restored DB
./scripts/smoke-test-restored-db.sh <db-name>

# 7. Record result in drill log
echo "$(date): drill PASSED for gl_db, restore target: <backup-name>" >> docs/ops/restore-drill-log.md
```

Smoke tests per database verify:
- Schema version matches expected migration level
- Row counts are non-zero for key tables
- A known reference record is present (e.g., a specific tenant in tenant-registry)

The drill log at `docs/ops/restore-drill-log.md` is updated after each drill with date, database tested, backup age, and pass/fail. A failed drill is a P0 incident.

---

## Implementation Bead Sequence

These beads should be created in order. Each bead is a prerequisite for the next.

### Bead 1: GCS bucket + IAM setup
- Create `7d-platform-backups-prod` in a separate GCP project
- Enable Object Lock (Compliance mode, 30-day default)
- Create dedicated service account `platform-backup@<project>.iam.gserviceaccount.com`
- Grant `storage.objectCreator` only (no delete)
- Store service account JSON in Secret Manager as `BACKUP_GCS_SERVICE_ACCOUNT`
- Verify: `gsutil ls gs://7d-platform-backups-prod/` from a container with the SA credentials

### Bead 2: WAL-G configuration for postgres containers (CRITICAL/HIGH tier)
- Add WAL-G binary to postgres images (or mount via shared volume)
- Add `archive_mode = on`, `archive_command = 'wal-g wal-push %p'` to each postgres container's env/config
- Set `WALG_GS_PREFIX` per database
- Scope: tenant-registry, auth, gl, ar, ap, payments, subscriptions, treasury
- Verify: trigger a WAL segment flush, confirm object appears in GCS

### Bead 3: WAL-G configuration for remaining databases (MEDIUM/LOW tier)
- Same as Bead 2 for: inventory, party, fixed-assets, consolidation, integrations, production, timekeeping, maintenance, ttp, notifications, projections, audit, pdf-editor
- Verify: same as Bead 2

### Bead 4: Backup orchestrator container
- Add `backup-orchestrator` service to `docker-compose.infrastructure.yml`
- Implement base backup cron (daily at 02:00 UTC)
- Implement retention delete cron (daily at 03:00 UTC)
- Implement heartbeat NATS publish on success
- Implement failure NATS publish on error
- Verify: trigger manual base backup for all 20 databases, confirm objects in GCS with correct retention metadata

### Bead 5: GCS lifecycle policy
- Add GCS lifecycle rules to enforce per-tier retention floors
- Daily objects: delete after 8 days (Object Lock provides the hard floor)
- Weekly objects: delete after 29 days
- Monthly objects: delete after 366 days
- Verify: lifecycle policy visible in GCS console, no unexpired locked objects deletable

### Bead 6: Alerting integration
- Subscribe notifications module to `backup.failed` and `backup.heartbeat.missed` NATS subjects
- Wire to on-call delivery channel (email initially)
- Verify: trigger a deliberate backup failure, confirm notification delivered

### Bead 7: Restore drill runbook + automation
- Create `scripts/restore-drill.sh` implementing the drill procedure
- Create `docs/ops/restore-drill-log.md`
- Create `scripts/smoke-test-restored-db.sh` with per-database smoke test logic
- Run first drill against gl_db and tenant-registry
- Record results in drill log
- Verify: drill script exits 0, log file updated

---

## Open Questions (Non-blocking)

1. **Encryption at rest**: GCS encrypts by default with Google-managed keys. Platform may want Customer-Managed Encryption Keys (CMEK) for compliance. Deferred — Fireproof aerospace customer should weigh in on whether CMEK is a compliance requirement.

2. **Cross-region replication**: Bucket can be multi-region (`US` or `EU`). Decision deferred pending where the production host is located. Default: `us-central1` single region, upgrade to multi-region if uptime SLA requires.

3. **Backup verification (automated restore testing)**: The quarterly drill is manual. Automating a nightly restore-verify cycle is possible but adds complexity. Deferred to after the first successful manual drill cycle.
