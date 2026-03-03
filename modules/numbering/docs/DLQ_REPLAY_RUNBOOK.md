# Numbering DLQ Replay Runbook

Scope: transactional outbox (`events_outbox`) for all numbering event publishing.

## Architecture

Numbering mutations (allocate, confirm, policy upsert) write events to `events_outbox` atomically within the same DB transaction as the business mutation. A background publisher reads unpublished rows and sends them to NATS. If NATS is unreachable or the publisher crashes, rows accumulate with `published_at IS NULL`.

Event types emitted via `events_outbox`:

| Event Type | Source Operation |
|---|---|
| `numbering.events.number.allocated` | Number allocation (standard or gap-free reservation) |
| `numbering.events.number.confirmed` | Gap-free reservation confirmed |
| `numbering.events.policy.updated` | Numbering policy created or updated |

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
SELECT id, event_id, event_type, aggregate_type, aggregate_id, created_at
FROM events_outbox
WHERE published_at IS NULL
  AND created_at < now() - interval '5 minutes'
ORDER BY id
LIMIT 200;
```

Count by event type:

```sql
SELECT event_type, COUNT(*) AS stuck_count
FROM events_outbox
WHERE published_at IS NULL
  AND created_at < now() - interval '5 minutes'
GROUP BY event_type
ORDER BY stuck_count DESC;
```

## Replay Procedure

1. **Confirm NATS is healthy.** If NATS is down, fix NATS first. Replaying without a working bus just marks rows as published without delivery.

2. **Freeze deploys** for the numbering service to prevent race conditions.

3. **Run candidate query** and capture row count.

4. **Replay in batches** using a lock-safe update transaction:

```sql
WITH candidates AS (
  SELECT id
  FROM events_outbox
  WHERE published_at IS NULL
    AND created_at < now() - interval '5 minutes'
  ORDER BY id
  FOR UPDATE SKIP LOCKED
  LIMIT 100
)
UPDATE events_outbox o
SET published_at = now()
FROM candidates c
WHERE o.id = c.id
RETURNING o.id, o.event_type;
```

5. **Re-run candidate query** until empty.

6. **Resume deploys** and monitor for 15 minutes.

## Post-Replay Verification

After replay, confirm that no stuck rows remain:

```sql
SELECT COUNT(*)
FROM events_outbox
WHERE published_at IS NULL
  AND created_at < now() - interval '5 minutes';
```

Expected result: 0.

## Recovery Metrics and Logs

| What to check | Where | Healthy signal |
|---|---|---|
| Unpublished outbox rows | `SELECT COUNT(*) FROM events_outbox WHERE published_at IS NULL` | 0 (or very small, recently created) |
| Publisher activity | Service logs: `outbox publisher` | Regular publish cycles logged |
| NATS connectivity | Service logs: NATS connection errors | No errors |

## Idempotency Safety

All numbering events carry a unique `event_id` (UUID v4). Downstream consumers should deduplicate by `event_id`. Replaying the same batch twice is safe:

- The replay SQL uses `FOR UPDATE SKIP LOCKED` to prevent concurrent replay conflicts.
- Already-published rows (`published_at IS NOT NULL`) are excluded from candidate queries.
- `replay_safe = true` on all numbering envelopes.

## Drill Command (Real Postgres)

Run a repeatable drill (inserts synthetic stuck rows, replays them, verifies result):

```bash
cargo run --manifest-path modules/numbering/Cargo.toml --bin dlq_replay_drill
```

Expected terminal output includes:

- `pending_candidates_before=<n>`
- `replayed_rows=<n>`
- `idempotency_check_replayed=0`
- `pending_candidates_after=0`
- `dlq_replay_drill=ok`
