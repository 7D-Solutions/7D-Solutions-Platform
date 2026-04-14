# Platform Operations Runbook

**Bead:** bd-3dptf (GAP-19)
**SLO definitions:** `ops/slo.yaml`
**Alert rules:** `ops/alerts/`
**Dashboard:** `ops/grafana/dashboards/platform-overview.json`

This runbook links every Prometheus alert to a probable cause, immediate action, escalation path, and owner. Keep it open when an alert fires.

---

## Alert Index

| Alert | Module | Severity | Routing |
|-------|--------|----------|---------|
| [*_AvailabilitySLOFastBurn](#module-availability-slo-fast-burn) | AP/AR/GL/Production/Inventory | critical | PagerDuty |
| [*_AvailabilitySLOSlowBurn](#module-availability-slo-slow-burn) | AP/AR/GL/Production/Inventory | warning | Slack |
| [*_ReadLatencySLOFastBurn](#module-latency-slo) | AP/AR/GL/Production/Inventory | critical | PagerDuty |
| [*_WriteLatencySLOFastBurn](#module-latency-slo) | AP/AR/GL/Production/Inventory | critical | PagerDuty |
| [*_HealthCheckFailing](#module-health-check-failing) | AP/AR/GL/Production/Inventory | critical | PagerDuty |

---

## Module Availability SLO Fast Burn

**Alert names:** `AP_AvailabilitySLOFastBurn`, `AR_AvailabilitySLOFastBurn`, `GL_AvailabilitySLOFastBurn`, `Production_AvailabilitySLOFastBurn`, `Inventory_AvailabilitySLOFastBurn`

**Trigger:** Error rate > 3.6% over 1 hour (36× the 0.1% SLO budget — 5% of monthly budget at risk).

**Owner:** Platform on-call engineer (PagerDuty)

### Probable Causes

1. Database connection pool exhausted — module cannot acquire a connection
2. Downstream service unreachable (e.g. BOM, Numbering, Audit)
3. Invalid migration or schema change causing widespread query failures
4. Memory pressure causing OOM kills (container restart loop)
5. NATS connectivity lost — event publishing failing on every write

### Immediate Actions

1. Check the module's recent error logs:
   ```bash
   docker compose logs --tail 200 7d-<module> 2>&1 | grep -E "ERROR|FATAL|panic"
   ```
2. Check health endpoint:
   ```bash
   curl -sf http://localhost:<port>/health | jq .
   ```
   Port map: AP=8093, AR=8086, GL=8090, Production=8108, Inventory=8092
3. Check database connectivity from within the container:
   ```bash
   docker exec 7d-<module> sh -c 'pg_isready -h $DB_HOST -U $DB_USER'
   ```
4. Check error rate in Grafana: `http://grafana:3000/d/platform-overview`
5. If database connection pool exhausted — restart the module container (it resets pool):
   ```bash
   AGENTCORE_WATCHER_OVERRIDE=1 docker restart 7d-<module>
   ```

### Escalation Path

- **< 5 min:** On-call engineer investigates using steps above
- **5–15 min:** Escalate to platform lead if root cause not identified
- **> 15 min:** Escalate to CTO; consider module rollback via git revert + commit

---

## Module Availability SLO Slow Burn

**Alert names:** `AP_AvailabilitySLOSlowBurn`, `AR_AvailabilitySLOSlowBurn`, `GL_AvailabilitySLOSlowBurn`, `Production_AvailabilitySLOSlowBurn`, `Inventory_AvailabilitySLOSlowBurn`

**Trigger:** Error rate > 1.2% over 6 hours (12× the SLO budget — 10% of monthly budget at risk).

**Owner:** Platform engineer via Slack (#platform-alerts)

### Probable Causes

1. Intermittent downstream dependency failures (occasional timeouts)
2. Background job or cron causing periodic errors
3. Single-route regression introduced in recent deploy

### Immediate Actions

1. Identify the failing route:
   ```promql
   sum by (route) (rate(<module>_http_requests_total{status=~"5.."}[30m]))
   / sum by (route) (rate(<module>_http_requests_total[30m]))
   ```
2. Check recent commits on main branch for the affected module
3. Review error logs for specific error patterns
4. If a single route is causing all errors, consider feature-flagging or routing around it

### Escalation Path

- Monitor for 30 min; if burn rate increases, treat as fast burn
- Create a bead to fix the root cause before the monthly budget depletes

---

## Module Latency SLO

**Alert names:** `*_ReadLatencySLOFastBurn`, `*_WriteLatencySLOFastBurn`

**Trigger:**
- Read p95 latency > 500ms sustained for 5 minutes
- Write p95 latency > 1000ms sustained for 5 minutes

**Owner:** Platform on-call engineer (PagerDuty)

### Probable Causes

1. Database slow queries — missing index, table bloat, or lock contention
2. Connection pool exhaustion causing queued requests
3. Downstream service (BOM, Numbering) adding latency to write path
4. Container CPU throttling under load

### Immediate Actions

1. Check slow query log in the module's Postgres instance:
   ```bash
   docker exec 7d-<module>-postgres psql -U <user> -d <db> \
     -c "SELECT query, mean_exec_time, calls FROM pg_stat_statements ORDER BY mean_exec_time DESC LIMIT 10;"
   ```
2. Check connection pool wait time:
   ```promql
   histogram_quantile(0.95, rate(<module>_http_request_duration_seconds_bucket[5m]))
   ```
   If latency is uniform across routes → pool or DB issue. If single route → query issue.
3. Check for lock contention:
   ```bash
   docker exec 7d-<module>-postgres psql -U <user> -d <db> \
     -c "SELECT pid, query, state, wait_event_type, wait_event FROM pg_stat_activity WHERE wait_event IS NOT NULL;"
   ```
4. If CPU throttling: check `docker stats 7d-<module>` for CPU %

### Escalation Path

- **5 min:** Identify whether DB or upstream is the bottleneck
- **15 min:** Escalate to platform lead for database analysis
- **30 min:** Consider increasing connection pool size or scaling the module

---

## Module Health Check Failing

**Alert names:** `AP_HealthCheckFailing`, `AR_HealthCheckFailing`, `GL_HealthCheckFailing`, `Production_HealthCheckFailing`, `Inventory_HealthCheckFailing`

**Trigger:** Prometheus `up{job="<module>"}` == 0 for 5 minutes (health endpoint not responding).

**Owner:** Platform on-call engineer (PagerDuty)

### Probable Causes

1. Container has crashed (OOM, panic, or failed startup)
2. Container is running but health endpoint is deadlocked
3. Binary failed to start — compilation or migration error at boot
4. Port conflict or network issue

### Immediate Actions

1. Check container status:
   ```bash
   docker ps | grep 7d-<module>
   docker inspect 7d-<module> | jq '.[0].State'
   ```
2. Check container logs for crash reason:
   ```bash
   docker compose logs --tail 100 7d-<module> 2>&1
   ```
3. Check if the binary is the correct version:
   ```bash
   sha256sum /path/to/target/<module>
   ```
4. If container is stopped: this is a Severity 1 incident. Notify on-call immediately.
5. DO NOT run `docker compose up/down` — the cross-watcher manages container lifecycle.
   Use: `AGENTCORE_WATCHER_OVERRIDE=1 docker restart 7d-<module>`

### Escalation Path

- **Immediate:** Page on-call via PagerDuty
- **5 min:** If container is not self-recovering, escalate to platform lead
- **10 min:** Engage CTO if health is not restored; prepare customer communication if AP/AR/GL affected

---

## AP-Specific Alerts

### AP Availability SLO Fast Burn {#ap-availability-slo-fast-burn}

See [Module Availability SLO Fast Burn](#module-availability-slo-fast-burn).

AP-specific additional checks:
- Verify payment provider connectivity (payment_runs may be triggering external calls)
- Check `ap_outbox_queue_depth` — if rising, event publishing may be blocking

### AP Latency SLO {#ap-latency-slo}

See [Module Latency SLO](#module-latency-slo).

AP-specific: bill approval and payment-run creation are the heaviest write paths. Check those first.

---

## AR-Specific Alerts

### AR Availability SLO Fast Burn {#ar-availability-slo-fast-burn}

See [Module Availability SLO Fast Burn](#module-availability-slo-fast-burn).

AR-specific: invoice finalization triggers subscription billing integration. Check subscription module if AR errors spike.

### AR Latency SLO {#ar-latency-slo}

See [Module Latency SLO](#module-latency-slo).

---

## GL-Specific Alerts

### GL Availability SLO Fast Burn {#gl-availability-slo-fast-burn}

See [Module Availability SLO Fast Burn](#module-availability-slo-fast-burn).

GL-specific: period close jobs are the riskiest operation. If errors spike during month-end, check for period close conflicts.

### GL Availability SLO Slow Burn {#gl-availability-slo-slow-burn}

See [Module Availability SLO Slow Burn](#module-availability-slo-slow-burn).

### GL Latency SLO {#gl-latency-slo}

See [Module Latency SLO](#module-latency-slo).

---

## Production-Specific Alerts

### Production Availability SLO Fast Burn {#production-availability-slo-fast-burn}

See [Module Availability SLO Fast Burn](#module-availability-slo-fast-burn).

Production-specific: work order composite create calls BOM and Numbering services. If those are slow/down, production errors will spike.

### Production Availability SLO Slow Burn {#production-availability-slo-slow-burn}

See [Module Availability SLO Slow Burn](#module-availability-slo-slow-burn).

### Production Latency SLO {#production-latency-slo}

See [Module Latency SLO](#module-latency-slo).

---

## Inventory-Specific Alerts

### Inventory Availability SLO Fast Burn {#inventory-availability-slo-fast-burn}

See [Module Availability SLO Fast Burn](#module-availability-slo-fast-burn).

### Inventory Availability SLO Slow Burn {#inventory-availability-slo-slow-burn}

See [Module Availability SLO Slow Burn](#module-availability-slo-slow-burn).

### Inventory Latency SLO {#inventory-latency-slo}

See [Module Latency SLO](#module-latency-slo).

---

## Error Budget Status

To check current error budget consumption for all modules:

```promql
# Availability budget remaining (30d window)
1 - (
  sum by (module) (
    rate({__name__=~"(ap|ar|gl|production|inventory)_http_requests_total",status=~"5.."}[30d])
  )
  /
  sum by (module) (
    rate({__name__=~"(ap|ar|gl|production|inventory)_http_requests_total"}[30d])
  )
)
```

Monthly budget = 43.2 min of downtime per module. When budget is < 10% remaining, freeze non-critical deployments.

---

## Contact / Escalation

| Role | Contact |
|------|---------|
| Platform on-call | PagerDuty rotation: `7d-platform-oncall` |
| Platform lead | Slack: `#platform-leads` |
| CTO | Direct message |
| Status page | Update at: `status.7dmanufacturing.com` |
