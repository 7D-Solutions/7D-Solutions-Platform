# Idempotency and Event Logging

## Overview

The AR (Accounts Receivable) system implements two critical reliability features:

1. **Idempotency Keys**: Prevent duplicate operations from being executed multiple times
2. **Event Logging**: Provides a complete forensics trail of all actions in the system

## Idempotency

### How It Works

Idempotency ensures that retrying the same request multiple times has the same effect as making it once. This is critical for:
- Network retry scenarios
- User double-clicks
- Failed requests that were actually processed
- Webhook deliveries

### Usage

Send an `Idempotency-Key` header with any write operation (POST, PUT, DELETE):

```bash
curl -X POST https://api.example.com/api/ar/customers \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: unique-key-12345" \
  -d '{
    "email": "customer@example.com",
    "name": "John Doe"
  }'
```

### Behavior

1. **First Request**:
   - Request is processed normally
   - Response is cached for 24 hours
   - Returns `201 Created` (or appropriate status)

2. **Duplicate Request** (same `Idempotency-Key`):
   - Cached response is returned immediately
   - No duplicate database records created
   - Returns same status code as original request

3. **No Idempotency Key**:
   - Request processed normally
   - No caching occurs
   - Duplicate requests will create duplicate records

### Implementation Details

- **Storage**: `billing_idempotency_keys` table
- **TTL**: 24 hours (configurable)
- **Key Format**: Any string (recommended: UUID or timestamp-based)
- **Request Hash**: SHA-256 hash of idempotency key + request body
- **Scope**: Per app_id (multi-tenant safe)

### Best Practices

1. **Generate Unique Keys**: Use UUIDs or combination of timestamp + entity ID
2. **Use for All Write Operations**: POST, PUT, DELETE, PATCH
3. **Don't Reuse Keys**: Each unique operation should have its own key
4. **Client-Side Generation**: Generate keys on the client to survive retries

### Example: Safe Retry

```javascript
async function createCustomer(customerData) {
  // Generate idempotency key once
  const idempotencyKey = `create-customer-${Date.now()}-${Math.random()}`;

  // Retry logic with same key
  for (let i = 0; i < 3; i++) {
    try {
      const response = await fetch('/api/ar/customers', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Idempotency-Key': idempotencyKey,
        },
        body: JSON.stringify(customerData),
      });

      if (response.ok) {
        return await response.json();
      }
    } catch (error) {
      // Network error - retry with same key
      if (i === 2) throw error;
      await sleep(1000 * (i + 1)); // Exponential backoff
    }
  }
}
```

## Event Logging

### Purpose

Event logging provides:
- **Audit Trail**: Complete history of who did what and when
- **Debugging**: Forensic analysis of issues
- **Analytics**: Understanding system usage patterns
- **Compliance**: Meeting regulatory requirements

### Event Structure

```json
{
  "id": 123,
  "app_id": "your-app-id",
  "event_type": "customer.created",
  "source": "api",
  "entity_type": "customer",
  "entity_id": "456",
  "payload": {
    "email": "customer@example.com",
    "name": "John Doe",
    "status": "active"
  },
  "created_at": "2026-02-10T12:34:56Z"
}
```

### Event Types

Events follow the pattern: `{entity}.{action}`

**Customer Events:**
- `customer.created`
- `customer.updated`
- `customer.deleted`

**Subscription Events:**
- `subscription.created`
- `subscription.updated`
- `subscription.canceled`
- `subscription.paused`
- `subscription.resumed`

**Invoice Events:**
- `invoice.created`
- `invoice.finalized`
- `invoice.paid`
- `invoice.voided`

**Charge Events:**
- `charge.created`
- `charge.succeeded`
- `charge.failed`
- `charge.refunded`

**Webhook Events:**
- `webhook.received`
- `webhook.processed`
- `webhook.failed`

### Event Sources

- `api`: Events triggered by API calls
- `webhook`: Events triggered by Tilled webhooks
- `system`: Events triggered by system processes (cron jobs, etc.)

### Querying Events

#### Get All Events

```bash
GET /api/ar/events
```

#### Filter by Entity

```bash
GET /api/ar/events?entity_id=456&entity_type=customer
```

#### Filter by Event Type

```bash
GET /api/ar/events?event_type=customer.created
```

