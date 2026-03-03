# Outbox Health Alerting — Tuning & First-Response Runbook

> **Alert rules file:** `infra/monitoring/alerts/outbox-health.yml`
> **Dashboard:** [Outbox Health](https://grafana.7dsolutions.com/d/outbox-health/outbox-health)
> **Metrics standard:** `docs/outbox-metrics-standard.md`

## Alert Summary

| Alert | Severity | Threshold | Duration | Meaning |
|-------|----------|-----------|----------|---------|
| OutboxBacklogWarning | warning | queue depth > 10 | 10 min | Publisher is slow or encountering transient errors |
| BillingOutboxBacklogCritical | critical | billing queue depth > 100 | 5 min | Billing event delivery is degraded |
| OutboxBacklogEmergency | critical | queue depth > 500 | 2 min | Publisher likely dead or NATS unreachable |
| DLQGrowthWarning | warning | DLQ rate > 1 event/min | 10 min | Consumers failing after retry exhaustion |

## Threshold Tuning Guide

### Queue Depth Thresholds

The queue depth gauge (`{module}_outbox_queue_depth`) counts rows in the outbox table where `published_at IS NULL`. Under normal operation this should be 0 or near-zero — the publisher polls every few seconds.

**Warning (> 10 for 10 min):** A small backlog can occur during deploy restarts or brief NATS hiccups. The 10-minute `for` duration filters out transient spikes. If this fires frequently during normal operation, consider:
- Increasing the threshold to 20–30
- Extending the `for` duration to 15 min
- Checking if the publisher poll interval is too slow for the event volume

**Critical (> 100 for 5 min — billing spine only):** This targets AR, payments, and subscriptions specifically because billing event delays have direct financial impact. If this fires:
- The billing publisher is likely backlogged, not dead (otherwise you'd see the emergency alert)
- Consider raising to 200 if the billing modules routinely generate bursts > 100 events (e.g., batch invoice runs)

**Emergency (> 500 for 2 min):** This should almost never fire. A backlog of 500+ means the publisher task is dead, NATS is down, or the database is under extreme load. The short 2-minute window is intentional — at this level, every minute of delay compounds downstream staleness.

### DLQ Growth Rate

The DLQ growth alert uses `rate(dlq_events_total[10m]) * 60 > 1`, meaning more than 1 event per minute entering the DLQ sustained over 10 minutes.

- **Tuning up:** If background noise (e.g., schema evolution causing occasional consumer failures) triggers this too often, raise to `> 3` events/min
- **Tuning down:** For critical services where any DLQ entry is concerning, lower to `> 0.5` events/min or reduce the `for` duration

## First Response Playbook

### OutboxBacklogWarning / BillingOutboxBacklogCritical

1. **Open the dashboard:** Check the "Outbox Queue Depth — All Services" time series to see which service(s) have backlogs and whether the trend is growing or stable
2. **Check publisher logs:** Each service runs an outbox publisher background task. Look for connection errors, timeouts, or panics:
   ```bash
   docker logs <service-container> 2>&1 | grep -i "outbox\|publish\|nats" | tail -50
   ```
3. **Check NATS health:** If multiple services are backlogged simultaneously, the issue is likely NATS:
   ```bash
   nats server check jetstream
   nats stream ls
   ```
4. **Check database load:** High database CPU/connections can slow the publisher's polling queries:
   ```bash
   # Check active connections
   psql -c "SELECT count(*) FROM pg_stat_activity WHERE state = 'active';"
   ```
5. **Restart the publisher:** If the publisher task panicked, restarting the service container clears it:
   ```bash
   docker restart <service-container>
   ```

### OutboxBacklogEmergency

This is a **page-level** alert. The publisher is likely dead.

1. **Immediately check if the service is running:**
   ```bash
   docker ps | grep <service>
   ```
2. **If running but not publishing:** The publisher background task may have panicked. Check logs for panic traces, then restart
3. **If NATS is down:** All services will show backlog simultaneously. Restore NATS first — the publishers will drain backlogs automatically once NATS reconnects
4. **If the database is unreachable:** The publisher can't poll. Check database health and connectivity
5. **After recovery:** Watch the dashboard — queue depths should drain to 0 within minutes. If they don't, the publisher may need another restart

### DLQGrowthWarning

DLQ growth means events are exhausting retries and being moved to the dead-letter queue. The events are safe but not being processed.

1. **Check the "DLQ Events by Subject" panel** to identify which event type is failing
2. **Check consumer logs** for the failing event subject:
   ```bash
   docker logs <consumer-service> 2>&1 | grep -i "dlq\|retry\|failed" | tail -50
   ```
3. **Common causes:**
   - Schema mismatch: a producer emits a new event shape that the consumer doesn't handle yet
   - Downstream service outage: the consumer depends on a service that's down
   - Data integrity: the event references an entity that doesn't exist (e.g., deleted tenant)
4. **Recovery:** After fixing the root cause, replay DLQ events:
   ```bash
   # See docs/runbooks/backup_restore.md for DLQ replay procedures
   ```

## Metric Sources

| Metric | Type | Source |
|--------|------|--------|
| `{module}_outbox_queue_depth` | Gauge | `SELECT COUNT(*) FROM {outbox_table} WHERE published_at IS NULL` — refreshed on each `/metrics` scrape |
| `dlq_events_total` | Counter | Incremented when an event is moved to the DLQ after retry exhaustion |
| `outbox_insert_failures_total` | Counter | Incremented when an outbox INSERT fails (state drift risk — the domain mutation committed but the event was lost) |

## Dashboard Panels

The Outbox Health dashboard (`uid: outbox-health`) has four sections:

1. **Stat panels** — Current queue depth per service, color-coded green/yellow/red
2. **Queue depth time series** — All services overlaid, showing backlog trends
3. **DLQ health** — Event rate and per-subject breakdown
4. **Insert failures** — Outbox write failures by module (state drift indicator)
