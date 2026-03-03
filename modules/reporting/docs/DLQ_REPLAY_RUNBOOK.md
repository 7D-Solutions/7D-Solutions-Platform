# Reporting DLQ Replay Runbook

Operational recovery guide for reporting ingest failures. The reporting module consumes domain events via NATS and populates cache tables. When ingestion fails, the checkpoint for the affected consumer/tenant stalls and new events are not processed.

## Scope

Use this when a reporting ingest consumer stops making progress (checkpoint stuck, cache stale, or ingest errors in logs).

## Preconditions

- Reporting database reachable (`REPORTING_DATABASE_URL` or `DATABASE_URL`)
- Root cause fixed first (schema mismatch, source event format change, DB connectivity)
- Operator can run repository binaries

## 1) Assess ingest health

Check which consumers have checkpoints and when they last advanced:

```bash
psql "$REPORTING_DATABASE_URL" -c "
SELECT consumer_name, tenant_id, last_sequence, last_event_id,
       processed_at, NOW() - processed_at AS lag
FROM rpt_ingestion_checkpoints
ORDER BY processed_at ASC
LIMIT 50;
"
```

Identify stale consumers (large `lag` values):

```bash
psql "$REPORTING_DATABASE_URL" -c "
SELECT consumer_name, tenant_id, processed_at,
       NOW() - processed_at AS lag
FROM rpt_ingestion_checkpoints
WHERE processed_at < NOW() - INTERVAL '1 hour'
ORDER BY lag DESC;
"
```

## 2) Validate replay procedure via drill (required)

```bash
cargo run --manifest-path modules/reporting/Cargo.toml --bin dlq_replay_drill
```

Expected output:

- `checkpoint_saved=true`
- `single_reset=ok deleted=1`
- `reset_all=ok deleted=2`
- `snapshot_idempotent=ok rows=N`
- `dlq_replay_drill=ok`

If drill fails, stop and fix the replay path before touching production checkpoints.

## 3) Replay workflow

Replay is a checkpoint-reset operation:

- **Guard**: confirm consumer is genuinely stalled, not just slow
- **Reset**: delete the checkpoint for the affected (consumer_name, tenant_id) pair
- **Effect**: on next service restart (or consumer re-subscribe), all events are re-processed from the beginning, rebuilding the cache from scratch

### Single tenant reset

```sql
DELETE FROM rpt_ingestion_checkpoints
WHERE consumer_name = 'reporting.ar_aging'
  AND tenant_id = 'TENANT_ID';
```

### Full consumer reset (all tenants)

```sql
DELETE FROM rpt_ingestion_checkpoints
WHERE consumer_name = 'reporting.ar_aging';
```

After reset, restart the reporting service or wait for the consumer to re-subscribe.

## 4) Cache rebuild safety

All reporting cache tables use idempotent upserts (`ON CONFLICT DO UPDATE`). Re-processing events never creates duplicate rows — it overwrites cache entries with recomputed values. Tables protected:

- `rpt_trial_balance_cache`
- `rpt_statement_cache`
- `rpt_ar_aging_cache`
- `rpt_ap_aging_cache`
- `rpt_cashflow_cache`
- `rpt_kpi_cache`
- `rpt_payment_history`
- `rpt_open_invoices_cache`

## 5) Snapshot runner recovery

If the daily snapshot job (`run_snapshot`) failed partway through, re-run it for the affected date range. The operation is fully idempotent:

```bash
# The admin API endpoint triggers a rebuild
curl -X POST http://localhost:8096/api/reporting/rebuild \
  -H "Content-Type: application/json" \
  -d '{"tenant_id": "TENANT_ID", "from": "2026-02-01", "to": "2026-02-28"}'
```

## 6) Metrics / signals to watch

- Checkpoint freshness (stale consumers):

```sql
SELECT consumer_name, COUNT(*), MAX(NOW() - processed_at) AS max_lag
FROM rpt_ingestion_checkpoints
GROUP BY consumer_name
ORDER BY max_lag DESC;
```

- Cache row counts (detect missing data):

```sql
SELECT 'trial_balance' AS cache, COUNT(*) FROM rpt_trial_balance_cache WHERE tenant_id = 'TENANT_ID'
UNION ALL
SELECT 'ar_aging', COUNT(*) FROM rpt_ar_aging_cache WHERE tenant_id = 'TENANT_ID'
UNION ALL
SELECT 'ap_aging', COUNT(*) FROM rpt_ap_aging_cache WHERE tenant_id = 'TENANT_ID'
UNION ALL
SELECT 'cashflow', COUNT(*) FROM rpt_cashflow_cache WHERE tenant_id = 'TENANT_ID'
UNION ALL
SELECT 'kpi', COUNT(*) FROM rpt_kpi_cache WHERE tenant_id = 'TENANT_ID';
```

## 7) Replay safety notes

- Checkpoint resets are safe because all downstream cache tables use `ON CONFLICT DO UPDATE` guards.
- Re-processing produces identical cache state (deterministic computations on the same events).
- Consumer names follow the pattern `reporting.<stream>` (e.g., `reporting.ar_aging`, `reporting.gl_trial_balance`, `reporting.payments`).
- There is no separate DLQ table — the replay mechanism is checkpoint deletion + re-ingest.
