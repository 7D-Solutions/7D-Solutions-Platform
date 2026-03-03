# Outbox Metrics Standard

Every service that uses the outbox pattern MUST expose the following Prometheus
metrics on its `/metrics` endpoint. The naming convention is
`{module}_outbox_{metric}`.

## Required Metrics

| Metric Name | Type | Description |
|---|---|---|
| `{module}_outbox_queue_depth` | Gauge | Number of unpublished events (`published_at IS NULL`). Refreshed on each Prometheus scrape. |
| `{module}_events_enqueued_total` | Counter | Lifetime count of events inserted into the outbox. |

### Metric Name Examples

| Service | Queue Depth | Enqueued Total |
|---|---|---|
| AR | `ar_outbox_queue_depth` | `ar_events_enqueued_total` |
| Payments | `payments_outbox_queue_depth` | `payments_events_enqueued_total` |
| Subscriptions | `subscriptions_outbox_queue_depth` | `subscriptions_events_enqueued_total` |
| GL | `gl_outbox_queue_depth` | `gl_events_enqueued_total` |
| AP | `ap_outbox_queue_depth` | `ap_events_enqueued_total` |
| Maintenance | `maintenance_outbox_queue_depth` | `maintenance_events_enqueued_total` |
| Workflow | `workflow_outbox_queue_depth` | `workflow_events_enqueued_total` |
| Notifications | `notifications_outbox_queue_depth` | `notifications_events_enqueued_total` |

## Regex for Dashboards and Alerts

```
# Match any service's outbox queue depth
{__name__=~".+_outbox_queue_depth"}

# Match any service's enqueued counter
{__name__=~".+_events_enqueued_total"}
```

## Platform-Level Metrics (from existing alert rules)

These metrics are referenced in alert rules and SHOULD be emitted by the
outbox publisher or consumer retry infrastructure:

| Metric | Type | Description |
|---|---|---|
| `outbox_insert_failures_total{module}` | Counter | Outbox insert failed (state drift risk) |
| `dlq_events_total` | Counter | Events landing in DLQ after retry exhaustion |
| `dlq_events_by_subject{subject}` | Counter | DLQ events broken down by subject |

## Implementation Pattern

Each module implements outbox metrics the same way:

1. Add `count_unpublished(pool) -> Result<i64>` to the outbox module.
2. Add `outbox_queue_depth: IntGauge` to the metrics struct.
3. In the `/metrics` handler, call `count_unpublished()` and set the gauge
   before encoding.

```rust
// In metrics handler:
match crate::outbox::count_unpublished(&state.pool).await {
    Ok(depth) => state.metrics.outbox_queue_depth.set(depth),
    Err(e) => tracing::warn!("Failed to fetch outbox queue depth: {}", e),
}
```

## Scrape Behavior

Queue depth is a **point-in-time snapshot** refreshed every 15 seconds
(Prometheus scrape interval). A sustained non-zero value means the publisher
is falling behind. A zero value means all events have been published.
