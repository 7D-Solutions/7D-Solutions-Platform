# E2E Test Failure: Complete Troubleshooting Rundown

## Problem Statement
E2E test fails because invoices are not marked as "paid" after payment processing completes.

## Expected Flow
1. **Subscriptions** module triggers bill run → creates invoice in AR
2. **AR** finalizes invoice → publishes `ar.payment.collection.requested` to NATS
3. **Payments** receives event → processes payment → publishes `payments.payment.succeeded` to NATS
4. **AR** receives payment.succeeded → marks invoice as `paid`
5. **Test assertion**: Verify invoice status is `paid`

## Current Behavior
- Test creates subscription and triggers bill run ✓
- AR creates invoice successfully ✓
- Invoice stays in `pending` status ❌
- Test fails at assertion: invoice never marked as `paid`

## Root Cause Analysis

### What I Found

#### 1. Code is Correct
All business logic, event schemas, and subject naming are correct:
- AR publishes to: `ar.events.ar.payment.collection.requested`
- Payments subscribes to: `ar.events.ar.payment.collection.requested`
- Payments publishes to: `payments.events.payments.payment.succeeded`
- AR subscribes to: `payments.events.payments.payment.succeeded`

#### 2. Database Evidence

**AR Outbox** (events marked as published):
```sql
SELECT event_id, event_type, created_at, published_at
FROM events_outbox
ORDER BY created_at DESC LIMIT 3;

               event_id               |           event_type            |         created_at         |        published_at
--------------------------------------+---------------------------------+----------------------------+----------------------------
 e8664566-3f5a-464f-9bd5-fc0ceb6969c9 | ar.payment.collection.requested | 2026-02-12 13:41:24.034197 | 2026-02-12 13:41:24.881107
```

**Payments Processed Events** (zero events received):
```sql
SELECT COUNT(*) FROM payments_processed_events;
-- Result: 0
```

**Payments Outbox** (zero events emitted):
```sql
SELECT COUNT(*) FROM payments_events_outbox;
-- Result: 0
```

**Conclusion**: AR published events, but Payments never received them.

#### 3. Timing Analysis

Container startup logs show:
```
AR published event:     2026-02-12T13:41:24.881107Z
Payments subscribed:    2026-02-12T13:41:42.995979Z
Time difference:        ~18 seconds
```

**Events were published 18 seconds BEFORE Payments was ready to receive them.**

#### 4. NATS Configuration

JetStream is **enabled** on NATS server:
```
docker logs 7d-nats | grep -i jetstream
[1] 2026/02/12 01:30:02.660430 [INF] Starting JetStream
[1] 2026/02/12 01:30:02.660556 [INF]   Store Directory: "/data/jetstream"
```

Docker compose shows correct configuration:
```yaml
# All services configured with:
BUS_TYPE: nats
NATS_URL: nats://7d-nats:4222
```

#### 5. The Bug

**File**: `platform/event-bus/src/nats_bus.rs`

Despite documentation claiming "NATS JetStream implementation", the code uses **NATS Core pub/sub**:

```rust
// Line 52-55: Uses core NATS publish (fire-and-forget)
async fn publish(&self, subject: &str, payload: Vec<u8>) -> BusResult<()> {
    self.client
        .publish(subject.to_string(), payload.into())  // ← Core NATS, not JetStream!
        .await
        .map_err(|e| BusError::PublishError(e.to_string()))?;
    Ok(())
}

// Line 60-65: Uses core NATS subscribe (no durability)
async fn subscribe(&self, subject: &str) -> BusResult<BoxStream<'static, BusMessage>> {
    let subscriber = self
        .client
        .subscribe(subject.to_string())  // ← Core NATS, not JetStream!
        .await
        .map_err(|e| BusError::SubscribeError(e.to_string()))?;
    // ...
}
```

**NATS Core behavior:**
- Messages are fire-and-forget
- If no subscriber exists when message is published, it's lost forever
- No durability, no replay, no persistence

**What we need (JetStream):**
- Messages stored in durable streams
- Consumers can start at any time and receive messages
- At-least-once delivery guarantees
- Survives restarts and timing issues

## The Fix

Update `platform/event-bus/src/nats_bus.rs` to use JetStream:

1. **Create streams** for each module (AR_EVENTS, PAYMENTS_EVENTS, etc.)
2. **Publish to streams** using `jetstream.publish()` instead of `client.publish()`
3. **Subscribe with durable consumers** using pull/push consumers
4. **Configure retention** and acknowledgment policies

