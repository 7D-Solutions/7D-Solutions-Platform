# Integrations DLQ Replay Runbook

This runbook documents operational replay of dead-lettered integration events in `failed_events`.

## Scope

Use this when integration event processing failed and events accumulated in `failed_events`. Common causes: external system timeout, webhook routing misconfiguration, database constraint violation during external ref upsert.

## Preconditions

- Integrations database reachable (`DATABASE_URL`)
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

Run the built-in drill; it inserts a synthetic DLQ record, replays it through the real external ref creation path, and verifies cleanup.

```bash
cargo run --manifest-path modules/integrations/Cargo.toml --bin dlq_replay_drill
```

Expected output includes:

- `pending_before=1`
- `ref_created=1`
- `outbox_events>=1`
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
  AND subject = 'integrations.events.external_ref.created'
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
  AND subject LIKE 'integrations.events.%';
"
```

Also validate side effects:

```bash
psql "$DATABASE_URL" -c "
SELECT event_type, COUNT(*)
FROM integrations_outbox
WHERE app_id = 'TENANT_ID'
GROUP BY event_type
ORDER BY COUNT(*) DESC
LIMIT 20;
"
```

Expected: one outbox event per replayed source event (idempotent duplicates do not create extra entries due to UNIQUE constraints).

## 5) Post-replay checks

- Re-check `failed_events` trend after 5-10 minutes
- Confirm no new error pattern for same subject
- Capture replay window and counts in incident notes

## Rollback posture

Replay is idempotent by design. External ref creation uses `ON CONFLICT ... DO UPDATE`, so replaying the same event multiple times is safe. Webhook ingest uses the `integrations_webhook_ingest_dedup` constraint to prevent duplicate raw payload storage.
