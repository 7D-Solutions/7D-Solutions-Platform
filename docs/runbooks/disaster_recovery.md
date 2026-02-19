# Disaster Recovery Runbook

**Phase 34 — Hardening / Launch Readiness**

## Purpose

End-to-end disaster recovery procedures for the 7D Solutions Platform.
Defines RPO/RTO targets, recovery procedures, validation steps, and
a repeatable quarterly drill process.

## RPO / RTO Targets

| Tier | Services | RPO | RTO | Notes |
|------|----------|-----|-----|-------|
| **Critical** | GL, AR, AP, Payments, Treasury | 1 hour | 4 hours | Financial data — SOX-sensitive |
| **High** | Subscriptions, Inventory, Fixed Assets, Consolidation | 1 hour | 4 hours | Core business operations |
| **Standard** | Notifications, Projections, Timekeeping, Party, Integrations | 4 hours | 8 hours | Supportive services |
| **Platform** | Auth, Tenant Registry, Audit | 1 hour | 2 hours | Must restore first — all services depend on these |
| **Infrastructure** | NATS (event bus) | 0 (durable) | 1 hour | JetStream persists to disk; rebuilt from config |

**RPO** = Recovery Point Objective (max acceptable data loss).
**RTO** = Recovery Time Objective (max acceptable downtime).

### Backup Schedule (to meet RPO)

| Target | Frequency | Retention |
|--------|-----------|-----------|
| All databases | Every 1 hour (production) | 7 days on hot storage |
| All databases | Daily at 02:00 UTC | 90 days on object store |
| Financial databases (GL, AR, AP, Payments, Treasury) | Daily | 7 years (SOX) |
| NATS JetStream data | Replicated (3-node cluster in prod) | N/A |

## Prerequisites

- `pg_dump`, `psql`, `pg_isready` on PATH (`brew install libpq` on macOS)
- Docker Compose stack accessible (or target databases reachable)
- Python 3 (for manifest parsing)
- Access to backup storage (local `./backups/` or remote object store)
- `scripts/backup_all.sh` and `scripts/restore_all.sh` present

## Recovery Procedure

### Phase 0: Detection & Assessment (target: < 15 min)

1. **Identify the failure scope** — which services, databases, or infrastructure are affected.
2. **Check monitoring** — review alerts, error logs, and healthcheck status.
3. **Classify severity:**
   - **Total loss**: All databases gone (e.g., disk failure, zone outage)
   - **Partial loss**: One or more module databases corrupted or lost
   - **Service degradation**: Databases intact but services cannot connect
4. **Declare DR** if estimated recovery exceeds normal restart procedures.

### Phase 1: Stop Application Services

Prevent writes to corrupted or partially restored databases.

```bash
docker compose -f docker-compose.modules.yml down
```

Verify no module processes are running:

```bash
docker ps --filter "label=com.7dsolutions.tier=module" --format "{{.Names}}"
# Should return empty
```

### Phase 2: Identify Latest Good Backup

```bash
# List available backups (most recent first)
ls -1td backups/*/

# Inspect the latest manifest
BACKUP_DIR="$(ls -1td backups/*/ | head -1)"
python3 -c "
import json
m = json.load(open('${BACKUP_DIR}/manifest.json'))
print(f\"Timestamp: {m['timestamp']}\")
print(f\"Errors:    {m['errors']}\")
print(f\"Databases: {len(m['databases'])}\")
for db in m['databases']:
    print(f\"  {db['name']:20s} {db['size_bytes']:>10,} bytes  {len(db.get('tables',[]))} tables\")
"

# Verify gzip integrity of all dumps
for f in "${BACKUP_DIR}"/*.sql.gz; do
  gunzip -t "$f" && echo "OK: $f" || echo "CORRUPT: $f"
done
```

If the latest backup is corrupt, move to the next most recent.

### Phase 3: Restore Platform Tier First

Platform databases (Auth, Tenant Registry, Audit) must be restored before
module databases because services depend on auth tokens and tenant routing.

```bash
# Restore platform databases individually (order matters)
for MODULE in auth tenant_registry audit; do
  DB="${MODULE}_db"
  # ... use single-database restore procedure from backup_restore.md
done
```

Or use the full restore which handles all databases:

```bash
bash scripts/restore_all.sh "${BACKUP_DIR}"
```

### Phase 4: Restore Module Databases

If using single-database restore (partial failure), restore in tier order:

