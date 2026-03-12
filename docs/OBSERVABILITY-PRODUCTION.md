# Production Observability

This document covers the metrics, health checks, alerting, and log access for the 7D Solutions Platform in production. The goal is to detect outages and billing/payment failures quickly enough to act before customer impact.

## Observability Stack

| Component | Tool | Purpose |
|-----------|------|---------|
| Metrics | Prometheus | Scrapes `/metrics` from all services every 15 s |
| Dashboards | Grafana | Visualises billing health, payment unknowns, invariant failures |
| Alerts | Prometheus → Alertmanager | Pages on service down, billing stall, payment failure rate |
| Health checks | `/healthz` + `/api/ready` | Liveness and readiness probes (see `docs/HEALTH-CONTRACT.md`) |
| Logs | Docker log driver (`json-file`) | Accessed via `docker logs` or SSH on the VPS |
| Audit | `scripts/production/health_audit.sh` | Standalone DB + HTTP endpoint audit (run on VPS) |
| Smoke suite | `scripts/production/smoke.sh` | Full health + data check run via SSH from CI or laptop |

## Starting the Monitoring Stack

The monitoring stack is separate from the application stacks. Start it last:

```bash
# On the VPS (as deploy user):
export GRAFANA_ADMIN_PASSWORD="<secret>"
docker compose -f docker-compose.monitoring.yml up -d
```

The stack adds three containers to the `7d-platform` network:
- `7d-alertmanager` — receives alerts from Prometheus, routes and deduplicates, sends notifications
- `7d-prometheus` — scrapes all service `/metrics` endpoints, evaluates alert rules
- `7d-grafana` — Grafana UI (auto-provisions Prometheus datasource and dashboards)

All ports are bound to `127.0.0.1` only; UFW blocks external access. Use SSH tunnels to access:

```bash
# Prometheus UI
ssh -L 9091:localhost:9091 deploy@prod.7dsolutions.example.com
# → open http://localhost:9091

# Alertmanager UI
ssh -L 9094:localhost:9094 deploy@prod.7dsolutions.example.com
# → open http://localhost:9094

# Grafana UI
ssh -L 3002:localhost:3002 deploy@prod.7dsolutions.example.com
# → open http://localhost:3002
```

## Metrics Endpoints

Every service exposes Prometheus-format metrics at `/metrics`. Prometheus scrapes them every 15 seconds via the `7d-platform` Docker network:

| Service | Container | Port | Metrics path |
|---------|-----------|------|--------------|
| Identity Auth (×2) | `7d-auth-1`, `7d-auth-2` | 8080 | `/metrics` |
| Control Plane | `7d-control-plane` | 8091 | `/metrics` |
| AR | `7d-ar` | 8086 | `/metrics` |
| Payments | `7d-payments` | 8088 | `/metrics` |
| Subscriptions | `7d-subscriptions` | 8087 | `/metrics` |
| TTP | `7d-ttp` | 8100 | `/metrics` |
| GL | `7d-gl` | 8090 | `/metrics` |
| Notifications | `7d-notifications` | 8089 | `/metrics` |
| Inventory | `7d-inventory` | 8092 | `/metrics` |
| AP | `7d-ap` | 8093 | `/metrics` |
| Treasury | `7d-treasury` | 8094 | `/metrics` |
| Fixed Assets | `7d-fixed-assets` | 8104 | `/metrics` |
| Consolidation | `7d-consolidation` | 8105 | `/metrics` |
| Timekeeping | `7d-timekeeping` | 8097 | `/metrics` |
| Party | `7d-party` | 8098 | `/metrics` |
| Integrations | `7d-integrations` | 8099 | `/metrics` |
| Maintenance | `7d-maintenance` | 8101 | `/metrics` |
| PDF Editor | `7d-pdf-editor` | 8102 | `/metrics` |
| Shipping-Receiving | `7d-shipping-receiving` | 8103 | `/metrics` |
| NATS | `7d-nats` | 8222 | `/varz` |

Scrape configuration: `infra/monitoring/prometheus.yml`

## Health Check Endpoints

All services implement the health contract (`docs/HEALTH-CONTRACT.md`):

| Path | Purpose | Success | Failure |
|------|---------|---------|---------|
| `/healthz` | Liveness — is the process running? | HTTP 200 `{"status":"alive"}` | No response |
| `/api/ready` | Readiness — can it serve traffic? | HTTP 200 `{"status":"ready"}` | HTTP 503 `{"status":"down"}` |
| `/api/health` | Alias for `/api/ready` (some modules) | HTTP 200 | HTTP 503 |

