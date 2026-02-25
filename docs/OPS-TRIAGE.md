# OPS-TRIAGE: Operator Triage Playbook

**Platform:** 7D Solutions
**Updated:** 2026-02-21 (Phase 45)

All commands run **on the VPS as the `deploy` user** unless noted. Services are
not exposed to the internet — all ports are bound to `127.0.0.1` and protected
by UFW. Use SSH tunnels for Grafana/Prometheus UI access.

---

## 1. Service Status

### Quick overview — all containers

```bash
docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
```

### Identify unhealthy containers

```bash
docker ps --filter 'health=unhealthy' --format '{{.Names}}\t{{.Status}}'
docker ps --filter 'status=exited' --format '{{.Names}}\t{{.Status}}'
```

### Health check per service (HTTP layer)

```bash
# Billing spine — check first
curl -sf http://localhost:8087/api/health && echo "subscriptions OK" || echo "subscriptions DOWN"
curl -sf http://localhost:8086/api/health && echo "ar OK"            || echo "ar DOWN"
curl -sf http://localhost:8088/api/health && echo "payments OK"      || echo "payments DOWN"
curl -sf http://localhost:8100/api/health && echo "ttp OK"           || echo "ttp DOWN"

# Platform
curl -sf http://localhost:8080/healthz    && echo "auth OK"          || echo "auth DOWN"
curl -sf http://localhost:8091/api/ready  && echo "control-plane OK" || echo "control-plane DOWN"

# Supporting modules
curl -sf http://localhost:8090/api/health && echo "gl OK"
curl -sf http://localhost:8089/api/health && echo "notifications OK"
curl -sf http://localhost:8092/api/health && echo "inventory OK"
curl -sf http://localhost:8093/api/health && echo "ap OK"
curl -sf http://localhost:8094/api/health && echo "treasury OK"
curl -sf http://localhost:8104/api/health && echo "fixed-assets OK"
curl -sf http://localhost:8105/api/health && echo "consolidation OK"
curl -sf http://localhost:8097/api/health && echo "timekeeping OK"
curl -sf http://localhost:8098/api/health && echo "party OK"
curl -sf http://localhost:8099/api/health && echo "integrations OK"
```

### Full automated audit (DB + HTTP)

```bash
bash /opt/7d-platform/scripts/production/health_audit.sh
```

### Service port reference

| Service | Container | Port |
|---------|-----------|------|
| Identity Auth (×2) | `7d-auth-1`, `7d-auth-2` | 8080 |
| Control Plane | `7d-control-plane` | 8091 |
| AR | `7d-ar` | 8086 |
| Payments | `7d-payments` | 8088 |
| Subscriptions | `7d-subscriptions` | 8087 |
| TTP | `7d-ttp` | 8100 |
| GL | `7d-gl` | 8090 |
| Notifications | `7d-notifications` | 8089 |
| Inventory | `7d-inventory` | 8092 |
| AP | `7d-ap` | 8093 |
| Treasury | `7d-treasury` | 8094 |
| Fixed Assets | `7d-fixed-assets` | 8104 |
| Consolidation | `7d-consolidation` | 8105 |
| Timekeeping | `7d-timekeeping` | 8097 |
| Party | `7d-party` | 8098 |
| Integrations | `7d-integrations` | 8099 |
| NATS | `7d-nats` | 4222 (client), 8222 (monitoring) |
| Prometheus | `7d-prometheus` | 9090 |
| Grafana | `7d-grafana` | 3001 |

---

## 2. Tail Logs

### Single service — follow

```bash
docker logs -f 7d-payments
docker logs -f 7d-ar
docker logs -f 7d-subscriptions
docker logs -f 7d-ttp
docker logs -f 7d-auth-1
docker logs -f 7d-control-plane
```

### Last N lines with timestamps

```bash
# Last 200 lines for payments (most common first-look)
docker logs --timestamps --tail 200 7d-payments

# Last hour of logs for a service
docker logs --since 1h 7d-ar
docker logs --since 30m 7d-subscriptions

# Specific time window (ISO 8601)
docker logs --since "2026-02-21T14:00:00" --until "2026-02-21T14:30:00" 7d-payments
```

### All services simultaneously

```bash
cd /opt/7d-platform
docker compose -f docker-compose.platform.yml \
               -f docker-compose.modules.yml \
               -f docker-compose.data.yml \
               logs -f --tail 30 --timestamps
```

