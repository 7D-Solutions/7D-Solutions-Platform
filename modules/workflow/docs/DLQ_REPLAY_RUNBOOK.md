# Workflow DLQ Replay Runbook

Scope: transactional outbox (`events_outbox`) for all workflow event publishing.

## Architecture

Workflow mutations (create definition, start instance, advance, cancel, hold/release, delegation, escalation) write events to `events_outbox` atomically within the same DB transaction as the business mutation. A background publisher reads unpublished rows and sends them to NATS. If NATS is unreachable or the publisher crashes, rows accumulate with `published_at IS NULL`.

Event types emitted via `events_outbox`:

| Event Type | Source Operation |
|---|---|
| `workflow.events.definition.created` | Create workflow definition |
| `workflow.events.instance.started` | Start workflow instance |
| `workflow.events.instance.advanced` | Advance instance to next step |
| `workflow.events.instance.completed` | Instance reaches terminal completed state |
| `workflow.events.instance.cancelled` | Instance is cancelled |
| `workflow.events.step.decision_recorded` | Actor records decision at parallel step |
| `workflow.events.step.parallel_threshold_met` | Parallel step threshold reached, auto-advance |
| `workflow.events.hold.applied` | Hold applied to entity |
| `workflow.events.hold.released` | Hold released from entity |
| `workflow.events.escalation.fired` | Escalation timer fires |
| `workflow.events.delegation.created` | Actor delegates authority |
| `workflow.events.delegation.revoked` | Delegation revoked |

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
SELECT id, event_id, event_type, aggregate_type, created_at
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

2. **Freeze deploys** for the workflow service to prevent race conditions.

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

After replay, confirm downstream consumers processed the events:

```sql
-- Check that replayed event_ids appear in processed events
SELECT e.event_id, e.event_type, p.processed_at
FROM events_outbox e
LEFT JOIN processed_events p ON p.event_id = e.event_id
WHERE e.published_at > now() - interval '30 minutes'
ORDER BY e.created_at;
```

If `processed_at` is NULL for replayed rows, the consumer may need investigation.

## Recovery Metrics and Logs

| What to check | Where | Healthy signal |
|---|---|---|
| Unpublished outbox rows | `SELECT COUNT(*) FROM events_outbox WHERE published_at IS NULL` | 0 (or very small, recently created) |
| Publisher activity | Service logs: `outbox publisher` | Regular publish cycles logged |
| Consumer lag | `processed_events` count vs `events_outbox` count | Counts roughly equal |
| NATS connectivity | Service logs: NATS connection errors | No errors |

## Idempotency Safety

All workflow events carry a deterministic `event_id` (UUID v4 generated at mutation time). Downstream consumers use `processed_events` to deduplicate. Replaying the same event twice is safe:

- The consumer checks `event_id` in `processed_events` before processing.
- If already processed, the duplicate is silently skipped.
- `replay_safe = true` on all workflow envelopes.

## Drill Command (Real Postgres)

Run a repeatable drill (inserts synthetic stuck rows, replays them, verifies result):

```bash
cargo run --manifest-path modules/workflow/Cargo.toml --bin dlq_replay_drill
```

Expected terminal output includes:

- `pending_candidates_before=<n>`
- `replayed_rows=<n>`
- `idempotent_replay_rows=0`
- `pending_candidates_after=0`
- `dlq_replay_drill=ok`
