# Shipping-Receiving DLQ Replay Runbook

Scope: transactional outbox (`sr_events_outbox`) for all shipping-receiving event publishing.

## Architecture

Shipping-receiving mutations (create shipment, status transition, inbound close, outbound ship) write events to `sr_events_outbox` atomically within the same DB transaction as the business mutation. A background publisher reads unpublished rows and sends them to NATS. If NATS is unreachable or the publisher crashes, rows accumulate with `published_at IS NULL`.

Event types emitted via `sr_events_outbox`:

| Event Type | Source Operation |
|---|---|
| `shipping.shipment.created` | New shipment created |
| `shipping.shipment.status_changed` | Status transition |
| `shipping.inbound.closed` | Inbound shipment fully received & closed |
| `shipping.outbound.shipped` | Outbound shipment handed to carrier |
| `shipping.outbound.delivered` | Outbound shipment confirmed delivered |

## Failure Modes

1. **NATS unavailable**: Publisher cannot connect. Outbox rows accumulate. Business mutations succeed (atomicity preserved).
2. **Publisher crash**: Background task dies. Rows written but never published. Restart the service to recover.
3. **Serialization failure**: Envelope validation rejects malformed payloads at write time (prevents bad rows from entering outbox). If this happens, the business mutation also rolls back.
4. **Partial publish**: Publisher reads a batch, publishes some, crashes mid-batch. Already-published rows have `published_at` set; remaining rows retry on next cycle.

## Failure Signal

- Outbox rows with `published_at IS NULL` growing over time.
- NATS consumer lag increasing (downstream projections stale).
- Logs: `outbox publisher` errors or absence of publish activity.

## Inspect: Candidate Query

Identify unpublished outbox rows older than 5 minutes:

```sql
SELECT id, event_id, event_type, tenant_id, created_at
FROM sr_events_outbox
WHERE published_at IS NULL
  AND created_at < now() - interval '5 minutes'
ORDER BY id
LIMIT 200;
```

Count by event type:

```sql
SELECT event_type, COUNT(*) AS stuck_count
FROM sr_events_outbox
WHERE published_at IS NULL
  AND created_at < now() - interval '5 minutes'
GROUP BY event_type
ORDER BY stuck_count DESC;
```

## Replay Procedure

1. **Confirm NATS is healthy.** If NATS is down, fix NATS first. Replaying without a working bus just marks rows as published without delivery.

2. **Freeze deploys** for the shipping-receiving service to prevent race conditions.

3. **Run candidate query** and capture row count.

4. **Replay in batches** using a lock-safe update transaction:

```sql
WITH candidates AS (
  SELECT id
  FROM sr_events_outbox
  WHERE published_at IS NULL
    AND created_at < now() - interval '5 minutes'
  ORDER BY id
  FOR UPDATE SKIP LOCKED
  LIMIT 100
)
UPDATE sr_events_outbox o
SET published_at = now()
FROM candidates c
WHERE o.id = c.id
RETURNING o.id, o.event_type;
```

5. **Re-run candidate query** until empty.

6. **Resume deploys** and monitor for 15 minutes.

## Post-Replay Verification

After replay, confirm downstream consumers processed the events:

```sql
-- Check that replayed event_ids appear in processed events
SELECT e.event_id, e.event_type, p.processed_at
FROM sr_events_outbox e
LEFT JOIN sr_processed_events p ON p.event_id = e.event_id
WHERE e.published_at > now() - interval '30 minutes'
ORDER BY e.created_at;
```

If `processed_at` is NULL for replayed rows, the consumer may need investigation.

## Recovery Metrics and Logs

| What to check | Where | Healthy signal |
|---|---|---|
| Unpublished outbox rows | `SELECT COUNT(*) FROM sr_events_outbox WHERE published_at IS NULL` | 0 (or very small, recently created) |
| Publisher activity | Service logs: `outbox publisher` | Regular publish cycles logged |
| Consumer lag | `sr_processed_events` count vs `sr_events_outbox` count | Counts roughly equal |
| NATS connectivity | Service logs: NATS connection errors | No errors |

## Idempotency Safety

All shipping-receiving events carry a deterministic `event_id`. Downstream consumers use `sr_processed_events` to deduplicate. Replaying the same event twice is safe:

- The consumer checks `event_id` in `sr_processed_events` before processing.
- If already processed, the duplicate is silently skipped.
- `replay_safe = true` on all shipping-receiving envelopes.

## Drill Command (Real Postgres)

Run a repeatable drill (inserts synthetic stuck rows, replays them, verifies result):

```bash
cargo run --manifest-path modules/shipping-receiving/Cargo.toml --bin dlq_replay_drill
```

Expected terminal output includes:

- `pending_candidates_before=<n>`
- `replayed_rows=<n>`
- `pending_candidates_after=0`
- `dlq_replay_drill=ok`
