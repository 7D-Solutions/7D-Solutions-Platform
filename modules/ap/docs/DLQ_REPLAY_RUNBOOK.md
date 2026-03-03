# AP DLQ Replay Runbook

Operational recovery guide for AP outbox publishing failures. The AP module uses a transactional outbox (Guard→Mutation→Outbox atomicity) to publish domain events to NATS. When the outbox publisher stalls or fails, events accumulate as unpublished rows.

## Scope

Use this when AP events are not reaching downstream consumers (GL posting, reporting, notifications) because the outbox publisher has stalled.

## Preconditions

- AP database reachable (`DATABASE_URL` or default `postgres://ap_user:ap_pass@localhost:5443/ap_db`)
- Root cause fixed first (NATS connectivity, publisher crash, DB connectivity)
- Operator can run repository binaries

## 1) Assess outbox health

Check for unpublished events (stalled outbox):

```bash
psql "$DATABASE_URL" -c "
SELECT event_type, COUNT(*) AS pending,
       MIN(created_at) AS oldest,
       NOW() - MIN(created_at) AS max_lag
FROM events_outbox
WHERE published_at IS NULL
GROUP BY event_type
ORDER BY max_lag DESC;
"
```

Check recent publishing activity:

```bash
psql "$DATABASE_URL" -c "
SELECT event_type, COUNT(*) AS published,
       MAX(published_at) AS last_published
FROM events_outbox
WHERE published_at IS NOT NULL
  AND published_at > NOW() - INTERVAL '1 hour'
GROUP BY event_type
ORDER BY last_published DESC;
"
```

## 2) Validate replay procedure via drill (required)

```bash
cargo run --manifest-path modules/ap/Cargo.toml --bin dlq_replay_drill
```

Expected output:

- `vendor_created=<uuid>`
- `fetch_unpublished=found`
- `mark_published=ok event_id=<uuid>`
- `post_publish_check=ok (event gone from unpublished)`
- `replay_simulation=ok (event reappears after reset)`
- `republish_idempotent=ok`
- `dlq_replay_drill=ok`

If drill fails, stop and fix the replay path before touching production data.

## 3) Replay workflow

AP replay is an outbox re-publish operation:

- **Guard**: confirm publisher is genuinely stalled, not just slow
- **Reset**: set `published_at = NULL` for the affected event(s)
- **Effect**: on next publisher tick (1-second poll), the events are re-fetched and re-published to NATS

### Single event replay

```sql
UPDATE events_outbox
SET published_at = NULL
WHERE event_id = 'EVENT_UUID';
```

### Replay all failed events for a specific type

```sql
UPDATE events_outbox
SET published_at = NULL
WHERE event_type = 'ap.vendor_bill_approved'
  AND published_at IS NULL;
```

This is a no-op if the events were never published — they will be picked up on the next tick automatically.

### Full outbox replay (all event types)

```sql
UPDATE events_outbox
SET published_at = NULL
WHERE published_at IS NOT NULL
  AND created_at > NOW() - INTERVAL '24 hours';
```

After reset, either restart the AP service or wait for the publisher loop to re-process (1-second poll interval).

## 4) Publishing safety

All downstream consumers are idempotent by design:

- GL posting uses idempotency keys from event_id
- Reporting cache tables use `ON CONFLICT DO UPDATE`
- Notification consumers deduplicate on event_id

Re-publishing the same event produces no side effects beyond the initial processing.

## 5) Event types reference

| Event Type | Aggregate | Description |
|---|---|---|
| `ap.vendor_created` | vendor | New vendor registered |
| `ap.vendor_updated` | vendor | Vendor fields or status changed |
| `ap.po_created` | po | Draft PO created |
| `ap.po_approved` | po | PO approved for fulfillment |
| `ap.po_closed` | po | PO closed (received/cancelled) |
| `ap.po_line_received_linked` | po | Receipt linked to PO line |
| `ap.vendor_bill_created` | bill | New vendor bill entered |
| `ap.vendor_bill_matched` | bill | Bill matched to PO |
| `ap.vendor_bill_approved` | bill | Bill approved for payment |
| `ap.vendor_bill_voided` | bill | Bill voided (reversal) |
| `ap.payment_run_created` | payment_run | Payment run initiated |
| `ap.payment_executed` | payment_run | Payment executed |

## 6) Metrics / signals to watch

- Outbox backlog (unpublished events older than 5 minutes):

```sql
SELECT COUNT(*) AS backlog,
       MIN(created_at) AS oldest,
       NOW() - MIN(created_at) AS max_lag
FROM events_outbox
WHERE published_at IS NULL;
```

- Publisher throughput (events published per minute):

```sql
SELECT DATE_TRUNC('minute', published_at) AS minute,
       COUNT(*) AS published
FROM events_outbox
WHERE published_at > NOW() - INTERVAL '10 minutes'
GROUP BY 1
ORDER BY 1 DESC;
```

## 7) Replay safety notes

- Outbox re-publish is safe because all downstream consumers are idempotent on event_id.
- The publisher polls the outbox every 1 second, publishing up to 100 events per batch.
- Subjects follow the pattern `ap.events.<event_type>` (e.g., `ap.events.ap.vendor_bill_approved`).
- There is no separate DLQ table — the replay mechanism is `published_at` reset + publisher re-poll.
