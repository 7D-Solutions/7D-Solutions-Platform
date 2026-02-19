# On-Call Support Checklist

**Phase 34 — Hardening / Launch Readiness**

## Purpose

Structured checklist for on-call engineers to start and end a shift, perform
daily platform health checks, and hand off to the next engineer.

---

## Shift Start Checklist

Run at the start of every on-call shift (or after waking from a page).

### 1. Verify service health

```bash
# Platform (auth load balancer)
curl -sf http://localhost:8080/api/health && echo " OK auth" || echo " FAIL auth"

# All modules
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

All lines should print `OK`. Any `FAIL` → P1 (see [incident_response.md](incident_response.md)).

### 2. Check NATS health

```bash
# Server connectivity
nats server check connection --server localhost:4222

# Consumer lag (look for AckPending > 1000)
nats consumer list PLATFORM --server localhost:4222
```

### 3. Check for UNKNOWN entities

```bash
# AR — invoices
PGPASSWORD=ar_pass psql -h localhost -p 5434 -U ar_user -d ar_db -c "
  SELECT COUNT(*) AS unknown_invoices,
         MAX(now() - updated_at) AS oldest
  FROM invoices WHERE status = 'unknown';
"

# Payments
PGPASSWORD=payments_pass psql -h localhost -p 5436 -U payments_user -d payments_db -c "
  SELECT COUNT(*) AS unknown_payments,
         MAX(now() - updated_at) AS oldest
  FROM payment_attempts WHERE status = 'unknown';
"
```

Anything > 1 hour old → investigate (see [incident_response.md](incident_response.md#unknown-protocol-resolution)).

### 4. Verify GL balance

```bash
PGPASSWORD=gl_pass psql -h localhost -p 5438 -U gl_user -d gl_db -c "
  SELECT SUM(debit_cents) - SUM(credit_cents) AS gl_imbalance
  FROM journal_entries;
"
```

Result MUST be `0`. Non-zero → P1 immediately.

### 5. Check backup freshness

```bash
# Most recent backup directory
ls -1td backups/*/ | head -3

# Check manifest timestamp
LAST_BACKUP="$(ls -1td backups/*/ | head -1)"
python3 -c "
import json, datetime
m = json.load(open('${LAST_BACKUP}/manifest.json'))
ts = datetime.datetime.fromisoformat(m['timestamp'].replace('Z', '+00:00'))
age = datetime.datetime.now(datetime.timezone.utc) - ts
print(f'Last backup: {m[\"timestamp\"]}  ({age.total_seconds()/3600:.1f} h ago)')
print(f'Errors: {m[\"errors\"]}')
"
```

Backups older than 25 h → investigate cron / run manual backup.

### 6. Check DLQ counts

```bash
nats stream info PLATFORM --server localhost:4222 | grep -E "DLQ|Messages"
```

Any DLQ count > 0 → review before assuming safe (see [incident_response.md](incident_response.md#dlq-replay)).

### 7. Check Docker container status

```bash
docker compose -f docker-compose.infrastructure.yml ps
docker compose -f docker-compose.modules.yml ps
# All should show "healthy" or "running"
```

---

## Daily Operations Tasks

| Task | Frequency | Command |
|------|-----------|---------|
| Review backup log | Daily | `cat "$(ls -1td backups/*/ | head -1)/backup.log" | tail -20` |
| Prune old local backups | Weekly | `find ./backups -maxdepth 1 -type d -mtime +30 -exec rm -rf {} +` |
| Review alert thresholds | Quarterly | `docs/ops/ALERT-THRESHOLDS.md` |
| Run DR drill | Quarterly | `bash scripts/dr_drill.sh` |
| Check projection lag | Daily (if concerns) | `nats consumer list PLATFORM` |

---

## Responding to a Page

When paged, work through this decision tree:

```
1. Which service/alert?
   → Check alert name against docs/ops/ALERT-THRESHOLDS.md

2. Is GL impacted?
   → Yes → P1: freeze financial reporting, escalate immediately
   → No  → Continue

3. Is a module service down?
   → Yes → Restart service, check logs
   → No  → Check DLQ or UNKNOWN entity backlog

4. Can I resolve within SLA?
   → No  → Escalate per severity table in incident_response.md
```

See [incident_response.md](incident_response.md) for specific procedures.

---

## Escalation Contacts

| Role | Contact | When |
|------|---------|------|
| On-call backup | Check rotation schedule | If primary is blocked > 15 min |
| Engineering lead | @PearlOwl | P1 incidents, all financial data risks |
| Orchestrator agent | BrightHill (agent mail) | Coordination, bead creation for systemic issues |

**Agent mail escalation:**
```bash
./scripts/agent-mail-helper.sh send BrightHill "Incident" \
  "P1 incident in progress: <description>. Need support bead created."
```

---

## Shift Handoff

Before going off-call, send a handoff note:

```
HANDOFF — {date} {shift end time}

Service health:   All OK / Issues: {list}
Open incidents:   {none / P1 ticket #xxx — status}
DLQ state:        Clean / {count} events pending resolution
Last backup:      {timestamp} — OK / FAILED
GL balance:       0 (OK) / IMBALANCED — see incident #xxx
Projection lag:   Nominal / {module} at {seconds}s lag
Follow-up needed: {none / description}
```

---

## Useful One-Liners

```bash
# All service health at once
for p in 8080 8086 8087 8088 8089 8090 8092 8093 8094 8095 8096 8097 8098 8099 8100; do
  curl -sf "http://localhost:${p}/api/health" && echo " :${p}" || echo " FAIL :${p}"
done

# Run a fresh backup immediately
bash scripts/backup_all.sh

# DR drill (dry run — no changes)
bash scripts/dr_drill.sh --dry-run

# Check all databases are reachable
for entry in \
  "auth_db:5433:auth_user:auth_pass" \
  "ar_db:5434:ar_user:ar_pass" \
  "gl_db:5438:gl_user:gl_pass" \
  "tenant_registry_db:5441:tenant_registry_user:tenant_registry_pass"; do
  db="${entry%%:*}"; rest="${entry#*:}"; port="${rest%%:*}"; rest="${rest#*:}"
  user="${rest%%:*}"; pass="${rest#*:}"
  PGPASSWORD="${pass}" pg_isready -h localhost -p "${port}" -U "${user}" -d "${db}" \
    && echo " OK ${db}" || echo " FAIL ${db}"
done

# Tail all module logs for errors
docker compose -f docker-compose.modules.yml logs --follow --no-log-prefix \
  | grep -E "ERROR|PANIC|invariant|unknown"
```

---

## References

- `docs/runbooks/incident_response.md` — incident procedures
- `docs/runbooks/disaster_recovery.md` — DR procedure
- `docs/runbooks/backup_restore.md` — backup/restore detail
- `docs/runbooks/projection_rebuild.md` — projection rebuild
- `docs/ops/ALERT-THRESHOLDS.md` — alert threshold definitions
- `scripts/backup_all.sh` — backup script
- `scripts/restore_all.sh` — restore script
- `scripts/dr_drill.sh` — quarterly DR drill

## Changelog

- **2026-02-19**: Phase 34 — initial support checklist (bd-x48w)
