# Fireproof ERP Reuse Investigation Brief

> **Purpose:** Determine what code from Fireproof ERP can be extracted and incorporated into the 7D Solutions Platform to accelerate the manufacturing build.
>
> **Fireproof ERP location:** `/Users/james/Projects/Fireproof-ERP/`
> **7D Platform location:** `/Users/james/Projects/7D-Solutions Platform/`
> **Manufacturing roadmap:** `docs/plans/MANUFACTURING-ROADMAP.md`

## Context

Fireproof ERP is our first vertical application — an aerospace/defense gauge management system built ON TOP of the 7D Platform. It has already solved many problems that the manufacturing build will need. Before creating new manufacturing beads, we need to catalog everything reusable and update the roadmap accordingly.

Fireproof is a **thin facade** — it delegates to 7D Platform services (maintenance, party, identity-auth, notifications, numbering) via typed HTTP clients. But it also contains cross-cutting infrastructure that was built for Fireproof but belongs in the platform.

## Fireproof Codebase Map

### Main crate: `crates/fireproof-erp/src/`

| Directory/File | LOC | What It Does |
|---|---|---|
| `organization/` | 1,165 | Facility -> Building -> Zone physical hierarchy. Tenant-scoped, integer IDs, display_order, is_active. Generic — not gauge-specific. |
| `inventory_movement/` | 630 | MovementRecord (immutable evidence of item moving between locations), CurrentLocation (where item is now). Entity types: gauge, tool, part. |
| `storage_location/` | 746 | StorageLocation linked to org hierarchy via zone_id. Types: bin, shelf, rack, cabinet, drawer, room, other. allowed_item_types per location. |
| `identity_auth/` | 1,584 | RBAC enforcement (AuthzGate, require/require_role middleware), RequestContext extractor, JWKS fetcher, JWT middleware. 21 platform scope constants. |
| `security/` | 1,206 | Token-bucket rate limiter (per-IP, per-tenant), CSRF double-submit cookie, HIBP password check, audit log emitter. |
| `error_registry.rs` | 1,159 | ApiError struct with IntoResponse, ~50 error codes mapped to HTTP statuses, convenience constructors (bad_request, not_found, etc). |
| `events/` | 1,191 | NATS JetStream client, DLQ handler, idempotency dedup (with_dedupe), event registry, event router. |
| `platform/` | ~600 | Typed HTTP clients for: notifications, numbering, SoD. Plus delivery receipt queries and numbering registry. |
| `maintenance/` | 1,527 | MaintenanceClient (typed HTTP client for 7D maintenance) + facade handlers (Gauge/Calibration/Reading API). |
| `admin/` | 1,487 | Control plane, tenant registry client, user management. |
| `config/events.rs` | 82 | NATS stream definitions (AUTH_EVENTS, PARTY_EVENTS, MAINTENANCE_EVENTS). |
| `projections/` | 126 | Auth activity projection handler (proj_auth_activity from auth events). |
| `party/client.rs` | 1,074 | Typed HTTP client for 7D party service. |

### Domain crate: `crates/fireproof-gauge-domain/src/`

| File | LOC | What It Does |
|---|---|---|
| `calibration_batch.rs` | 531 | BatchStatus state machine (Draft->PendingSend->Sent->Received->Completed+Cancelled). ItemStep state machine. Ordinal-based gating. **Pattern reference** for quality inspection batch workflows. |
| Full crate | 3,161 | Gauge-specific domain logic. NOT extractable as-is, but patterns are reusable. |

### Gauge service crate: `crates/fireproof-gauge-service/src/`
| Total | 10,166 | Gauge-specific service layer. NOT extractable. |

## What Already Exists in 7D Platform

| Platform Component | LOC | Overlap with Fireproof |
|---|---|---|
| `platform/security/` | 3,548 | Has claims, rbac, authz_middleware, permissions, ratelimit, middleware, service_auth, webhook_verify, tracing, redaction. **Fireproof's security may be partially redundant.** |
| `platform/identity-auth/` | ~3,000 | JWT auth, RBAC, SoD, session management. Fireproof's identity_auth/ is a CLIENT for this. |
| `modules/quality-inspection/` | 1,504 | Has inspection plans, receiving inspections, disposition state machine (pending->held->accepted/rejected/released). **Fireproof's calibration batch pattern could enhance this.** |
| `modules/inventory/` | existing | Has inventory transactions. Fireproof's inventory_movement is a DIFFERENT pattern (physical tracking vs financial). |
| `modules/production/` | existing | Has work orders, routing, operations. |
| `modules/maintenance/` | existing | Fireproof delegates TO this. |

## Investigation Questions (per focus area)

For each Fireproof module you investigate:

1. **Is it already in the platform?** Compare against existing platform/modules code.
2. **Is it generic or gauge-specific?** Could a second vertical (e.g., food manufacturing) use it as-is?
3. **What changes would be needed?** Strip gauge-specific logic, rename types, adjust dependencies.
4. **Where does it land in the platform?** New module? New platform crate? Extension of existing module?
5. **Which manufacturing roadmap phase does it accelerate?** Reference specific phases from the roadmap.
6. **LOC estimate for extraction?** How much code can be lifted vs rewritten?

## Output Format

Each agent produces a report at `docs/fireproof-reuse-{focus-area}.md` with:
- Module-by-module assessment (extract / adapt-pattern / skip)
- Concrete code paths to extract
- Dependencies that come along
- Mapping to manufacturing roadmap phases
- Recommended bead(s) for extraction work
