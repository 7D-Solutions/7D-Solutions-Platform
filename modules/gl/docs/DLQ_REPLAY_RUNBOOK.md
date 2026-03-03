# GL DLQ Replay Runbook

This runbook documents operational replay of GL dead-lettered events in `failed_events`.

## Scope

Use this when GL event processing failed and events accumulated in `failed_events`.

## Preconditions

- GL database reachable (`DATABASE_URL`)
- Operator can run `cargo` commands in this repository
- Root cause fixed before replay (schema/data/code)

## 1) Inspect current DLQ backlog

```bash
psql "$DATABASE_URL" -c "
SELECT subject, tenant_id, COUNT(*) AS pending
FROM failed_events
GROUP BY subject, tenant_id
ORDER BY pending DESC;
"
```

## 2) Verify drill path (required before real replay)

Run the built-in drill; it inserts a synthetic DLQ record, replays it through the real journal posting path, and verifies cleanup.

```bash
cargo run --manifest-path modules/gl/Cargo.toml --bin dlq_replay_drill
```

Expected output includes:

- `pending_before=1`
- `journal_entries_for_event=1`
- `pending_after=0`
- `dlq_replay_drill=ok`

If drill fails, stop and resolve before replaying production DLQ rows.

## 3) Replay a targeted batch (manual SQL path)

Example: replay a single tenant + subject batch.

```bash
psql "$DATABASE_URL" -c "
SELECT event_id, subject, tenant_id, failed_at
FROM failed_events
WHERE tenant_id = 'TENANT_ID'
  AND subject = 'gl.events.posting.requested'
ORDER BY failed_at
LIMIT 100;
"
```

For each row, parse `envelope_json.payload` and re-submit through the same service path used by consumers. Use the drill binary as the reference implementation.

## 4) Validate replay result

```bash
psql "$DATABASE_URL" -c "
SELECT COUNT(*) AS still_pending
FROM failed_events
WHERE tenant_id = 'TENANT_ID'
  AND subject = 'gl.events.posting.requested';
"
```

Also validate journal side effects:

```bash
psql "$DATABASE_URL" -c "
SELECT source_event_id, COUNT(*)
FROM journal_entries
WHERE tenant_id = 'TENANT_ID'
GROUP BY source_event_id
ORDER BY COUNT(*) DESC
LIMIT 20;
"
```

Expected: one journal entry per replayed source event (idempotent duplicates do not create extra entries).

## 5) Post-replay checks

- Re-check `failed_events` trend after 5-10 minutes
- Confirm no new error pattern for same subject
- Capture replay window and counts in incident notes

## Rollback posture

Replay is idempotent by `event_id`. If a batch is interrupted, rerunning the same events is safe; duplicates are rejected by `processed_events`/`source_event_id` guards.
