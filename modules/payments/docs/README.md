# Payments Module

## Purpose
Owns processor integrations and payment execution for the 7D Solutions Platform.

## Ownership Boundaries
**Owns:**
- processor_customers
- payment_method_refs
- payment_intents
- captures
- refunds
- dispute_records
- webhook_events (verified)

**Does NOT own:**
- Invoices (AR owns invoice state)
- Ledger entries (GL owns ledger)
- Customer master data (AR owns customers)

## Key Principles
- Never mutates AR database
- Never stores raw card data (PCI-DSS scope minimization)
- All processor secrets encrypted at rest
- All webhook events stored for audit
- Idempotent webhook processing

## Architecture References
- **Module Spec:** `docs/architecture/PAYMENTS-MODULE-SPEC.md`
- **OpenAPI Contract:** `contracts/payments/payments-v0.1.0.yaml`
- **Event Schemas:** `contracts/events/payments-*.v1.json`

## Customer-Facing Checkout Session API (bd-ddsm)

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

Polls the session status. For `pending` sessions the handler refreshes status from Tilled live.

**Response:**
```json
{
  "session_id": "<uuid>",
  "status": "pending | succeeded | failed | cancelled",
  "payment_intent_id": "pi_xxx",
  "invoice_id": "inv_abc123",
  "amount": 5000,
  "currency": "usd"
}
```

### POST /api/payments/webhook/tilled

Tilled PSP callback endpoint. Validates the `tilled-signature` HMAC and updates the matching
`checkout_session` status. Idempotent: terminal sessions (succeeded/failed/cancelled) are never
overwritten.

**Handled event types:** `payment_intent.succeeded`, `payment_intent.payment_failed`,
`payment_intent.canceled`.

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
