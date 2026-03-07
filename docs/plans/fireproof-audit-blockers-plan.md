# Fireproof Audit Blockers — Plan Sign-Off Request

## Context

Fireproof ERP (first paying customer, aerospace/defense) completed a full codebase audit. Five agents independently reviewed the 7D Platform, two rounds of consensus. Two blockers prevent Fireproof from consuming Platform events. Both confirmed in source code.

ChatGPT has produced a 3-bead plan to fix them. I need your sign-off before publishing beads.

---

## Blocker 1: identity-auth EventEnvelope Divergence

The canonical envelope (`platform/event-bus/src/envelope/mod.rs`) and identity-auth's envelope (`platform/identity-auth/src/events/envelope.rs`) are structurally incompatible:

| Field | Canonical | identity-auth |
|-------|-----------|---------------|
| Payload | `payload: T` | `data: T` |
| Source | `source_module: String` | `producer: String` |
| Tenant ID | `tenant_id: String` | `tenant_id: Uuid` |
| Trace ID | `Option<String>` | `String` (required) |
| Causation ID | `Option<String>` | `Option<Uuid>` |

identity-auth is also missing 10+ canonical fields and has extra fields not in canonical (`aggregate_type`, `aggregate_id`). It has its own publisher that bypasses `event_bus` entirely.

**Impact:** Any consumer using `event_bus::EventEnvelope` gets serde deserialization failures on identity-auth events.

## Blocker 2: Event Naming Convention Inconsistency

- identity-auth: `auth.events.user.registered` with `schema_version: "auth.user.registered.v1.json"`
- All other modules: `{module}.{event_name}` with `schema_version: "1.0.0"` (semver)

Fireproof registered handlers for `user.registered` with semver schema — neither the subject nor version format matches.

---

## ChatGPT's Proposed Plan (3 beads)

### Bead 1 (P0): Fix identity-auth — publish canonical EventEnvelope

- Remove divergent envelope from `platform/identity-auth/src/events/envelope.rs`
- Migrate all publishing call-sites to use `event_bus::EventEnvelope::new()`
- Map fields: `data`->`payload`, `producer`->`source_module`, `tenant_id: Uuid`->String
- Preserve `aggregate_type`/`aggregate_id` inside payload (not new top-level fields)
- Integration test: publish + deserialize roundtrip proving canonical compatibility
- Depends on: nothing

### Bead 2 (P0): Fix identity-auth — align subjects + schema_version

- Map subjects to canonical format: `auth.user_registered`, `auth.user_logged_in`, etc.
- Set `schema_version` to semver `"1.0.0"`
- Dual-publish to legacy `auth.events.*` subjects behind a config flag (`AUTH_LEGACY_SUBJECTS=1`)
- Integration test verifying new subjects, semver version, and optional dual-publish
- Depends on: Bead 1

### Bead 3 (P1): CI guardrails to prevent regression

- CI check: fail if any crate defines `EventEnvelope` outside `platform/event-bus/`
- CI check: fail on `.events.` in subjects or non-semver `schema_version`
- Wire into PR CI
- Depends on: Bead 2

---

## What I Need From You

Please review and confirm:

1. **Correctness**: Does the migration approach handle all field differences without breaking existing consumers? Specifically: is putting `aggregate_type`/`aggregate_id` inside payload (rather than new top-level envelope fields) the right call?

2. **Completeness**: Are there edge cases we're missing? For example:
   - Are there any consumers of the legacy identity-auth envelope format we need to worry about?
   - Does the `tenant_id` Uuid->String conversion need special handling?
   - What about existing events already persisted in NATS JetStream with the old format?

3. **Risk**: Is the dual-publish approach for subjects safe? Could it cause duplicate processing?

4. **Scope**: Is this appropriately lean — minimum work to unblock Fireproof, no over-engineering?

Please respond with APPROVED, APPROVED WITH CHANGES, or BLOCKED (with reasons).
