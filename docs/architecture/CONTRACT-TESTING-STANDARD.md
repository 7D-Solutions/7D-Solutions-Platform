# Contract Testing Standard

## Purpose
Define minimum required tests to ensure contract-driven integration remains stable as modules evolve.

This standard applies to:
- OpenAPI contracts under `contracts/*`
- Event schemas under `contracts/events/*`

---

## Definitions
- **Contract validation:** parsing + schema validation (syntax/structure)
- **Contract compatibility:** detecting breaking changes vs previous version
- **Contract tests:** executable tests that ensure:
  - servers conform to OpenAPI
  - clients can call endpoints successfully using generated types
  - event producers emit valid schemas
  - event consumers accept valid schemas and reject invalid ones

---

## Minimum Requirements (Per Module Release)
Each module release MUST include:

### A) OpenAPI Validation
- Spec parses
- All schemas compile
- Example payloads (at least one per primary endpoint)

### B) OpenAPI Conformance Tests (smoke)
- Start module server
- Call `/health`
- Call at least 1 endpoint per resource group with a valid request
- Ensure response matches OpenAPI schema (best-effort)
  - status codes correct
  - required fields present

### C) Event Schema Validation
- All produced events validate against schema
- At least one golden example JSON file per produced event:
  - `contracts/events/examples/<event-name>.json`

### D) Event Consumer Contract Tests
For each consumed event:
- Provide a valid example
- Provide an invalid example (missing required field)
- Consumer must:
  - accept valid and ack
  - reject invalid and log
  - be idempotent on duplicates (same event_id)

---

## Compatibility Rules (Breaking vs Non-breaking)

### OpenAPI
Breaking changes include:
- removing endpoints
- removing required fields
- changing field types
- narrowing enum values
- changing auth requirements

Non-breaking changes:
- adding optional fields
- adding new endpoints

### Event Schemas
Breaking changes include:
- removing required fields
- changing field types/meaning
- changing event semantics

Non-breaking:
- adding optional fields
- adding new event types

---

## CI Expectations (Phased)
Current baseline CI validates parsing. The target CI should also include:
1) Parse + validate contracts (already exists)
2) Run contract tests (new)
3) Run compatibility checks against last released version (new)

---

## Deliverables Checklist
For a module version bump:
- Updated OpenAPI spec
- Updated event schemas (if applicable)
- Test updates demonstrating conformance
- Changelog entry describing contract impact