## Technical Details

### Event Envelope Format
All events use this structure:
```json
{
  "event_id": "uuid",
  "event_type": "ar.payment.collection.requested",
  "aggregate_type": "invoice",
  "aggregate_id": "invoice-123",
  "tenant_id": "tenant-123",
  "correlation_id": "uuid",
  "causation_id": "uuid",
  "occurred_at": "2026-02-12T13:41:24.034197Z",
  "payload": {
    "invoice_id": "123",
    "amount_minor": 5000,
    "currency": "USD"
  }
}
```

### Transactional Outbox Pattern
Each module uses outbox pattern for reliable publishing:
1. Business operation + event enqueued in same DB transaction
2. Background publisher polls outbox every 1 second
3. Publishes to NATS and marks as published
4. Idempotency via `processed_events` table on consumer side

### Module Dependencies
```
event-bus (platform/event-bus)
├── async-nats = "0.33"  ← Supports JetStream
├── Used by: AR, Payments, Subscriptions, Notifications
└── Interface: EventBus trait
```

### Test Environment
- Docker Compose with separate containers per module
- Each module has dedicated PostgreSQL database
- Shared NATS server with JetStream enabled
- RUST_LOG=info for all services

## Files Involved

### Core Event Bus
- `platform/event-bus/src/lib.rs` - EventBus trait definition
- `platform/event-bus/src/nats_bus.rs` - **BUG IS HERE** ← needs JetStream
- `platform/event-bus/src/inmemory_bus.rs` - In-memory implementation (dev/test)

### AR Module
- `modules/ar/src/events/publisher.rs` - Background publisher (polls outbox)
- `modules/ar/src/events/outbox.rs` - Transactional outbox operations
- `modules/ar/src/consumer_tasks.rs` - Consumes payment.succeeded events
- `modules/ar/src/models.rs` - PaymentSucceededPayload model

### Payments Module
- `modules/payments/src/consumer_task.rs` - Consumes ar.payment.collection.requested
- `modules/payments/src/events/publisher.rs` - Publishes payment.succeeded

### Test
- `e2e-tests/tests/real_e2e.rs` - Integration test that fails

## Reproduction Steps
```bash
# 1. Start infrastructure
docker compose -f docker-compose.infrastructure.yml up -d

# 2. Build and start modules
docker compose -f docker-compose.modules.yml build
docker compose -f docker-compose.modules.yml up -d

# 3. Wait for healthy state
docker ps  # All containers should show (healthy)

# 4. Run test
cd e2e-tests
cargo test real_e2e -- --nocapture

# Expected: ❌ FAIL - Invoice not marked as paid
# After fix: ✅ PASS - Invoice status changes to paid
```

## Verification Commands

Check if Payments received AR events:
```bash
docker exec -i 7d-payments-postgres psql -U payments_user -d payments_db \
  -c "SELECT COUNT(*) FROM payments_processed_events;"
# Current: 0 (should be > 0 after fix)
```

Check if Payments emitted payment.succeeded:
```bash
docker exec -i 7d-payments-postgres psql -U payments_user -d payments_db \
  -c "SELECT COUNT(*) FROM payments_events_outbox;"
# Current: 0 (should be > 0 after fix)
```

Check AR outbox:
```bash
docker exec -i 7d-ar-postgres psql -U ar_user -d ar_db \
  -c "SELECT event_type, published_at FROM events_outbox ORDER BY created_at DESC LIMIT 3;"
# Shows events were marked as published
```

Check invoice status:
```bash
docker exec -i 7d-ar-postgres psql -U ar_user -d ar_db \
  -c "SELECT id, tenant_id, status FROM ar_invoices ORDER BY created_at DESC LIMIT 3;"
# Current: status='pending' (should be 'paid' after fix)
```

## Next Steps
1. Implement JetStream in `platform/event-bus/src/nats_bus.rs`
2. Rebuild all module containers: `docker compose -f docker-compose.modules.yml build`
3. Restart containers: `docker compose -f docker-compose.modules.yml up -d`
4. Re-run e2e test to verify fix
5. Commit changes with bead ID prefix

## Additional Context
- Using Rust with Tokio async runtime
- SQLx for database operations with compile-time query checking
- Axum web framework for REST APIs
- Tracing for structured logging
- All services use workspace-aware Dockerfiles for faster builds