### Stream logs for a specific service window

```bash
# Capture 5 minutes of logs from payments to file
docker logs --since 5m --timestamps 7d-payments > /tmp/payments-$(date +%Y%m%d-%H%M%S).log
```

---

## 3. Find Error Spikes

### Count errors in the last hour (per service)

```bash
# Count ERROR lines in the last hour for billing spine
for svc in 7d-payments 7d-ar 7d-subscriptions 7d-ttp; do
  count=$(docker logs --since 1h "$svc" 2>&1 | grep -ic 'error\|panic\|FATAL' || true)
  echo "$svc: $count errors"
done
```

### Find the most recent errors

```bash
# Payments — last 10 errors
docker logs --since 2h 7d-payments 2>&1 | grep -i 'error\|panic' | tail -10

# AR — look for invoice failures
docker logs --since 2h 7d-ar 2>&1 | grep -i 'error\|failed\|UNKNOWN' | tail -20

# Subscriptions — billing cycle failures
docker logs --since 2h 7d-subscriptions 2>&1 | grep -i 'error\|cycle\|failed' | tail -20
```

### Correlate errors across services by timestamp

```bash
# Pull last 30 min from billing spine, sort by time
{
  docker logs --since 30m --timestamps 7d-payments 2>&1 | sed 's/^/[payments] /'
  docker logs --since 30m --timestamps 7d-ar        2>&1 | sed 's/^/[ar] /'
  docker logs --since 30m --timestamps 7d-subscriptions 2>&1 | sed 's/^/[subscriptions] /'
} | sort | grep -i 'error\|UNKNOWN\|failed'
```

### Prometheus — check metric spikes (via SSH tunnel)

```bash
# From your local machine, open Grafana tunnel:
ssh -L 3001:localhost:3001 deploy@prod.7dsolutions.example.com
# Then open http://localhost:3001 in your browser.

# Or query Prometheus directly:
ssh -L 9090:localhost:9090 deploy@prod.7dsolutions.example.com
# Then open http://localhost:9090
```

Useful PromQL queries:
- `sum(rate(payment_attempts_total{status="failed"}[5m]))` — payment failure rate
- `unknown_invoice_age_seconds` — age of oldest UNKNOWN invoice
- `invariant_violations_total` — GL/AR integrity violations

---

## 4. Verify Webhook and Billing Failures

### Payment webhook delivery

Webhooks from the payment gateway (Tilled) arrive at `7d-payments` on
`POST /webhooks/tilled`. Check:

```bash
# Recent webhook requests received
docker logs --since 1h 7d-payments 2>&1 | grep -i 'webhook\|tilled'

# Webhook signature failures (invalid secret or replay)
docker logs --since 1h 7d-payments 2>&1 | grep -i 'signature\|invalid\|unauthorized' | tail -20

# UNKNOWN payment attempts older than 30 minutes
docker exec 7d-payments-postgres psql \
  -U payments_user -d payments_db \
  -c "SELECT id, created_at, now() - updated_at AS age
      FROM payment_attempts
      WHERE status = 'unknown'
      ORDER BY updated_at
      LIMIT 20;"
```

> **Note:** `PGPASSWORD` is not needed when using `docker exec` against the
> postgres container directly — the container's `pg_hba.conf` trusts local
> connections from `payments_user`. Credentials are sourced from
> `/etc/7d/production/secrets.env`.

### Billing cycle stall

```bash
# Subscriptions: check for active subscriptions with no recent billing event
docker exec 7d-subscriptions-postgres psql \
  -U subscriptions_user -d subscriptions_db \
  -c "SELECT id, tenant_id, status, current_period_end
      FROM subscriptions
      WHERE status = 'active'
        AND current_period_end < now()
      ORDER BY current_period_end
      LIMIT 10;"

# AR: invoices stuck in UNKNOWN
docker exec 7d-ar-postgres psql \
  -U ar_user -d ar_db \
  -c "SELECT id, tenant_id, created_at, now() - updated_at AS age
      FROM invoices
      WHERE status = 'unknown'
      ORDER BY updated_at
      LIMIT 20;"

# AR: invoices with no corresponding payment attempt (billing gap)
docker exec 7d-ar-postgres psql \
  -U ar_user -d ar_db \
  -c "SELECT id, tenant_id, total_amount, status, created_at
      FROM invoices
      WHERE status IN ('open','past_due')
        AND created_at < now() - interval '2 hours'
      ORDER BY created_at
      LIMIT 20;"
```

