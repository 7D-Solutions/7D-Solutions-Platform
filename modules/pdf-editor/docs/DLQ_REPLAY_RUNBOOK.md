# PDF Editor DLQ Replay Runbook

Scope: transactional outbox (`events_outbox`) for all pdf-editor event publishing.

## Architecture

PDF Editor mutations (form submission, PDF generation) write events to `events_outbox` atomically within the same DB transaction as the business mutation. A background publisher reads pending rows and sends them to NATS. If NATS is unreachable or the publisher crashes, rows accumulate with `status = 'pending'`.

Event types emitted via `events_outbox`:

| Event Type | Source Operation |
|---|---|
| `pdf.form.submitted` | Form submission finalized (draft → submitted) |
| `pdf.form.generated` | Filled PDF generated from a submitted form |

## Failure Modes

1. **NATS unavailable**: Publisher cannot connect. Outbox rows accumulate. Business mutations succeed (atomicity preserved).
2. **Publisher crash**: Background task dies. Rows written but never published. Restart the service to recover.
3. **Serialization failure**: Envelope validation rejects malformed payloads at write time (prevents bad rows from entering outbox). If this happens, the business mutation also rolls back.
4. **Partial publish**: Publisher reads a batch, publishes some, crashes mid-batch. Already-published rows have `status = 'published'`; remaining rows retry on next cycle.

## Failure Signal

- Outbox rows with `status = 'pending'` growing over time.
- NATS consumer lag increasing (downstream projections stale).
- Logs: `outbox publisher` errors or absence of publish activity.

## Inspect: Candidate Query

Identify unpublished outbox rows older than 5 minutes:

```sql
SELECT id, event_id, event_type, tenant_id, created_at
FROM events_outbox
WHERE status = 'pending'
  AND created_at < now() - interval '5 minutes'
ORDER BY id
LIMIT 200;
```

Count by event type:

```sql
SELECT event_type, COUNT(*) AS stuck_count
FROM events_outbox
WHERE status = 'pending'
  AND created_at < now() - interval '5 minutes'
GROUP BY event_type
ORDER BY stuck_count DESC;
```

## Replay Procedure

1. **Confirm NATS is healthy.** If NATS is down, fix NATS first. Replaying without a working bus just marks rows as published without delivery.

2. **Freeze deploys** for the pdf-editor service to prevent race conditions.

3. **Run candidate query** and capture row count.

4. **Replay in batches** using a lock-safe update transaction:

```sql
WITH candidates AS (
  SELECT id
  FROM events_outbox
  WHERE status = 'pending'
    AND created_at < now() - interval '5 minutes'
  ORDER BY id
  FOR UPDATE SKIP LOCKED
  LIMIT 100
)
UPDATE events_outbox o
SET status = 'published', published_at = now()
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
| Unpublished outbox rows | `SELECT COUNT(*) FROM events_outbox WHERE status = 'pending'` | 0 (or very small, recently created) |
| Publisher activity | Service logs: `outbox publisher` | Regular publish cycles logged |
| Consumer lag | `processed_events` count vs `events_outbox` count | Counts roughly equal |
| NATS connectivity | Service logs: NATS connection errors | No errors |

## Idempotency Safety

All pdf-editor events carry a unique `event_id` (UUID v4). Downstream consumers use `processed_events` to deduplicate. Replaying the same event twice is safe:

- The consumer checks `event_id` in `processed_events` before processing.
- If already processed, the duplicate is silently skipped.
- `replay_safe = true` on all pdf-editor envelopes.

## Drill Command (Real Postgres)

Run a repeatable drill (inserts synthetic stuck rows, replays them, verifies result):

```bash
cargo run --manifest-path modules/pdf-editor/Cargo.toml --bin dlq_replay_drill
```

Expected terminal output includes:

- `pending_candidates_before=<n>`
- `replayed_rows=<n>`
- `pending_candidates_after=0`
- `idempotent_replay_rows=0`
- `dlq_replay_drill=ok`
