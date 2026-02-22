# Disaster Recovery Runbook

**Phase 48 — Production Hardening (last updated: bd-3len)**

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
# auth → tenant_registry → audit
for MODULE in auth tenant_registry audit; do
  DB="${MODULE}_db"
  USER="${MODULE}_user"
  PORT_MAP="auth:5433 tenant_registry:5441 audit:5440"
  PORT=$(echo "$PORT_MAP" | tr ' ' '\n' | grep "^${MODULE}:" | cut -d: -f2)
  gunzip -c "${BACKUP_DIR}/${MODULE}.sql.gz" | \
    docker exec -i "7d-${MODULE//_/-}-postgres" psql -U "${USER}" -d "${DB}"
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
   # Platform
   curl -sf http://localhost:8080/healthz    && echo " OK auth"         || echo " FAIL auth"
   curl -sf http://localhost:8091/api/ready  && echo " OK control-plane" || echo " FAIL control-plane"

   # Billing spine (check first)
   for svc_port in "ar:8086" "subscriptions:8087" "payments:8088" "ttp:8100"; do
     svc="${svc_port%%:*}"; port="${svc_port##*:}"
     curl -sf "http://localhost:${port}/api/health" && echo " OK ${svc}" || echo " FAIL ${svc}"
   done

   # All other modules
   for svc_port in \
     "notifications:8089" "gl:8090" "inventory:8092" \
     "ap:8093" "treasury:8094" "fixed-assets:8095" \
     "consolidation:8096" "timekeeping:8097" "party:8098" "integrations:8099"; do
     svc="${svc_port%%:*}"; port="${svc_port##*:}"
     curl -sf "http://localhost:${port}/api/health" && echo " OK ${svc}" || echo " FAIL ${svc}"
   done
   ```

2. **Run E2E test suite** (non-destructive read-only checks):
   ```bash
   ./scripts/cargo-slot.sh test -p e2e-tests --no-fail-fast -- --nocapture
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

---

## Decision Trees

### Scope Classification

Use this tree to classify the incident before taking action:

```
All services down?
├── YES → Total loss (disk, VPS, or Docker daemon failure)
│   → Declare DR immediately. Follow full recovery procedure above.
└── NO  → One or more services affected
    Are databases intact (docker exec psql connects)?
    ├── NO  → Partial DB loss → restore only affected module DBs (Phase 4)
    └── YES → Service degradation → skip restore, go to Phase 7 (restart services)
              Check for outbox lag or NATS issue before assuming DB problem.
```

### Rollback vs. DR Decision

```
Did a deploy precede the failure (within 30 min)?
├── YES → Attempt rollback first (faster than DR):
│   1. bash /opt/7d-platform/scripts/production/rollback_stack.sh
│   2. bash /opt/7d-platform/scripts/production/smoke.sh
│   3. If rollback succeeds → exit DR mode. Verify GL balance.
│   4. If rollback fails (DB schema already migrated) → proceed with DR restore
└── NO  → Infrastructure failure → go directly to DR procedure above.
```

### Webhook Failure During DR Recovery

If webhook failures appear after services restart:

```
Are NATS consumers draining (nats consumer report PLATFORM)?
├── NO (lag rising) → NATS relay is stuck. Restart affected service.
└── YES → Check outbox tables for un-published events:
    docker exec 7d-payments-postgres psql -U payments_user -d payments_db \
      -c "SELECT COUNT(*) FROM outbox WHERE published_at IS NULL;"
    If > 0 → restart the service; the relay will re-publish on startup.
    If 0 but webhook deliveries failing → tenant endpoint is down; events will
    exhaust retries into DLQ. Follow DLQ Replay procedure in incident_response.md.
```

---

## References

- `scripts/production/rollback_stack.sh` — automated rollback
- `scripts/production/smoke.sh` — post-recovery smoke test
- `scripts/production/log_bundle.sh` — capture diagnostic log bundle
- `scripts/backup_all.sh` — automated backup
- `scripts/restore_all.sh` — automated restore + smoke-test
- `scripts/dr_drill.sh` — quarterly drill automation
- `docs/runbooks/backup_restore.md` — detailed backup/restore procedures
- `docs/runbooks/incident_response.md` — rollback and webhook failure decision trees
- `docker-compose.infrastructure.yml` — database port/credential defaults
- `docker-compose.modules.yml` — application services

## Changelog

- **2026-02-22**: Phase 48 — add Decision Trees for scope classification, rollback vs DR, and post-DR webhook failure; fix Phase 3 restore commands; fix Phase 8 health check port list to match actual service ports; update E2E test command to use cargo-slot.sh (bd-3len)
- **2026-02-19**: Phase 34 — initial DR runbook with RPO/RTO targets (bd-12k9)
