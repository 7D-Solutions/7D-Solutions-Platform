# Doc-Mgmt DLQ Replay Runbook

Scope: publication and distribution workflows backed by `doc_outbox`.

## Failure Signal

- Outbox rows with `published_at IS NULL` growing over time.
- Affected event types:
  - `document.created`
  - `document.released`
  - `document.distribution.requested`
  - `document.distribution.status.updated`

## Candidate Query

Use this query to identify replay candidates older than 5 minutes:

```sql
SELECT id, event_type, subject, created_at
FROM doc_outbox
WHERE published_at IS NULL
  AND created_at < now() - interval '5 minutes'
ORDER BY id
LIMIT 200;
```

## Replay Procedure

1. Freeze deploys for `doc-mgmt`.
2. Run candidate query and capture row count.
3. Replay in batches using a lock-safe update transaction:

```sql
WITH candidates AS (
  SELECT id
  FROM doc_outbox
  WHERE published_at IS NULL
    AND created_at < now() - interval '5 minutes'
  ORDER BY id
  FOR UPDATE SKIP LOCKED
  LIMIT 100
)
UPDATE doc_outbox o
SET published_at = now()
FROM candidates c
WHERE o.id = c.id
RETURNING o.id, o.event_type;
```

4. Re-run candidate query until empty.
5. Resume deploys and monitor for 15 minutes.

## Drill Command (Real Postgres)

Run a repeatable drill (inserts synthetic stuck rows, replays them, verifies result):

```bash
cargo run --manifest-path platform/doc-mgmt/Cargo.toml --bin dlq_replay_drill
```

Expected terminal output includes:

- `pending_candidates_before=<n>`
- `replayed_rows=<n>`
- `pending_candidates_after=<m>`
- `dlq_replay_drill=ok`