Run the health audit to verify all services from the VPS:

```bash
# DB layer (checks postgres containers)
bash scripts/production/health_audit.sh

# HTTP layer (checks /healthz + /api/ready on localhost)
# (included automatically in the above; skip with SKIP_HTTP=true)

# Full smoke suite via SSH from off-host:
bash scripts/production/smoke.sh --host prod.7dsolutions.example.com
```

## Alert Rules

Alert rules are stored in `infra/monitoring/alerts/` and loaded by Prometheus automatically. Prometheus evaluates rules and sends firing alerts to Alertmanager, which handles grouping, deduplication, inhibition, and notification delivery.

| Rule file | Coverage |
|-----------|---------|
| `service-down.yml` | Service unreachable (billing spine: 1 min, platform: 2 min, modules: 5 min), NATS down |
| `payment-unknown.yml` | Payments stuck in UNKNOWN state >30 min (warning) / >1 h (critical) |
| `invariant-failure.yml` | GL / AR / Payments / Subscriptions invariant violations |
| `latency-slo.yml` | Per-endpoint HTTP latency SLOs and 5xx error rates (auth, AR, payments, TTP) |
| `outbox-health.yml` | Outbox backlog, DLQ growth, outbox insert failures |

> **SLO baseline re-sampling:** Thresholds in `latency-slo.yml` are pre-production baselines derived
> from k6 load testing and staging drills. After the first 72 h of real production traffic, re-sample
> p95/p99 using the PromQL queries in `docs/ALERT-THRESHOLDS.md §7` and update the alert rules.
> Repeat monthly (first Monday) and after any major release. See `docs/ALERT-THRESHOLDS.md` for the
> full re-sampling procedure.

## Alertmanager

Alertmanager receives alerts from Prometheus and routes them to notification channels. Configuration: `infra/monitoring/alertmanager.yml`.

### Routing

| Alert category | Group wait | Repeat interval | Rationale |
|---------------|------------|-----------------|-----------|
| Billing-spine critical (AR, Payments, Subscriptions, TTP) | 10 s | 5 min | Revenue-impacting — page immediately |
| Platform availability critical (auth, control-plane, NATS) | 10 s | 5 min | All services depend on these |
| Data integrity critical (invariant violations, outbox) | 10 s | 5 min | Financial data corruption risk |
| All warnings | 5 min | 4 h | Investigate promptly but don't page |
| Everything else | 30 s | 4 h | Default catch-all |

### Inhibition

- Critical alerts suppress warnings for the same alert name (avoids duplicate noise).
- `NATSDown` suppresses all event-delivery warnings (downstream effect of bus outage).

### Notification Channel — Slack

The default receiver is `ops-webhook`, configured to post alerts to a Slack channel (`#ops-alerts`). Before first deployment, set the real webhook URL:

1. Create a Slack app at https://api.slack.com/apps and enable Incoming Webhooks.
2. Add a webhook for the `#ops-alerts` channel (or your preferred channel).
3. Copy the webhook URL and replace the placeholder in `infra/monitoring/alertmanager.yml`:
   ```yaml
   api_url: 'https://hooks.slack.com/services/REPLACE/WITH/REAL-WEBHOOK-URL'
   ```
4. Reload Alertmanager: `curl -X POST http://localhost:9094/-/reload`

Alert messages include the alert name, severity, module, and summary annotation. Colors indicate status: red for critical firing, yellow for warning firing, green for resolved.

To switch to email or PagerDuty, replace `slack_configs` in the `receivers` section. See https://prometheus.io/docs/alerting/latest/configuration/

#### Testing Alert Delivery

Fire a synthetic alert to confirm Slack delivery:

```bash
# Post a test alert to Alertmanager
curl -X POST http://localhost:9094/api/v2/alerts \
  -H 'Content-Type: application/json' \
  -d '[{
    "labels": {
      "alertname": "TestAlert",
      "severity": "warning",
      "module": "test"
    },
    "annotations": {
      "summary": "This is a test alert — safe to ignore."
    },
    "startsAt": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
    "endsAt": "'$(date -u -v+5M +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d "+5 min" +%Y-%m-%dT%H:%M:%SZ)'"
  }]'
```

You should see the test alert appear in the `#ops-alerts` Slack channel within 30 seconds.

### Verifying Alertmanager

```bash
# Health check
curl -s http://localhost:9094/-/healthy

# View active alerts
curl -s http://localhost:9094/api/v2/alerts | jq .

# View silences
curl -s http://localhost:9094/api/v2/silences | jq .

# Reload config after editing alertmanager.yml
curl -X POST http://localhost:9094/-/reload
```

