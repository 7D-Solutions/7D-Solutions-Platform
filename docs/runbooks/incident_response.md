# Incident Response Runbook

**Phase 48 — Production Hardening (last updated: bd-3len)**

## Purpose

Step-by-step procedures for responding to platform incidents: service outages,
alert threshold breaches, data-integrity violations, and DLQ exhaustion events.

## Severity Classification

| Severity | Criteria | Response SLA | Escalation |
|----------|----------|-------------|------------|
| **P1 — Critical** | Service down, financial data at risk, GL imbalance, > 5 invariant violations | < 15 min page response | On-call → Eng lead → CTO |
| **P2 — High** | Elevated DLQ rate, single module degraded, UNKNOWN entities > 4 h | < 1 hour | On-call → Team lead |
| **P3 — Warning** | Approaching thresholds, slow queries, backup anomaly | < 4 hours (business hours) | On-call |

## Alert Response Matrix

| Alert | Severity | Runbook section |
|-------|----------|----------------|
| `UnknownInvoiceDurationCritical` | P2 | [UNKNOWN Resolution](#unknown-protocol-resolution) |
| `GLInvariantViolationCritical` | P1 | [Invariant Violations](#invariant-violations) |
| `DLQEventRateCritical` | P1/P2 | [DLQ Replay](#dlq-replay) |
| `OutboxInsertFailure` | P1 | [Outbox Failures](#outbox-atomicity-failures) |
| Service health check failing | P1 | [Service Recovery](#service-recovery) |

---

## UNKNOWN Protocol Resolution

**Background**: UNKNOWN is a valid state indicating business logic uncertainty
(e.g., idempotency key collision, partial payment, ambiguous invoice finalization).
It must resolve within the retry window — alerts fire at 1 h warning, 4 h critical.

### Step 1: Identify stuck entities

```bash
# AR — invoices stuck in UNKNOWN
docker exec 7d-ar-postgres psql -U ar_user -d ar_db -c "
  SELECT id, created_at, updated_at, now() - updated_at AS age
  FROM invoices
  WHERE status = 'unknown'
  ORDER BY updated_at
  LIMIT 20;
"

# Payments — stuck payment attempts
docker exec 7d-payments-postgres psql -U payments_user -d payments_db -c "
  SELECT id, created_at, updated_at, now() - updated_at AS age
  FROM payment_attempts
  WHERE status = 'unknown'
  ORDER BY updated_at
  LIMIT 20;
"
```

### Step 2: Check NATS for pending retry events

```bash
# List consumers with pending messages for the affected subject
nats consumer info PLATFORM invoice.finalization.requested --server localhost:4222

# Check DLQ for exhausted retries
nats consumer info PLATFORM invoice.finalization.requested.DLQ --server localhost:4222
```

### Step 3: Replay or manually resolve

**Option A — replay from outbox** (if retry window not exhausted):
```bash
# Outbox events are replayed automatically by NATS retry policy.
# Force re-delivery by resetting consumer sequence:
nats consumer reset PLATFORM invoice.finalization.requested \
  --subject "invoice.finalization.requested" \
  --server localhost:4222
```

**Option B — manual state transition** (P1 escalation required):
```bash
# Transition specific invoice to a terminal state with audit note
PGPASSWORD=ar_pass psql -h localhost -p 5434 -U ar_user -d ar_db << 'SQL'
BEGIN;
UPDATE invoices
  SET status = 'failed',
      updated_at = NOW(),
      metadata = metadata || '{"manual_resolution": "P1 incident 2026-XX-XX, ops override"}'
WHERE id = '<invoice-id>' AND status = 'unknown';
-- verify exactly 1 row affected before committing
COMMIT;
SQL
```

**Step 4: Verify resolution**
```bash
# Confirm no remaining UNKNOWNs beyond 1 hour
PGPASSWORD=ar_pass psql -h localhost -p 5434 -U ar_user -d ar_db -c "
  SELECT COUNT(*) FROM invoices
  WHERE status = 'unknown' AND updated_at < NOW() - INTERVAL '1 hour';
"
# Should return 0
```

---

## DLQ Replay

**Background**: Events in the Dead Letter Queue (DLQ) have exhausted all retry
attempts. They require investigation before replay to avoid re-triggering
the root cause.

### Step 1: Inspect DLQ contents

```bash
# List DLQ subjects with pending counts
nats stream info PLATFORM --server localhost:4222 | grep -A5 "Consumer"

# View DLQ messages (don't ack — inspect only)
nats consumer next PLATFORM <SUBJECT>.DLQ \
  --count 5 --no-ack --server localhost:4222
```

### Step 2: Identify root cause

Check service logs for the error that caused exhaustion:
```bash
docker logs 7d-ar --tail 200 | grep -E "ERROR|DLQ|retry_exhausted"
docker logs 7d-gl --tail 200 | grep -E "ERROR|DLQ|retry_exhausted"
```

### Step 3: Fix root cause

Do NOT replay until the root cause is resolved. Common causes:

| Cause | Fix |
|-------|-----|
| Schema migration not applied | Run migrations, then replay |
| Downstream service unavailable | Restore service, then replay |
| Malformed event (schema drift) | Transform events, then replay |
| Database constraint violation | Investigate data, fix constraint or event |

### Step 4: Replay after fix

```bash
# Move DLQ messages back to the main stream for reprocessing
nats consumer reset PLATFORM <SUBJECT>.DLQ \
  --subject "<SUBJECT>" \
  --server localhost:4222

# Monitor consumption
nats consumer info PLATFORM <SUBJECT> --server localhost:4222
```

### Step 5: Verify GL integrity after replay

```bash
PGPASSWORD=gl_pass psql -h localhost -p 5438 -U gl_user -d gl_db -c "
  SELECT
    SUM(debit_cents) AS debits,
    SUM(credit_cents) AS credits,
    SUM(debit_cents) - SUM(credit_cents) AS imbalance
  FROM journal_entries;
"
# imbalance MUST be 0
```

---

## Invariant Violations

**Background**: Invariant violations indicate data corruption or business logic
bugs. Zero tolerance in production — any non-zero count requires investigation.

### Step 1: Identify violated invariants

```bash
# Check module metrics endpoints
for port in 8086 8087 8088 8090 8093 8094; do
  echo "Port $port:"
  curl -sf "http://localhost:${port}/metrics" | grep "invariant_violations"
done
```

### Step 2: Inspect violation details

```bash
# AR: no unknown outside retry window
PGPASSWORD=ar_pass psql -h localhost -p 5434 -U ar_user -d ar_db -c "
  SELECT id, status, created_at, updated_at
  FROM invoices
  WHERE status = 'unknown'
    AND updated_at < NOW() - INTERVAL '6 hours';
"

# GL: double-entry balance check
PGPASSWORD=gl_pass psql -h localhost -p 5438 -U gl_user -d gl_db -c "
  SELECT tenant_id,
         SUM(debit_cents)  AS debits,
         SUM(credit_cents) AS credits,
         SUM(debit_cents) - SUM(credit_cents) AS imbalance
  FROM journal_entries
  GROUP BY tenant_id
  HAVING SUM(debit_cents) <> SUM(credit_cents);
"

# AR: no duplicate invoices per cycle
PGPASSWORD=ar_pass psql -h localhost -p 5434 -U ar_user -d ar_db -c "
  SELECT tenant_id, billing_period_start, COUNT(*) AS cnt
  FROM invoices
  GROUP BY tenant_id, billing_period_start
  HAVING COUNT(*) > 1;
"
```

### Step 3: Determine scope

- **Isolated row(s)**: Manual correction with audit trail (P1 approval required)
- **Pattern across tenants**: Suspect deployment; consider rollback
- **GL imbalance**: Freeze financial reporting, escalate immediately

### Step 4: Freeze if necessary

```bash
# Stop module service to prevent further writes during investigation
docker compose -f docker-compose.modules.yml stop 7d-ar

# Restart after fix
docker compose -f docker-compose.modules.yml start 7d-ar
```

---

## Outbox Atomicity Failures

**Background**: The Guard → Mutation → Outbox pattern requires that every domain
mutation inserts an outbox event in the same transaction. Failures here risk
state drift where the database reflects state but NATS has no corresponding event.

### Step 1: Identify missing events

```bash
# Check outbox table for pending/failed events
PGPASSWORD=ar_pass psql -h localhost -p 5434 -U ar_user -d ar_db -c "
  SELECT event_type, COUNT(*), MAX(created_at) AS latest
  FROM outbox_events
  WHERE published_at IS NULL
  GROUP BY event_type
  ORDER BY COUNT(*) DESC;
"
```

### Step 2: Check service logs for transaction errors

```bash
docker logs 7d-ar --tail 500 | grep -E "outbox|transaction|rollback"
```

### Step 3: Re-publish orphaned outbox events

If events are in the outbox but not yet published to NATS:
```bash
# The outbox relay runs as part of each service. If it's stuck:
docker compose -f docker-compose.modules.yml restart 7d-ar

# Verify relay picks up pending events
docker logs 7d-ar --follow | grep "outbox published"
```

---

## Service Recovery

### Health check all services

```bash
# Platform
curl -sf http://localhost:8080/api/health && echo " OK auth"

# Modules
for svc_port in \
  "ar:8086" "subscriptions:8087" "payments:8088" "notifications:8089" \
  "gl:8090" "inventory:8092" "ap:8093" "treasury:8094" \
  "fixed-assets:8095" "consolidation:8096" "timekeeping:8097" \
  "party:8098" "integrations:8099" "ttp:8100"; do
  svc="${svc_port%%:*}"
  port="${svc_port##*:}"
  curl -sf "http://localhost:${port}/api/health" \
    && echo " OK ${svc}" \
    || echo " FAIL ${svc}:${port}"
done
```

### Restart a single service

```bash
docker compose -f docker-compose.modules.yml restart 7d-ar
docker compose -f docker-compose.modules.yml logs -f 7d-ar | grep -E "started|error|panic"
```

### Restart all module services

```bash
docker compose -f docker-compose.modules.yml restart
docker compose -f docker-compose.modules.yml ps
# All should show "healthy"
```

### NATS connectivity check

```bash
# Verify NATS is reachable
nats server check connection --server localhost:4222

# List all streams and consumer lag
nats stream list --server localhost:4222
nats consumer list PLATFORM --server localhost:4222
```

### Full service recovery (including databases)

See [disaster_recovery.md](disaster_recovery.md) for full DR procedure.

---

## Post-Incident

### Evidence checklist

After resolving any P1 or P2 incident:

- [ ] Root cause identified and documented
- [ ] Data integrity verified (GL balance, invariant checks)
- [ ] All DLQ events resolved or tracked
- [ ] Alerts silenced only after underlying issue is fixed (never mask)
- [ ] Fresh backup taken of recovered state: `bash scripts/backup_all.sh`
- [ ] Post-mortem scheduled (within 48 h for P1)

### Communication template

```
SUBJECT: [INCIDENT] 7D Solutions Platform — {P1/P2} {Resolved/In Progress}

Severity:  P{1/2}
Detected:  {timestamp}
Resolved:  {timestamp} (or "In progress")
Impact:    {services affected, data risk if any}
Root cause: {brief description}
Fix applied: {what was done}
Next steps: {monitoring, post-mortem, follow-up beads}
```

---

---

## Decision Trees

### Scenario 1: Rollback Incident

**Trigger**: A deployment caused a regression — elevated 5xx rate, health-check failures, or GL/AR invariant violation within 15 min of deploy.

```
Did health checks pass immediately after deploy?
├── YES → Monitor for 10 min. Is error rate elevated vs pre-deploy?
│   ├── NO  → Deploy is good. Continue monitoring.
│   └── YES → Treat as rollback scenario (start below)
└── NO  → Rollback immediately (start below)

ROLLBACK DECISION:
Is the failure isolated to ONE service?
├── YES → Single-module rollback:
│   1. docker compose -f docker-compose.modules.yml stop 7d-<svc>
│   2. Edit deploy/production/MODULE-MANIFEST.md → revert image tag to previous
│   3. bash /opt/7d-platform/scripts/production/rollback_stack.sh
│   4. Verify: curl http://localhost:<port>/api/health
│   5. Confirm GL balance unchanged:
│      docker exec 7d-gl-postgres psql -U gl_user -d gl_db \
│        -c "SELECT SUM(debit_cents)-SUM(credit_cents) AS imbalance FROM journal_entries;"
│      # Must be 0
└── NO  → Full stack rollback:
    1. bash /opt/7d-platform/scripts/production/rollback_stack.sh
    2. Verify all services healthy: bash /opt/7d-platform/scripts/production/smoke.sh
    3. Run GL integrity check (above)
    4. If rollback itself fails → declare DR, follow disaster_recovery.md

Post-rollback actions:
- Capture log bundle: bash /opt/7d-platform/scripts/production/log_bundle.sh
- Create incident bead documenting root cause
- Schedule post-mortem within 48 h (P1) or 1 week (P2)
```

---

### Scenario 2: Webhook Failure Incident

**Trigger**: `WebhookFailureRateCritical` alert OR payments-spine UNKNOWN rate rising AND recent webhook delivery failures in logs.

```
Are webhook failures isolated to ONE tenant?
├── YES → Tenant webhook endpoint issue:
│   1. Check tenant's configured webhook URL (via TCP UI or tenant-registry DB)
│   2. Verify the endpoint is reachable: curl -I <tenant-webhook-url>
│   3. If endpoint is down: pause webhook retries (AR/Payments will queue)
│      Event will exhaust retries → DLQ → manual re-delivery when endpoint recovers
│   4. Follow up with tenant to restore their endpoint
└── NO  → Platform-wide webhook issue:
    Is the failure on INBOUND webhooks (Tilled → Payments)?
    ├── YES → Tilled delivery failure:
    │   1. Check Tilled dashboard for delivery errors / status
    │   2. Verify signature secret has not rotated without updating secrets:
    │      grep TILLED_WEBHOOK_SECRET /etc/7d/production/secrets.env
    │   3. Check payments logs: docker logs --tail 500 7d-payments | grep -i 'signature\|webhook\|401\|403'
    │   4. If signature mismatch → rotate and update TILLED_WEBHOOK_SECRET, restart payments
    │   5. Replay any UNKNOWN payment_attempts (follow UNKNOWN-RESOLUTION above)
    └── NO  → Outbound webhook delivery failure (7D Platform → tenants):
        1. Check AR/Payments outbox for stuck events:
           docker exec 7d-ar-postgres psql -U ar_user -d ar_db \
             -c "SELECT COUNT(*) FROM outbox WHERE published_at IS NULL;"
        2. Check NATS relay is consuming: nats consumer info PLATFORM --server localhost:4222
        3. If NATS relay stuck → restart affected service
        4. Check for DLQ buildup:
           docker exec 7d-ar-postgres psql -U ar_user -d ar_db \
             -c "SELECT subject, COUNT(*) FROM failed_events GROUP BY subject;"
        5. After root cause fixed → replay DLQ (see DLQ Replay section above)
```

---

## References

- `docs/ops/ALERT-THRESHOLDS.md` — full alert threshold definitions
- `docs/runbooks/disaster_recovery.md` — DR procedure
- `docs/runbooks/backup_restore.md` — backup and restore
- `docs/runbooks/projection_rebuild.md` — projection rebuild
- `scripts/production/rollback_stack.sh` — automated rollback script
- `scripts/production/smoke.sh` — post-recovery smoke test
- `scripts/production/log_bundle.sh` — capture diagnostic log bundle

## Changelog

- **2026-02-22**: Phase 48 — add Decision Trees for rollback and webhook failure scenarios; fix psql commands to use docker exec (bd-3len)
- **2026-02-19**: Phase 34 — initial incident response runbook (bd-x48w)
