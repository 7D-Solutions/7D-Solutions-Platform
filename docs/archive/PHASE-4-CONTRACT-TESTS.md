# Phase 4 — Contract Tests + Golden Examples (No Feature Logic)

## Objective
Harden the platform by adding:
1) **Golden example payloads** for every event schema
2) **Executable contract tests** for:
   - Event schemas (producer conformance)
   - Event consumer acceptance + idempotency (minimal)
   - OpenAPI spec sanity (parsing + minimal endpoint presence)
3) **CI integration** to run contract tests for each module

This phase introduces **validation only**. No business workflows.

---

## Scope
Applies to:
- `contracts/events/*.json` (event schemas)
- `contracts/*/*.yaml` (OpenAPI specs)
- `modules/ar` (consumer contract tests for Payments events)
- `modules/payments`, `modules/notifications` (schema conformance tests)
- `modules/subscriptions` (OpenAPI contract tests only for now; no events in v0.1)

---

## Deliverables

### A) Golden Examples
Create:
- `contracts/events/examples/` directory
- One `.json` golden example **per event schema**:
  - `payments-payment-succeeded.v1.example.json`
  - `payments-payment-failed.v1.example.json`
  - `payments-refund-succeeded.v1.example.json`
  - `payments-refund-failed.v1.example.json`
  - `notifications-delivery-succeeded.v1.example.json`
  - `notifications-delivery-failed.v1.example.json`
  - (Optional: subscriptions event examples already exist? If not, add them too.)

Each example must:
- Conform to the envelope standard in `contracts/events/README.md`
- Conform to the specific event schema
- Use realistic identifiers and values
- Use `amount_minor` integers and `currency` ISO codes

### B) Contract Test Harness (Rust)
Create a small shared internal test helper crate:
- `tools/contract-tests/` (Rust crate)
Purpose:
- Load JSON schema files
- Load example JSON files
- Validate examples against schemas
- Run in CI

This crate MUST NOT be used as a runtime dependency by modules.
It is test/tooling only.

### C) Module-Level Tests
Add minimal tests:

**Payments module**
- Validates each payments golden example against its schema

**Notifications module**
- Validates each notification golden example against its schema

**AR module**
- Validates it can parse/accept payments events:
  - Ensure JSON examples parse into expected internal shape (or generic Value)
  - Enforce idempotency rule at least at the "seen event_id" layer (can be in-memory for test)
  - Ensure forbidden transitions are rejected (stubbed or minimal)

### D) OpenAPI Contract Sanity
Add a script/tool check:
- Parse each OpenAPI yaml
- Verify minimum endpoints exist:
  - Payments: `/health`, `/payment-methods`, `/refunds`
  - Notifications: `/health`, `/notifications/send`, `/notifications/{id}`
  - Subscriptions: `/health`, `/subscriptions`, `/bill-runs/execute`

### E) CI Wiring
Update `.github/workflows/ci.yml` to add one job:
- `contract-tests`
Which runs:
- schema validation tests
- OpenAPI sanity parsing
This job must run before build jobs.

---

## Non-goals
- No implementation of event bus consumption
- No processor integrations
- No email/SMS providers
- No subscription billing logic
- No new modules
- No refactors

---

## Acceptance Criteria
Phase 4 is complete when:
- Every event schema has a validated golden example in `contracts/events/examples/`
- `cargo test` runs contract tests successfully
- CI runs contract tests successfully
- No module boundaries are violated