### Critical Alerts — Immediate Action Required

| Alert | Meaning | First response |
|-------|---------|---------------|
| `BillingServiceDown` | AR, Payments, Subscriptions, or TTP is down | Check `docker ps` + `docker logs <container>` on VPS |
| `PlatformServiceDown` | Auth or Control Plane is down | Check container status; review recent deploy for regression |
| `NATSDown` | NATS message bus unreachable | Check `7d-nats` container; event-driven workflows halted |
| `BillingCycleStalled` | No billing cycles completing for 30 min | Check Subscriptions logs; review AR for invoice failures |
| `PaymentProcessingStalled` | No payments succeeding for 15 min | Check Payments logs; verify gateway credentials and status |
| `PaymentUnknownDurationCritical` | Payment stuck in UNKNOWN > 1 h | Run UNKNOWN resolution runbook; check gateway webhook delivery |

### Warning Alerts — Investigate Promptly

| Alert | Meaning | First response |
|-------|---------|---------------|
| `ModuleServiceDown` | Non-billing module unreachable | Check container; assess customer-facing impact |
| `BillingCycleFailureRateCritical` | <50% of billing cycles completing | Investigate Subscriptions + AR logs |
| `PaymentFailureRateCritical` | >20% of payment attempts failing | Check gateway status; review recent code or config changes |
| `PaymentUnknownDurationWarning` | Payment stuck in UNKNOWN > 30 min | Investigate root cause before it hits critical |
| `*InvariantViolationWarning` | Any invariant violation detected | Freeze related deployments; investigate data integrity |

## Dashboards

Pre-built Grafana dashboards are in `infra/monitoring/grafana/dashboards/`. They auto-load when Grafana starts.

| Dashboard | Covers |
|-----------|-------|
| `billing-unknown.json` | Payment UNKNOWN duration and age distribution |
| `audit-integrity.json` | Invariant violation counts per module |
| `projection-lag.json` | Event consumer lag per service (NATS) |

Access dashboards via SSH tunnel to Grafana on port 3002.

## Logs

All services write JSON logs to Docker's `json-file` log driver. On the VPS:

```bash
# Tail logs for a service
docker logs -f 7d-payments
docker logs -f 7d-ar
docker logs -f 7d-subscriptions

# Show last 100 lines with timestamps
docker logs --since 1h --tail 100 7d-payments

# Search for errors
docker logs 7d-payments 2>&1 | grep -i error | tail -20

# All services simultaneously (requires docker compose)
docker compose logs -f --tail 50
```

Log level is controlled by the `RUST_LOG` environment variable in each service's env config. Default is `info`. Set to `debug` temporarily for deep investigation (redeploy required).

## Health Audit Script

`scripts/production/health_audit.sh` performs two checks:

1. **Database audit**: Connects to each Postgres container via `psql` and verifies it accepts queries.
2. **HTTP audit**: Curls `/healthz` and `/api/ready` on localhost for all critical services.

```bash
# Full audit (DB + HTTP)
bash scripts/production/health_audit.sh

# DB-only (skip HTTP)
SKIP_HTTP=true bash scripts/production/health_audit.sh

# Restore drill audit (checks ephemeral drill container only)
bash scripts/production/health_audit.sh --drill

# Point at a non-localhost host (e.g., staging)
HTTP_AUDIT_HOST=10.0.0.1 bash scripts/production/health_audit.sh
```

Exit code 0 = all reachable checks passed. Non-zero = one or more failures.

## First-Time Setup Checklist

After provisioning a new VPS:

1. Deploy the application stacks (data → platform → services → frontend).
2. Set `GRAFANA_ADMIN_PASSWORD` in the production secrets file.
3. Start the monitoring stack: `docker compose -f docker-compose.monitoring.yml up -d`
4. Verify Prometheus is scraping: SSH tunnel to port 9091 → Status → Targets.
5. Verify alert rules loaded: Prometheus → Alerts (all rules should be listed).
6. Run the health audit: `bash scripts/production/health_audit.sh`
7. Run the smoke suite: `bash scripts/production/smoke.sh --host <VPS> --dry-run` then without `--dry-run`.
8. Set the Slack webhook URL in `infra/monitoring/alertmanager.yml` (replace the placeholder `api_url`). See "Notification Channel — Slack" above.
9. Verify Alertmanager is healthy: `curl -s http://localhost:9094/-/healthy`
10. Verify Prometheus is connected: Prometheus → Status → Alertmanagers (should show `7d-alertmanager:9093`).
