# Event Subscriptions Map

**Version**: 1.0.0
**Status**: Active
**Last Updated**: 2026-02-11

## Overview

This document defines the standard conventions for event subscriptions in the 7D Solutions Platform, including subject naming, durable consumer naming, dead letter queue (DLQ) handling, and versioning strategies.

## Subject Naming Convention

### Format

All event subjects MUST follow this pattern:

```
<module>.events.<event-type>
```

Where:
- **`<module>`** - The source module emitting the event (e.g., `ar`, `payments`, `auth`, `notifications`)
- **`events`** - Literal namespace separator indicating this is an event stream
- **`<event-type>`** - The specific event type following dot notation (e.g., `ar.invoice.issued`, `payments.payment.succeeded`)

### Examples

| Module | Event Type | Full Subject |
|--------|-----------|--------------|
| AR | ar.invoice.issued | `ar.events.ar.invoice.issued` |
| AR | ar.payment.collection.requested | `ar.events.ar.payment.collection.requested` |
| AR | ar.payment.applied | `ar.events.ar.payment.applied` |
| Payments | payments.payment.succeeded | `payments.events.payments.payment.succeeded` |
| Payments | payments.payment.failed | `payments.events.payments.payment.failed` |
| Payments | payments.refund.succeeded | `payments.events.payments.refund.succeeded` |
| Auth | auth.user.registered | `auth.events.user.registered` |
| Notifications | notifications.delivery.succeeded | `notifications.events.notifications.delivery.succeeded` |
| Subscriptions | subscriptions.created | `subscriptions.events.subscriptions.created` |
| GL | gl.posting.accepted | `gl.events.gl.posting.accepted` |

### Wildcard Subscriptions

The platform supports NATS-style wildcard patterns for subscribing to multiple events:

- **`*`** - Matches exactly one token
  - `auth.events.auth.user.*` matches `auth.events.auth.user.created`
  - Does NOT match `auth.events.auth.user.profile.updated`

- **`>`** - Matches one or more tokens (must be last in pattern)
  - `auth.events.>` matches ALL auth events
  - `*.events.*.payment.>` matches all payment-related events from any module

**Best Practice**: Use specific subscriptions rather than wildcards in production to ensure clear event routing and prevent accidental message consumption.

## Durable Consumer Naming

### Format

Durable consumers ensure that event processing can resume after service restarts and that multiple instances of the same consumer share the processing load. Consumer names MUST follow this pattern:

```
<module>-<purpose>-consumer
```

Where:
- **`<module>`** - The consuming module (e.g., `notifications`, `payments`)
- **`<purpose>`** - Brief description of what this consumer does (e.g., `invoice-notifier`, `payment-processor`)
- **`consumer`** - Literal suffix

### Examples

| Consumer Module | Purpose | Durable Name | Subscribes To |
|----------------|---------|--------------|---------------|
| Notifications | Process invoice events | `notifications-invoice-notifier-consumer` | `ar.events.ar.invoice.issued` |
| Notifications | Process payment successes | `notifications-payment-success-consumer` | `payments.events.payments.payment.succeeded` |
| Notifications | Process payment failures | `notifications-payment-failure-consumer` | `payments.events.payments.payment.failed` |
| Payments | Process AR payment requests | `payments-ar-collection-consumer` | `ar.events.ar.payment.collection.requested` |
| GL | Process posting requests | `gl-posting-processor-consumer` | `gl.events.gl.posting.request` |

### Consumer Group Semantics

- **Load Balancing**: Multiple instances of the same consumer (same durable name) will share message processing
- **At-Least-Once Delivery**: Messages are redelivered if not acknowledged
- **Ordering**: Messages are delivered in order per subject, but parallel consumers may process out of order
- **State Persistence**: Consumer state (last acked message) persists across restarts

### Implementation Note

Current implementation uses basic NATS subscriptions without explicit durable consumer setup. Future implementation should use JetStream durable consumers for production deployment.

```rust
// Future implementation pattern
let consumer_config = async_nats::jetstream::consumer::pull::Config {
    durable_name: Some("notifications-invoice-notifier-consumer".to_string()),
    filter_subject: "ar.events.ar.invoice.issued".to_string(),
    ack_policy: AckPolicy::Explicit,
    max_deliver: 5,
    ..Default::default()
};
```

## Dead Letter Queue (DLQ) Convention

### Purpose

DLQs capture events that fail processing after max retry attempts, enabling:
- Debugging and root cause analysis
- Manual intervention and replay
- Prevention of message loss
- Separation of failed messages from healthy processing

### Stream Naming

DLQ streams MUST follow this pattern:

```
<MODULE>_DLQ
```

Where `<MODULE>` is the uppercase name of the module (e.g., `AUTH_DLQ`, `PAYMENTS_DLQ`).

### Subject Pattern

DLQ subjects MUST follow this pattern:

```
<module>.dlq.<event-type>
```

Where:
- **`<module>`** - Source module in lowercase
- **`dlq`** - Literal namespace for dead letter messages
- **`<event-type>`** - Original event type that failed

### Configuration

| Setting | Value | Rationale |
|---------|-------|-----------|
| Retention | 30 days | Longer than regular events (14 days) for investigation |
| Max Age | 30 days | Automatic cleanup of old failures |
| Storage | File-based | Persist for debugging |
| Republish | Manual | Requires explicit intervention to replay |

### Examples

| Module | Stream Name | Subject Pattern | Example Failed Event |
|--------|-------------|-----------------|---------------------|
| Auth | `AUTH_DLQ` | `auth.dlq.*` | `auth.dlq.user.registered` |
| Payments | `PAYMENTS_DLQ` | `payments.dlq.*` | `payments.dlq.payment.succeeded` |
| AR | `AR_DLQ` | `ar.dlq.*` | `ar.dlq.invoice.issued` |

