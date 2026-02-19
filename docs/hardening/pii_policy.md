# PII Policy — Log Redaction & Data Handling

## Overview

This runbook defines how Personally Identifiable Information (PII) is handled
across all platform modules. The goal is to prevent PII leakage through logs,
metrics, error messages, and distributed traces.

## PII Field Inventory

The following fields are classified as PII and must never appear verbatim in
logs, metrics labels, or tracing spans.

| Category     | Fields                                                   |
|--------------|----------------------------------------------------------|
| Identity     | `email`, `name`, `phone`, `date_of_birth`                |
| Financial    | `card_number`, `account_number`, `routing_number`        |
| Tax / Legal  | `ssn`, `tax_id`, `vat_number`, `ein`                     |
| Address      | `street`, `city`, `state`, `postal_code`, `country`      |
| Credentials  | `password`, `secret`, `api_key`, `token` (full values)   |

**Safe to log:** internal IDs (`customer_id`, `invoice_id`, `tenant_id`, `app_id`),
status codes, metric counts, durations, event types.

## Redaction Helpers

All redaction utilities live in `platform/security/src/redaction.rs`.

### `Redacted<T>` — opaque wrapper

Wrap any sensitive value so it cannot leak through `Debug` or `Display`:

```rust
use security::redaction::Redacted;

let email = Redacted(customer.email.clone());
tracing::info!(email = %email, "processing customer"); // logs: email = [REDACTED]
```

The inner value remains fully accessible via `.inner()` or `.into_inner()` when
you actually need it (e.g. to send the email).

### `redact_email(email: &str) -> String`

Preserves the domain for audit context while masking the local part:

```
alice@example.com  →  [redacted]@example.com
```

Use this when you need a partially-masked representation in audit events.

### `redact_partial(value: &str, visible: usize) -> String`

Masks all but the last `visible` characters — useful for card numbers:

```
4111111111111234  →  XXXXXXXXXXXX1234
```

### `redact_name(name: &str) -> String`

Reduces a name to initials for audit context:

```
Alice Bob  →  A. B.
```

## Module-Specific Rules

### AR (Accounts Receivable)

- `Customer.email` and `Customer.name` — wrap in `Redacted` before logging.
- Error messages about duplicate emails must not include the email value.
- Internal IDs (`id`, `app_id`, `external_customer_id`) are safe to log.

**Example (AR customers route):**
```rust
// BAD — leaks email in error path
tracing::error!("Duplicate email: {}", email);

// GOOD — log error without PII
tracing::error!(app_id = %app_id, "Failed to check duplicate email");
```

### Notifications

- `recipient_email` must never appear in log fields.
- Log `notification_id`, `channel`, and `template` — these are safe identifiers.
- Delivery failure logs: include `notification_id` and error code, not the address.

### Payments

- `card_number`, `bank_account_number` — never log. Use masked versions via
  `redact_partial` if audit context is required.
- `tilled_customer_id`, `payment_id` — safe to log (opaque provider IDs).

### Subscriptions / Timekeeping / Treasury

- No free-form user data fields should appear in log statements.
- `tenant_id`, `subscription_id`, `invoice_id` are safe operational identifiers.

## Metrics Labels

Metric label cardinality must never include PII values. Allowed label values:

- Status codes: `200`, `404`, `500`
- Route templates: `/api/ar/customers/:id` (not the actual ID value)
- Outcome labels: `success`, `failure`, `duplicate`
- Module names: `ar`, `payments`, `notifications`

Never use `customer_email`, `user_name`, or tenant-specific strings as label values.

## Audit Events vs. Logs

Structured audit records stored in the `audit_db` are subject to different rules
than operational logs:

| Concern         | Operational Logs   | Audit Events          |
|----------------|--------------------|-----------------------|
| Storage         | Time-limited (30d) | Long-term (7 years)   |
| PII allowed?    | No                 | Yes, encrypted at rest|
| Redaction rule  | Always redact      | Store but protect     |

Audit events may contain actor identity and resource identifiers because they are
covered by access control. Operational logs (stdout / tracing) are not.

## Enforcement

1. **Code review:** PRs that add log statements containing PII field names
   (`email`, `name`, `phone`, etc.) must be rejected.
2. **Test coverage:** The `security::redaction` module has unit tests confirming
   that `Redacted<T>` and helper functions never emit raw PII. Run with:
   ```bash
   ./scripts/cargo-slot.sh test -p security -- redaction
   ```
3. **Log scanning:** Run the following to detect potential PII in log statements:
   ```bash
   grep -rn 'info!\|warn!\|error!\|debug!' modules/*/src/ \
     --include='*.rs' \
     | grep -Ei '(email|phone|\.name|address|ssn|tax_id|card_number|password|secret)'
   ```
   Any matches should be reviewed and wrapped in `Redacted` or removed.

## Incident Response

If PII is discovered in logs:

1. **Contain:** Identify the log stream and truncate/rotate if possible.
2. **Fix:** Add `Redacted<T>` wrapper or remove the log statement.
3. **Notify:** Alert the security lead and tenant data owner within 24 hours.
4. **Document:** Add an entry to the incident log with fields: date, module,
   field name, estimated exposure window, remediation commit SHA.

## References

- `platform/security/src/redaction.rs` — helper implementations
- `docs/hardening/stabilization_gate.md` — overall hardening gate criteria
- GDPR Article 32 — security of processing
- SOC 2 CC6.1 — logical access controls
