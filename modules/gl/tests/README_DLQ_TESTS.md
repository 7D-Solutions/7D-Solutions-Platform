# GL Module DLQ Behavior Tests

## Overview

The `dlq_behavior_test.rs` file contains comprehensive tests for the Dead Letter Queue (DLQ) functionality in the GL module's event consumer.

## Purpose

These tests validate that:
1. **Invalid events are captured**: Malformed or validation-failing GL posting requests are properly moved to the `failed_events` table
2. **Observability is complete**: All DLQ entries include correlation_id, tenant_id, and detailed error reasons for debugging
3. **No panics on bad input**: The consumer gracefully handles garbage data without crashing
4. **Context preservation**: Full event envelopes are stored for replay/debugging

## Test Scenarios

### 1. `test_dlq_captures_missing_required_field`
- **Trigger**: Publish event missing required `description` field
- **Expected**: Event in DLQ with deserialization error
- **Validates**: Schema validation, error capture, metadata preservation

### 2. `test_dlq_captures_unbalanced_entry`
- **Trigger**: Publish event with debits != credits ($100 debit, $50 credit)
- **Expected**: Event in DLQ with validation error
- **Validates**: Business logic validation, balanced entry enforcement

### 3. `test_dlq_captures_invalid_currency`
- **Trigger**: Publish event with lowercase currency code ("usd" instead of "USD")
- **Expected**: Event in DLQ with currency validation error
- **Validates**: Currency format validation, tenant_id capture

### 4. `test_dlq_handles_garbage_json_without_panic`
- **Trigger**: Publish completely invalid JSON payload
- **Expected**: Consumer continues running (no panic)
- **Validates**: Resilience to malformed messages, graceful error handling

### 5. `test_dlq_captures_empty_account_ref`
- **Trigger**: Publish event with empty account_ref string
- **Expected**: Event in DLQ with validation error
- **Validates**: Account reference validation

## Running the Tests

```bash
# Run all DLQ tests (requires GL database)
cargo test --package gl-rs --test dlq_behavior_test -- --ignored

# Run a specific test
cargo test --package gl-rs --test dlq_behavior_test test_dlq_captures_unbalanced_entry -- --ignored
```

**Prerequisites:**
- GL PostgreSQL database running on port 5438
- Database URL: `postgres://postgres:postgres@localhost:5438/gl_test`
- Tests use `InMemoryBus` (no NATS required for these tests)

## Test Architecture

- **Database**: Uses `gl_test` database on port 5438
- **Event Bus**: Uses `InMemoryBus` for isolation (no external NATS dependency)
- **Consumer**: Starts actual GL posting consumer with retry logic
- **Isolation**: Tests use `#[serial]` to prevent concurrent execution
- **Cleanup**: Each test cleans up DLQ entries after completion

## Observability Assertions

Each test verifies that DLQ entries include:

1. **event_id**: Unique identifier for idempotency
2. **subject**: NATS subject where event was published
3. **tenant_id**: Multi-tenant isolation key
4. **correlation_id**: Distributed tracing correlation
5. **error**: Detailed error message explaining failure
6. **retry_count**: Number of retries attempted before DLQ
7. **envelope**: Full JSON payload for replay/debugging

## Implementation Notes

- **Non-retriable errors**: Validation errors go straight to DLQ (no retries)
- **Retriable errors**: Database errors retry with exponential backoff (default: 3 attempts)
- **No PII logging**: Tests avoid capturing sensitive data in logs
- **Graceful degradation**: Consumer continues processing other events after DLQ write

## Quality Gates (ChatGPT-mandated)

✅ Test publishes malformed/invalid event, lands in failed_events
✅ Logs include correlation_id, tenant_id, error reason
✅ Tracing spans propagate correctly
✅ No panics on bad input
✅ DLQ captures enough context for debugging
✅ No PII/sensitive data in logs

## Related Files

- Consumer implementation: `modules/gl/src/consumer/gl_posting_consumer.rs`
- DLQ handler: `modules/gl/src/dlq.rs`
- Failed events repository: `modules/gl/src/repos/failed_repo.rs`
- Contract validation: `modules/gl/src/validation.rs`