1. **Critical**: gl, ar, ap, payments, treasury
2. **High**: subscriptions, inventory, fixed_assets, consolidation
3. **Standard**: notifications, projections, timekeeping, party, integrations

### Phase 5: Verify Data Integrity

Run smoke test against restored databases:

```bash
bash scripts/restore_all.sh "${BACKUP_DIR}" --smoke-test
```

Expected output: all `PASS` lines, exit code 0.

### Phase 6: Restore Infrastructure

**NATS JetStream**: In production, JetStream data is replicated across the
cluster. If the NATS node is lost:

```bash
# NATS rebuilds from other cluster members on restart
docker compose -f docker-compose.infrastructure.yml up -d nats

# Verify streams exist
nats stream list
```

If all NATS nodes are lost, streams are recreated by services on startup
(services declare their streams idempotently). Events between last consumer
ack and failure are replayed from the outbox tables.

### Phase 7: Restart Application Services

```bash
docker compose -f docker-compose.modules.yml up -d
```

Verify all healthchecks pass:

```bash
docker compose -f docker-compose.modules.yml ps
# All services should show "healthy"
```

### Phase 8: Post-Recovery Validation

1. **Hit each service health endpoint** to confirm connectivity:
   ```bash
   for port in 8081 8082 8083 8084 8085 8086 8087 8088 8089 8090 8091 8092 8093 8094 8095 8096 8097 8098 8099; do
     curl -sf "http://localhost:${port}/health" && echo " OK :${port}" || echo " FAIL :${port}"
   done
   ```

2. **Run E2E test suite** (non-destructive read-only checks):
   ```bash
   AUDIT_DATABASE_URL=postgres://postgres:postgres@localhost:5432/audit_db \
   PROJECTIONS_DATABASE_URL=postgres://postgres:postgres@localhost:5432/projections_db \
   TENANT_REGISTRY_DATABASE_URL=postgres://postgres:postgres@localhost:5432/tenant_registry_db \
   cargo test -p e2e-tests --no-fail-fast -- --nocapture
   ```

3. **Verify NATS consumers are draining** (no stuck messages):
   ```bash
   nats consumer list --all
   ```

4. **Take a fresh backup** of the recovered state:
   ```bash
   bash scripts/backup_all.sh
   ```

## Quarterly DR Drill

Run `scripts/dr_drill.sh` to execute a non-destructive DR drill that validates
backup integrity, restore capability, and service health without affecting
production data.

```bash
# Preview what the drill will do
bash scripts/dr_drill.sh --dry-run

# Run the full drill (creates timestamped report)
bash scripts/dr_drill.sh
```

The drill script:
1. Validates all databases are reachable
2. Creates a fresh backup
3. Verifies backup integrity (gzip + manifest)
4. Runs smoke-test verification against live databases
5. Checks NATS connectivity and stream health
6. Checks service health endpoints
7. Produces a timestamped report in `dr-reports/`

Schedule quarterly: January, April, July, October.

## Roles & Responsibilities

| Role | Responsibility |
|------|---------------|
| **On-call engineer** | Detect, assess, declare DR; execute runbook |
| **Platform lead** | Approve DR declaration; coordinate comms |
| **DBA** | Validate backup integrity; assist with restore |
| **QA** | Run post-recovery E2E validation |

## Communication Template

```
SUBJECT: [DR] 7D Solutions Platform — Recovery {In Progress | Complete}

Scope:     {Total / Partial — list affected services}
Detected:  {timestamp}
Declared:  {timestamp}
RPO met:   {Yes/No — data loss window}
RTO met:   {Yes/No — time to restore}
Status:    {Restoring / Validating / Recovered}
Next step: {description}
```

## Rollback

If a restored database causes issues (e.g., schema mismatch after migration):

1. Stop affected service
2. Restore from the *previous* backup set
3. Re-run smoke test
4. If persistent: escalate to DBA for manual inspection

## References

- `scripts/backup_all.sh` — automated backup
- `scripts/restore_all.sh` — automated restore + smoke-test
- `scripts/dr_drill.sh` — quarterly drill automation
- `docs/runbooks/backup_restore.md` — detailed backup/restore procedures
- `docker-compose.infrastructure.yml` — database port/credential defaults
- `docker-compose.modules.yml` — application services

## Changelog

- **2026-02-19**: Phase 34 — initial DR runbook with RPO/RTO targets (bd-12k9)