#### Filter by Source

```bash
GET /api/ar/events?source=webhook
```

#### Time Range Query

```bash
GET /api/ar/events?start=2026-02-01T00:00:00Z&end=2026-02-10T23:59:59Z
```

#### Pagination

```bash
GET /api/ar/events?limit=50&offset=100
```

#### Complex Query

```bash
GET /api/ar/events?entity_id=456&event_type=customer.updated&start=2026-02-01T00:00:00Z&limit=10
```

### Response Format

```json
[
  {
    "id": 789,
    "app_id": "your-app-id",
    "event_type": "customer.created",
    "source": "api",
    "entity_type": "customer",
    "entity_id": "456",
    "payload": {
      "email": "customer@example.com",
      "name": "John Doe"
    },
    "created_at": "2026-02-10T12:34:56Z"
  }
]
```

### Get Single Event

```bash
GET /api/ar/events/789
```

### Implementation for Developers

Events are logged automatically for key operations. To add event logging to a new endpoint:

```rust
use crate::idempotency::log_event_async;

// After successful operation
log_event_async(
    db.clone(),
    app_id.to_string(),
    "customer.created".to_string(),  // Event type
    "api".to_string(),                // Source
    Some("customer".to_string()),     // Entity type
    Some(customer.id.to_string()),    // Entity ID
    Some(serde_json::to_value(&customer).unwrap_or_default()), // Payload
);
```

### Event Retention

- Events are **never deleted** (append-only log)
- Events are stored in PostgreSQL
- Consider archiving old events to cold storage for long-term retention
- Query performance maintained via indexes on `app_id`, `event_type`, `source`, and `created_at`

## Database Tables

### billing_idempotency_keys

```sql
CREATE TABLE billing_idempotency_keys (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    request_hash VARCHAR(64) NOT NULL,
    response_body JSONB NOT NULL,
    status_code INTEGER NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL,
    CONSTRAINT unique_app_idempotency_key UNIQUE (app_id, idempotency_key)
);
```

### billing_events

```sql
CREATE TABLE billing_events (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    source VARCHAR(20) NOT NULL,
    entity_type VARCHAR(50),
    entity_id VARCHAR(255),
    payload JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

## Testing

### Test Idempotency

```bash
# First request
curl -X POST http://localhost:8086/api/ar/customers \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: test-key-001" \
  -d '{"email": "test@example.com", "name": "Test User"}'

# Duplicate request (should return same response, no new record)
curl -X POST http://localhost:8086/api/ar/customers \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: test-key-001" \
  -d '{"email": "test@example.com", "name": "Test User"}'
```

### Test Event Logging

```bash
# Create a customer
curl -X POST http://localhost:8086/api/ar/customers \
  -H "Content-Type: application/json" \
  -d '{"email": "test@example.com", "name": "Test User"}'

# Query events
curl http://localhost:8086/api/ar/events?event_type=customer.created
```

## Monitoring

### Idempotency Key Metrics

- **Cache Hit Rate**: Percentage of requests with duplicate idempotency keys
- **Expired Keys**: Keys that have passed their TTL
- **Key Collisions**: Different requests with same key (should be zero)

### Event Log Metrics

- **Events Per Second**: Rate of event creation
- **Events By Type**: Distribution of event types
- **Events By Source**: API vs webhook vs system events
- **Event Query Performance**: Time to query events

## Troubleshooting

### Idempotency Key Not Working

1. **Check Header**: Ensure `Idempotency-Key` header is set correctly
2. **Check TTL**: Keys expire after 24 hours
3. **Check app_id**: Idempotency is scoped per app_id
4. **Check Request Body**: Request body must match exactly

### Events Not Logging

1. **Check Database**: Ensure `billing_events` table exists
2. **Check Async Execution**: Events log asynchronously (may have slight delay)
3. **Check Database Errors**: Look for constraint violations or connection issues
4. **Check Event Code**: Ensure `log_event_async` is called after successful operations

## Security Considerations

1. **Event Payloads**: May contain sensitive data - restrict access to events endpoint
2. **Idempotency Keys**: Can be used to infer system behavior - treat as sensitive
3. **Rate Limiting**: Apply rate limits to prevent abuse of idempotency cache
4. **Access Control**: Implement proper authentication/authorization for event queries
