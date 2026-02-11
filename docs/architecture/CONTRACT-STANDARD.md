# Contract Standard — OpenAPI + Event Schemas

## Purpose
Contracts are the source of truth for module integration.
No module may rely on another module's source code or database.
Integration is **contract-driven** only.

This standard defines:
- where contracts live
- how they are versioned
- what constitutes breaking change
- minimum documentation and testing requirements

---

## Contract Types
### OpenAPI (sync)
- REST APIs are specified in OpenAPI 3.x YAML
- One primary OpenAPI spec per module version

### Event Schemas (async)
- Events are specified as JSON schemas (and/or AsyncAPI where adopted)
- All events use the standard envelope defined in `contracts/events/README.md`

---

## Location & Ownership
- OpenAPI contracts live under `contracts/<module>/` (or the repo's established convention)
- Event schemas live under `contracts/events/`
- The module that owns a domain prefix owns the schemas for that domain.
  - `ar.*` is owned by AR
  - `payments.*` is owned by Payments
  - `subscriptions.*` is owned by Subscriptions
  - `notifications.*` is owned by Notifications
  - `gl.*` is owned by GL

No module may publish contracts under another module's domain prefix.

---

## Versioning Rules
### Module versioning
Each module is independently versioned using SemVer:
- MAJOR: breaking changes
- MINOR: backward compatible changes
- PATCH: fixes with no contract change

### Contract versioning
- OpenAPI file naming should reflect module major/minor (repo convention)
- Event schema files include `.v{N}` for major versions:
  - `ar-invoice-issued.v1.json`
  - `ar-invoice-issued.v2.json`

---

## Breaking Changes
### OpenAPI breaking changes
- remove/rename endpoint
- remove required field
- change field type
- change meaning of field
- narrow enum values
- change auth requirements
- change status code semantics

### Event breaking changes
- remove required field
- change type/meaning of a field
- change event semantics or required consumer behavior

Breaking change requires:
- major version bump (module + schema)
- changelog entry
- migration/rollout notes

---

## Non-breaking Changes
- add optional fields
- add new endpoints
- add new events
- add new enum values only if consumers treat unknown values safely (explicitly documented)

---

## Required Contents
For each contract:
- Description of purpose and owning module
- Stable identifiers for key entities (IDs)
- Example payloads (at least one)
- Error codes/taxonomy (OpenAPI) or failure semantics (events)

---

## Testing Requirements
See `docs/architecture/CONTRACT-TESTING-STANDARD.md`.
At minimum:
- parsing/validation in CI
- smoke conformance tests per module
- event producer/consumer schema tests
- idempotency tests

---

## Contract-first Development Rule
Any cross-module work must follow:
1) Write/modify contract
2) Validate contract
3) Generate clients/types (if used)
4) Implement server/producer/consumer
5) Add contract tests

No implementation without contracts.

---

## Contract Governance (Implementation Notes)
- OpenAPI YAML files in /contracts are the authoritative public surface.
- Rust code may generate candidate specs.
- Generated output must be reviewed and committed manually.
- CI will eventually validate that generated spec matches committed spec.
