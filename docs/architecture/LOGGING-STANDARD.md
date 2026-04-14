# Structured Logging Standard

**Status:** Active  
**Bead:** bd-5ea4y  
**Applies to:** All platform modules (`modules/*`)

---

## Required Fields by Log Level

Every log event emitted in an HTTP request context **must** include the fields listed below.
Fields marked _optional_ are recorded when available but are not required.

| Field        | ERROR | WARN | INFO | DEBUG |
|--------------|-------|------|------|-------|
| `message`    | ✓ required | ✓ required | ✓ required | ✓ required |
| `tenant_id`  | ✓ required | ✓ required | ✓ required | — optional |
| `request_id` | ✓ required | ✓ required | ✓ required | — optional |
| `actor_id`   | ✓ required | — optional | — optional | — |
| `module`     | ✓ required | ✓ required | ✓ required | — optional |
| `error_code` | ✓ required | — | — | — |
| `file`       | ✓ required | — | — | — |
| `line`       | ✓ required | — | — | — |

> **Note:** `file` and `line` are automatically populated by tracing's `%file` and `%line` span
> fields when the log event is recorded at the call site. You do not need to add them manually.

---

## How Required Fields Are Injected

### HTTP request handlers (automatic)

The `platform_trace_middleware` in `platform-sdk` creates a tracing span at the start of every
request that includes:

```
tenant_id, actor_id, request_id, trace_id, correlation_id, method, path
```

All `tracing::info!`, `tracing::warn!`, and `tracing::error!` calls inside a handler inherit
these fields automatically from the parent span. **No manual field injection is needed** in
handler code.

The middleware runs after JWT auth, so `tenant_id` and `actor_id` are populated from
`VerifiedClaims`. For unauthenticated routes they are empty strings.

### Event consumers and background tasks (manual)

Code outside the HTTP stack (NATS consumers, scheduled jobs, outbox publisher) does not go
through `platform_trace_middleware`. Use `ctx.log_span()` to create an equivalent span:

```rust
// In a consumer or background task:
let span = ctx.log_span(&tenant_id.to_string(), &request_id, &actor_id.to_string());
let _guard = span.enter();

tracing::info!(event = "inventory.adjusted", "stock level updated");
// ↑ Automatically includes tenant_id, request_id, actor_id, module
```

Or instrument an `async` block:

```rust
use tracing::Instrument as _;

async move {
    tracing::info!("processing event");
}.instrument(ctx.log_span(&tenant_id, &request_id, &actor_id)).await;
```

---

## Examples

### Correct: ERROR with required fields

The span created by the middleware (or `ctx.log_span()`) supplies the required fields.
The caller only needs to add `error_code`:

```rust
tracing::error!(
    error_code = "INVENTORY_INSUFFICIENT",
    error = %e,
    "cannot allocate stock: quantity unavailable"
);
```

Output (JSON format):
```json
{
  "level": "ERROR",
  "message": "cannot allocate stock: quantity unavailable",
  "tenant_id": "abc-123",
  "request_id": "7a3f1b...",
  "actor_id": "user-456",
  "module": "inventory",
  "error_code": "INVENTORY_INSUFFICIENT",
  "file": "src/http/adjustments.rs",
  "line": 84
}
```

### Correct: WARN with required fields

```rust
tracing::warn!(
    count = stale_count,
    "stale cache entries detected — background refresh triggered"
);
```

### Incorrect: missing required fields outside a span

```rust
// ❌ BAD — called outside any span; tenant_id and request_id are absent
tracing::error!("allocation failed: {}", e);

// ✅ GOOD — inside middleware-created span or ctx.log_span()
tracing::error!(error_code = "ALLOC_FAILED", error = %e, "allocation failed");
```

---

## CI Enforcement

`tools/ci/check-log-fields.sh` scans HTTP handler files for bare `tracing::error!` or
`tracing::warn!` calls that lack `error_code` or other structured fields. It does not
enforce the span context (which is structural, not textual) but catches unstructured one-liner
log calls in handler code.

Run locally:
```bash
bash tools/ci/check-log-fields.sh
```

---

## Cross-Module Audit (Compliance)

Because `tenant_id` and `request_id` flow through the structured log, a query against the
aggregated log store (e.g. Loki) can reconstruct all actions by a given actor:

```logql
{service="inventory"} | json | tenant_id="abc-123" | actor_id="user-456"
```

This satisfies the "show all actions by user X" requirement without manual grep.
