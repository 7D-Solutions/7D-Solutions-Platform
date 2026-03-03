# Notifications DLQ Replay Runbook

Operational recovery guide for notification records stuck in `dead_lettered` state.

## Scope

Use this when `scheduled_notifications.status = 'dead_lettered'` accumulates and deliveries must be replayed safely.

## Preconditions

- Notifications database reachable (`DATABASE_URL`)
- Root cause fixed first (template bug, provider outage, bad payload)
- Operator can run repository binaries

## 1) Assess DLQ backlog

```bash
psql "$DATABASE_URL" -c "
SELECT tenant_id, channel, COUNT(*) AS dead_lettered
FROM scheduled_notifications
WHERE status = 'dead_lettered'
GROUP BY tenant_id, channel
ORDER BY dead_lettered DESC;
"
```

Optional detail for one tenant:

```bash
psql "$DATABASE_URL" -c "
SELECT id, template_key, retry_count, dead_lettered_at, last_error
FROM scheduled_notifications
WHERE tenant_id = 'TENANT_ID' AND status = 'dead_lettered'
ORDER BY dead_lettered_at DESC
LIMIT 100;
"
```

## 2) Validate replay procedure via drill (required)

```bash
cargo run --manifest-path modules/notifications/Cargo.toml --bin dlq_replay_drill
```

Expected output:

- `pending_before=1`
- `pending_after=0`
- `new_status=pending`
- `replay_outbox_rows=1` (or higher)
- `dlq_replay_drill=ok`

If drill fails, stop and fix replay path before touching production DLQ rows.

## 3) Replay workflow

Replay is a guarded transition:

- Guard: row must be `dead_lettered` and tenant-scoped
- Mutation:
  - `status -> pending`
  - `deliver_at -> NOW()`
  - `retry_count -> 0`
  - `replay_generation += 1`
  - clear `last_error`, `dead_lettered_at`, `failed_at`
- Outbox event: `notifications.events.dlq.replayed` with payload containing notification id and status transition

This is implemented in the DLQ API handler and the drill binary.

## 4) Metrics / signals to watch

- Dead-letter backlog trend:

```sql
SELECT COUNT(*) FROM scheduled_notifications WHERE status='dead_lettered';
```

- Dispatch throughput health:

```sql
SELECT status, COUNT(*)
FROM scheduled_notifications
GROUP BY status
ORDER BY status;
```

- Replay audit events:

```sql
SELECT COUNT(*)
FROM events_outbox
WHERE subject = 'notifications.events.dlq.replayed'
  AND created_at > NOW() - INTERVAL '1 hour';
```

## 5) Replay safety notes

- Replays are idempotent at the delivery-attempt layer using keys:
  `notif:{notification_id}:gen:{replay_generation}:attempt:{attempt_no}`
- Incrementing `replay_generation` creates a new replay epoch while preserving dedupe inside each epoch.
- Rows in `abandoned` status are intentionally excluded from dispatch; do not replay abandoned items unless policy explicitly allows reactivation.
