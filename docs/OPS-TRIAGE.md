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
curl -sf http://localhost:8095/api/health && echo "fixed-assets OK"
curl -sf http://localhost:8096/api/health && echo "consolidation OK"
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
| Fixed Assets | `7d-fixed-assets` | 8095 |
| Consolidation | `7d-consolidation` | 8096 |
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
