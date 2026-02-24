# Payments Module

## Purpose
Owns payment execution, PSP integration, and checkout session flows for the 7D Solutions Platform.

## Ownership Boundaries
**Owns:**
- `payment_attempts` — deterministic attempt ledger with exactly-once enforcement
- `checkout_sessions` — customer-facing Tilled.js payment flow
- `payments_events_outbox` — transactional outbox for reliable event publishing
- `payments_processed_events` — idempotent event consumption tracking
- `failed_events` — Dead Letter Queue for events that fail after all retries

**Does NOT own:**
- Invoices (AR owns invoice state)
- Ledger entries (GL owns ledger)
- Customer master data (AR owns customers)
- Raw card data (Tilled.js handles PCI scope)

## Key Principles
- Never mutates AR database
- Never stores raw card data (PCI-DSS scope minimization)
- PSP is an implementation detail — product apps interact with checkout sessions
- Idempotent webhook processing (conditional UPDATE, terminal states never overwritten)
- Guard → Mutate → Emit pattern for all state transitions
- UNKNOWN protocol: ambiguous PSP results block retries until reconciled

## Architecture References
- **Vision Doc:** `modules/payments/docs/PAYMENTS-MODULE-SPEC.md`
- **Legacy Spec:** `docs/architecture/PAYMENTS-MODULE-SPEC.md`
- **OpenAPI Contract:** `contracts/payments/payments-v1.0.0.yaml`
- **Event Schemas:** `contracts/events/payments-payment-succeeded.v1.json`, `contracts/events/payments-payment-failed.v1.json`

## Customer-Facing Checkout Session API

Product apps (e.g. TrashTech) never call Tilled directly. They use these endpoints to create
a Tilled.js-compatible payment flow.

### POST /api/payments/checkout-sessions

Creates a Tilled PaymentIntent in `requires_payment_method` state and returns a `client_secret`
that the browser passes to `tilled.js confirmPayment()`.

**Request:**
```json
{
  "invoice_id": "inv_abc123",
  "tenant_id": "tenant_xyz",
  "amount": 5000,
  "currency": "usd",
  "return_url": "https://app.example.com/payment/success",
  "cancel_url":  "https://app.example.com/payment/cancel"
}
```

**Response (201):**
```json
{
  "session_id": "<uuid>",
  "payment_intent_id": "pi_xxx",
  "client_secret": "pi_xxx_secret_yyy"
}
```

### GET /api/payments/checkout-sessions/:id

Full session data including client_secret. For non-terminal sessions, polls Tilled for live status.

### POST /api/payments/checkout-sessions/:id/present

Idempotent: transitions `created` → `presented` on hosted page load.

### GET /api/payments/checkout-sessions/:id/status

Lightweight status poll (no client_secret exposed).

### POST /api/payments/webhook/tilled

Tilled PSP callback endpoint. Validates the `tilled-signature` HMAC and updates the matching
`checkout_session` status. Idempotent: terminal sessions (completed/failed/canceled) are never
overwritten.

**Handled event types:** `payment_intent.succeeded`, `payment_intent.payment_failed`,
`payment_intent.canceled`.

**Checkout session state machine:** `created → presented → completed | failed | canceled | expired`

### Provider behaviour

| `PAYMENTS_PROVIDER` | Behaviour |
|---------------------|-----------|
| `mock` (default)    | Generates fake `mock_pi_*` IDs — no network calls |
| `tilled`            | Calls `api.tilled.com` — requires `TILLED_API_KEY` + `TILLED_ACCOUNT_ID` |

## Development
```bash
cargo check
cargo test
cargo run
```

Default port: 8088