### NATS delivery lag (outbox relay)

```bash
# List all NATS streams and consumer lag
nats stream list --server localhost:4222
nats consumer list PLATFORM --server localhost:4222

# Inspect a specific stream for pending messages
nats stream info PLATFORM --server localhost:4222

# Check specific consumer lag (e.g. payments → AR)
nats consumer info PLATFORM payments.completed --server localhost:4222
```

### Outbox table — pending/failed events

```bash
# Payments outbox: events not yet published to NATS
docker exec 7d-payments-postgres psql \
  -U payments_user -d payments_db \
  -c "SELECT aggregate_type, event_type, created_at, published_at
      FROM outbox
      WHERE published_at IS NULL
      ORDER BY created_at
      LIMIT 20;"

# AR outbox
docker exec 7d-ar-postgres psql \
  -U ar_user -d ar_db \
  -c "SELECT aggregate_type, event_type, created_at, published_at
      FROM outbox
      WHERE published_at IS NULL
      ORDER BY created_at
      LIMIT 20;"
```

If outbox events are stuck, restart the affected service to resume the relay:

```bash
cd /opt/7d-platform
docker compose -f docker-compose.modules.yml restart 7d-payments
docker compose -f docker-compose.modules.yml restart 7d-ar
```

---

## 5. Verify DB and NATS Connectivity

### Postgres — connection test (all databases)

```bash
# Run the full DB audit (fastest)
bash /opt/7d-platform/scripts/production/health_audit.sh

# Or test a single database manually
docker exec 7d-ar-postgres psql -U ar_user -d ar_db -c "SELECT 1;"
docker exec 7d-payments-postgres psql -U payments_user -d payments_db -c "SELECT 1;"
docker exec 7d-subscriptions-postgres psql -U subscriptions_user -d subscriptions_db -c "SELECT 1;"
docker exec 7d-gl-postgres psql -U gl_user -d gl_db -c "SELECT 1;"
```

### Postgres — check replication lag (if applicable)

```bash
# On the primary: check if replicas are connected
docker exec 7d-auth-postgres psql -U auth_user -d auth_db \
  -c "SELECT client_addr, state, sent_lsn, replay_lsn FROM pg_stat_replication;"
```

### NATS — connectivity and cluster health

```bash
# Test connection
nats server check connection --server localhost:4222

# Server info and JetStream status
nats server info --server localhost:4222

# Stream list and message counts
nats stream list --server localhost:4222

# Consumer lag summary
nats consumer report PLATFORM --server localhost:4222
```

### NATS monitoring endpoint (JSON)

```bash
# Server variables (uptime, connections, memory)
curl -sf http://localhost:8222/varz | python3 -m json.tool | grep -E 'uptime|connections|mem'

# JetStream info
curl -sf http://localhost:8222/jsz | python3 -m json.tool | head -40

# Stream list
curl -sf http://localhost:8222/jsz?streams=true | python3 -m json.tool
```

---

## 6. Capture a Log Bundle

Use `log_bundle.sh` to capture a timestamped diagnostic bundle without leaking
secrets. Hand the bundle to engineering for async analysis.

```bash
# Capture the last hour from all services
bash /opt/7d-platform/scripts/production/log_bundle.sh

# Capture the last 4 hours
bash /opt/7d-platform/scripts/production/log_bundle.sh --since 4h

# Capture only billing spine services
bash /opt/7d-platform/scripts/production/log_bundle.sh \
  --services "7d-payments,7d-ar,7d-subscriptions,7d-ttp"

# Capture a specific time window
bash /opt/7d-platform/scripts/production/log_bundle.sh \
  --since "2026-02-21T14:00:00" --until "2026-02-21T14:30:00"
```

The bundle is written to `/tmp/7d-log-bundle-YYYYMMDD-HHMMSS.tar.gz`.
It contains only log output — **no environment files, no secrets**.

---

## 7. Emergency Restart Procedures

### Restart a single service

```bash
cd /opt/7d-platform
docker compose -f docker-compose.modules.yml restart 7d-payments
docker compose -f docker-compose.platform.yml restart 7d-control-plane
docker compose -f docker-compose.infrastructure.yml restart 7d-nats
```