### Moving Messages to DLQ

When a consumer exhausts retry attempts:

```rust
async fn handle_failed_event(
    event_bus: &dyn EventBus,
    original_subject: &str,
    payload: Vec<u8>,
    error: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let module = original_subject.split('.').next().unwrap();
    let event_type = original_subject.split('.').skip(2).collect::<Vec<_>>().join(".");

    let dlq_subject = format!("{}.dlq.{}", module, event_type);

    // Add failure metadata to payload
    let dlq_payload = json!({
        "original_subject": original_subject,
        "original_payload": payload,
        "error": error,
        "failed_at": Utc::now().to_rfc3339(),
    });

    event_bus.publish(&dlq_subject, serde_json::to_vec(&dlq_payload)?).await?;

    tracing::error!(
        subject = original_subject,
        dlq_subject = dlq_subject,
        error = error,
        "Event moved to DLQ after max retries"
    );

    Ok(())
}
```

### DLQ Monitoring

Recommended alerts:
- DLQ message count > 0 (indicates processing failures)
- DLQ message rate increasing (indicates systematic issue)
- DLQ age > 7 days (indicates unresolved failures)

## Versioning Strategy

### Schema Versioning

Event schemas use semantic versioning in their contract filenames:

```
<module>-<event-type>.v<major>.json
```

Examples:
- `ar-invoice-issued.v1.json`
- `payments-payment-succeeded.v1.json`
- `gl-posting-request.v1.json`

### Subject Versioning

**Current Strategy**: Subject names do NOT include version numbers.

- Subject: `ar.events.ar.invoice.issued`
- Schema: `ar-invoice-issued.v1.json`

This allows:
- **Backward compatibility**: Publishers can evolve payload structure without changing subjects
- **Consumer flexibility**: Consumers subscribe to subjects, not versions
- **Gradual migration**: Multiple schema versions can coexist on same subject during transitions

### Version Evolution Rules

| Change Type | Requires New Version | Breaking Change |
|-------------|---------------------|-----------------|
| Add optional field | No | No |
| Add required field | Yes | Yes |
| Remove field | Yes | Yes |
| Rename field | Yes | Yes |
| Change field type | Yes | Yes |
| Change field semantics | Yes | Yes |

### Handling Breaking Changes

When introducing breaking changes (v1 → v2):

1. **Dual Publishing** (Transition Period)
   - Publisher emits to BOTH old and new subjects
   - `ar.events.ar.invoice.issued` (v1 format)
   - `ar.events.ar.invoice.issued.v2` (v2 format)

2. **Consumer Migration**
   - Consumers migrate to new subject/schema at their own pace
   - No forced downtime or coordinated deployments

3. **Deprecation**
   - Announce deprecation timeline for old version
   - Monitor consumers of old subject
   - Stop publishing to old subject after grace period (e.g., 90 days)

### Event Envelope Versioning

All events include a `source_version` field indicating the module version that produced the event:

```json
{
  "event_id": "550e8400-e29b-41d4-a716-446655440000",
  "occurred_at": "2026-02-11T23:30:00Z",
  "tenant_id": "tenant_123",
  "source_module": "ar",
  "source_version": "1.2.3",
  "correlation_id": "corr_abc",
  "payload": { ... }
}
```

This enables:
- Debugging which version produced an event
- Tracking migration progress during rollouts
- Identifying version-specific issues

## Idempotency

### Idempotency Key

Every event includes a globally unique `event_id` (UUIDv4) as the idempotency key.

### Consumer Responsibility

Consumers MUST:
1. Check if `event_id` has been processed before handling
2. Store processed `event_id` values in a `processed_events` table
3. Skip processing if `event_id` already exists
4. Handle duplicate events gracefully

### Implementation Pattern

```rust
pub async fn process_event_idempotent<T, F, Fut>(
    consumer: &EventConsumer,
    msg: &BusMessage,
    handler: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    T: DeserializeOwned,
    F: FnOnce(T) -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;
    let event_id = envelope["event_id"].as_str().ok_or("Missing event_id")?;
    let event_id = Uuid::parse_str(event_id)?;

    // Check if already processed
    if consumer.is_processed(event_id).await? {
        tracing::debug!(event_id = %event_id, "Event already processed, skipping");
        return Ok(());
    }

    // Deserialize and handle
    let payload: T = serde_json::from_value(envelope["payload"].clone())?;
    handler(payload).await?;

    // Mark as processed
    consumer.mark_processed(event_id, &msg.subject, "source_module").await?;

    Ok(())
}
```

## Summary Table

| Aspect | Pattern | Example |
|--------|---------|---------|
| **Subject** | `<module>.events.<event-type>` | `ar.events.ar.invoice.issued` |
| **Durable Name** | `<module>-<purpose>-consumer` | `notifications-invoice-notifier-consumer` |
| **DLQ Stream** | `<MODULE>_DLQ` | `AUTH_DLQ` |
| **DLQ Subject** | `<module>.dlq.<event-type>` | `auth.dlq.user.registered` |
| **Schema Version** | `<module>-<event-type>.v<major>.json` | `ar-invoice-issued.v1.json` |
| **Idempotency** | `event_id` (UUID) | `550e8400-e29b-41d4-a716-446655440000` |

## References

- [Event Bus README](platform/event-bus/README.md) - EventBus trait and implementations
- [Event Contracts](contracts/events/README.md) - Envelope standard and event schemas
- [NATS JetStream Documentation](https://docs.nats.io/nats-concepts/jetstream)
- [AR Module Spec](AR-MODULE-SPEC.md) - Example event-driven module design
