# Contract Examples Standard (Golden Files)

## Purpose
Golden examples are immutable payload samples used to:
- validate event schemas
- validate consumers and producers
- support docs and debugging
- prevent silent schema drift

---

## Location
- `contracts/events/examples/`

---

## Naming
Use:
- `<schema-filename>.example.json`

Example:
- `payments-payment-succeeded.v1.example.json`

---

## Requirements (Events)
Each event example must:
- include the full envelope per `contracts/events/README.md`
- include realistic identifiers
- include `tenant_id`
- include `source_module` and `source_version`
- include `occurred_at` RFC3339 timestamp
- include `payload` matching the event schema

---

## Requirements (OpenAPI)
OpenAPI examples (if present) should:
- use integer minor units for money
- include required headers for auth if defined
- include realistic error payloads

---

## Versioning
If an event schema becomes v2:
- create a new `*.v2.example.json`
- keep v1 examples unchanged

Examples are **append-only**, never "updated" to match new meaning.