### Restart the billing spine (in order)

```bash
cd /opt/7d-platform
# Stop in reverse dependency order
docker compose -f docker-compose.modules.yml stop 7d-subscriptions 7d-ar 7d-payments 7d-ttp
# Start in dependency order
docker compose -f docker-compose.modules.yml start 7d-ttp 7d-payments 7d-ar 7d-subscriptions
```

### Full stack rollback

```bash
bash /opt/7d-platform/scripts/production/rollback_stack.sh
```

---

## 8. Related Documents

| Document | Purpose |
|----------|---------|
| `docs/OBSERVABILITY-PRODUCTION.md` | Metrics, alerts, dashboards, log access |
| `docs/HEALTH-CONTRACT.md` | Liveness/readiness health check spec |
| `docs/runbooks/incident_response.md` | Severity classification, alert response matrix |
| `docs/runbooks/BACKUP-RESTORE-RUNBOOK.md` | Database backup and restore procedures |
| `docs/RESTORE-DRILL.md` | Restore drill procedures |
| `scripts/production/health_audit.sh` | Automated DB + HTTP audit |
| `scripts/production/smoke.sh` | Full smoke suite (off-host) |
| `scripts/production/log_bundle.sh` | Capture diagnostic log bundle |

---

## 9. Alert Runbooks

> These sections are the authoritative runbooks referenced by Prometheus alert
> `runbook_url` annotations in `infra/monitoring/alerts/`. Each heading maps
> directly to an alert group.

### SERVICE-DOWN

**Alerts:** `BillingServiceDown`, `PlatformServiceDown`, `ModuleServiceDown`

1. Open `infra/monitoring/grafana/dashboards/service-health.json` in Grafana — identify which service shows DOWN.
2. Check container status: `docker ps --filter 'status=exited' --format '{{.Names}}\t{{.Status}}'`
3. Tail logs for the affected service: `docker logs --tail 200 7d-<service>`
4. Attempt restart: `docker compose -f docker-compose.modules.yml restart 7d-<service>`
5. If restart fails, check DB connectivity (section 5 above) and NATS status.
6. Escalate if service does not recover within 5 minutes of restart.

### NATS-DOWN

**Alert:** `NATSDown`

1. Check NATS container: `docker ps | grep nats`
2. Tail NATS logs: `docker logs --tail 200 7d-nats`
3. Restart NATS: `docker compose -f docker-compose.infrastructure.yml restart 7d-nats`
4. Verify outbox relay resumes: watch `outbox_pending_events` metric drop in Grafana.
5. After recovery, verify no events were lost — check outbox tables for stranded rows.

### BILLING-CYCLE-FAILURE

**Alerts:** `BillingCycleFailureRateCritical`, `BillingCycleStalled`

1. Open `infra/monitoring/grafana/dashboards/billing-runs.json` in Grafana.
2. Check completion rate panel — if 0%, subscriptions service may be stuck.
3. Inspect subscriptions logs: `docker logs --tail 500 7d-subscriptions | grep -i 'error\|cycle\|billing'`
4. Check for DB lock contention on the subscriptions database.
5. Verify NATS is healthy — billing cycles emit events that trigger downstream work.
6. If stalled: restart subscriptions service, then verify cycles resume within 5 minutes.

### PAYMENT-FAILURE-RATE

**Alert:** `PaymentFailureRateCritical`

1. Open `infra/monitoring/grafana/dashboards/webhook-failures.json` — check Payments webhook failures.
2. Check payment gateway status (Tilled dashboard / status page).
3. Inspect payments logs: `docker logs --tail 500 7d-payments | grep -i 'error\|fail\|gateway'`
4. Verify payment credentials are not expired: check `.env` payment API key expiry.
5. If gateway is healthy but failures persist, check payments DB for stuck records.

### PAYMENT-PROCESSING-STALLED

**Alert:** `PaymentProcessingStalled`

1. Confirm payment attempts are being made but nothing succeeds (Grafana billing-runs dashboard).
2. Check payment gateway connectivity from the container:
   `docker exec 7d-payments curl -sf https://api.tilled.com/health`
3. Inspect payments logs for gateway timeout or credential errors.
4. Rotate payment credentials if expired; restart payments service after update.

### UNKNOWN-RESOLUTION

**Alerts:** `PaymentUnknownDurationWarning`, `PaymentUnknownDurationCritical`

1. Query payments DB for stuck UNKNOWN records:
   `psql $PAYMENTS_DB_URL -c "SELECT id, tenant_id, created_at FROM payments WHERE status='UNKNOWN' ORDER BY created_at"`
2. For each UNKNOWN payment, check gateway status via Tilled API.
3. If gateway confirms success → manually transition to SUCCEEDED via admin API.
4. If gateway confirms failure → manually transition to FAILED and trigger retry.
5. If gateway is unreachable → wait for gateway recovery, then re-query.

### PAYMENT-SYSTEMIC-FAILURE

**Alert:** `PaymentUnknownSystemicCritical`

Treat as critical incident. More than 10 payments stuck in UNKNOWN indicates gateway or network failure.

1. Page on-call. Freeze non-critical deployments.
2. Follow `UNKNOWN-RESOLUTION` steps above in bulk.
3. Review `docs/runbooks/incident_response.md` for P0 severity escalation path.

### INVARIANT-INVESTIGATION

**Alerts:** `GLInvariantViolationCritical`, `ARInvariantViolationCritical`, etc.

1. Open `infra/monitoring/grafana/dashboards/audit-integrity.json` in Grafana.
2. Identify the violation type and affected tenant from the alert label.
3. **Freeze related deployments immediately.**
4. Query the affected module's DB for the violating records.
5. Review recent commits for logic changes touching the violated invariant.
6. If data corruption is confirmed, follow `docs/runbooks/incident_response.md` rollback procedure.

### DUPLICATE-INVOICE-RESOLUTION

**Alert:** `DuplicateInvoicesPerCycleCritical`

1. Query subscriptions DB: `SELECT subscription_id, billing_period, COUNT(*) FROM invoices GROUP BY subscription_id, billing_period HAVING COUNT(*) > 1`
2. Identify which billing cycle triggered duplicate creation.
3. Mark duplicate invoices as VOID via admin API (do not delete — preserve audit trail).
4. File bug with subscription billing cycle idempotency key implementation.

### LIFECYCLE-INTEGRITY

**Alert:** `RetroactiveStateChangesCritical`

1. Identify which invoice had a retroactive state change from the alert label.
2. Query AR DB for the invoice state history.
3. If caused by a code bug, roll back the deployment and restore the correct state from audit log.
4. If caused by a manual operation, document the deviation in the incident record.

### REFERENTIAL-INTEGRITY

**Alert:** `OrphanedFinalizationAttemptsCritical`

1. Query AR DB for finalization attempts without parent invoice:
   `SELECT * FROM finalization_attempts fa WHERE NOT EXISTS (SELECT 1 FROM invoices i WHERE i.id = fa.invoice_id)`
2. This indicates a DB-level bug or a failed transaction that wrote partial state.
3. Treat as P0. Freeze billing operations. Engage DB restore if data loss is confirmed.

### ERROR-SPIKE

For HTTP 5xx spikes observed in `infra/monitoring/grafana/dashboards/error-rates.json`:

1. Identify the service with elevated 5xx rate from the Grafana panel.
2. Tail service logs for error details: `docker logs --tail 500 7d-<service> | grep -c 'ERROR\|500'`
3. Correlate with recent deploys: `git log --oneline -10`
4. If spike follows a deploy, initiate rollback via `docs/OPS-TRIAGE.md#7-emergency-restart-procedures`.

### WEBHOOK-FAILURE

For webhook failures observed in `infra/monitoring/grafana/dashboards/webhook-failures.json`:

1. Check the failing webhook endpoint (AR or Payments) for reachability.
2. Inspect webhook delivery logs: `docker logs 7d-ar | grep -i webhook` / `docker logs 7d-payments | grep -i webhook`
3. Verify the target endpoint URL is still valid (tenant webhook URL config).
4. For exhausted retries: manually trigger re-delivery via admin API if the target is now healthy.

### DB-NATS

For DB/NATS issues observed in `infra/monitoring/grafana/dashboards/db-nats-status.json`:

1. **NATS down:** follow `NATS-DOWN` runbook above.
2. **DB connection exhaustion:** identify which service is exhausting its pool; check for slow queries or connection leaks via `SHOW PROCESSLIST` / `pg_stat_activity`.
3. **Outbox lag:** if pending event count is rising, check NATS status first, then check relay consumer logs.
