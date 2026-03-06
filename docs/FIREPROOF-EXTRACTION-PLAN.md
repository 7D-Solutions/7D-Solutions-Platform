# Fireproof ERP Extraction Plan — Complete Findings

**Date:** 2026-03-05
**Status:** Final — ready for ChatGPT and Claude Desktop review before bead creation
**Investigators:** CopperRiver, SageDesert, DarkOwl, Claude Desktop, ChatGPT, BrightHill (orchestrator)

---

## Background

Before creating manufacturing beads, we commissioned 6 independent investigations of the Fireproof ERP codebase (~15,000 LOC) to determine what code and patterns should be pulled into the 7D Solutions Platform. Each investigator examined different modules and compared them against existing platform capabilities.

**Reports produced:**
1. `docs/fireproof-reuse-physical-hierarchy.md` — CopperRiver: Org hierarchy, storage locations, inventory movement
2. `docs/fireproof-reuse-security-rbac.md` — SageDesert: Identity/auth, RBAC, security (3,949 LOC examined)
3. `docs/fireproof-reuse-events-infra.md` — DarkOwl: Event consumer infrastructure, DLQ, idempotency (1,191 LOC)
4. `docs/fireproof-reuse-quality-patterns.md` — DarkOwl: Calibration batch, status machine, quality patterns (2,717 LOC)
5. `docs/fireproof-reuse-platform-clients.md` — CopperRiver: Platform service clients, notifications, numbering (4,251 LOC)
6. `docs/fireproof-reuse-claude-desktop.md` — Claude Desktop: Full cross-reference synthesis
7. `docs/fireproof-reuse-synthesis.md` — Claude Desktop: Final synthesis after reading all agent reports

---

## Part 1: What We Found (Module by Module)

### 1.1 Event Consumer Infrastructure (1,191 LOC in Fireproof)

**Source:** `crates/fireproof-erp/src/events/` — client.rs (407), registry.rs (198), router.rs (201), context.rs (26), idempotency.rs (94), dlq.rs (252)

**Platform gap:** The platform has strong producer-side infrastructure (EventEnvelope, outbox pattern, NATS publish, validation) but ZERO standardized consumer-side infrastructure. No handler registry, no idempotency guard, no DLQ persistent storage, no event router. The C1 receipt event bridge (bd-986e4) already built bespoke dedupe infrastructure, proving this gap is actively causing duplication.

**What Fireproof built:**
- **EventClient** — JetStream durable consumer manager with health/status endpoints
- **HandlerRegistry** — Builder-pattern registry mapping (event_type, schema_version) to async handler functions
- **Event Router** — Validates EventEnvelope, builds HandlerContext, dispatches through registry, returns RouteOutcome (Handled/Skipped/DeadLettered/Invalid/HandlerError)
- **HandlerContext** — Struct carrying tenant_id, actor_id, correlation_id, causation_id, event_id, schema_version
- **Idempotency Guard** — `with_dedupe()` wraps handler in Postgres transaction with (event_id, handler_name) dedupe check, guaranteeing exactly-once over NATS at-least-once delivery
- **DLQ Handler** — Records failures to event_dlq table with full replay context, classifies failures as transient/permanent, provides list/get/mark_replayed queries

**All code is generic** — zero Fireproof-specific types. The only vertical-specific part is the stream subscription config (3 stream names).

**Investigator agreement:** All 7 agree this is the highest-leverage extraction. Every manufacturing phase from B onward needs consumer infrastructure.

---

### 1.2 Security Audit Log (127 LOC in Fireproof, ~60 LOC extractable)

**Source:** `crates/fireproof-erp/src/security/audit_log.rs`

**Platform gap:** Platform uses bare `tracing::warn!()` for auth denials. No structured security event format, no filterable target for SIEM integration, no dedicated audit log abstraction.

**What Fireproof built:**
- `SecurityOutcome` enum (Success/Denied/RateLimited)
- `security_event()` function emitting structured tracing events with event_type, tenant_id, user_id, ip, request_id, outcome, metadata
- Uses `target: "security_event"` for filtering

**Zero Fireproof-specific dependencies.** Direct lift with minimal changes.

**Investigator agreement:** 6 of 7 agree on EXTRACT. Claude Desktop initially said "file enhancement request" but reversed after reading SageDesert's detailed analysis.

---

### 1.3 Security/RBAC/Rate Limiting (3,949 LOC in Fireproof)

**Source:** `crates/fireproof-erp/src/identity_auth/` (1,584 LOC) + `security/` (1,206 LOC) + `error_registry.rs` (1,159 LOC)

**Platform comparison:** Platform security crate (3,548 LOC) is strictly ahead:

| Feature | Platform | Fireproof |
|---------|----------|-----------|
| JWT claims | Typed UUIDs, ActorType enum | Raw strings |
| Auth middleware | Tower Layer (composable, strict/permissive) | Raw middleware::from_fn |
| RBAC enforcement | RequirePermissionsLayer | AuthzGate (less composable) |
| Rate limiting | DashMap (lock-free) + Prometheus metrics | Mutex + HashMap, no metrics |
| Key rotation | PEM env vars, 2-key overlap | JWKS endpoint discovery |

**Only extractable piece:** The 60 LOC audit log (covered above).

**Other notable items:**
- JWKS cache pattern (229 LOC): Useful for verticals deployed separately from identity-auth. Document as reference, don't extract.
- Retry-After header: Platform rate limiter lacks this. Minor enhancement (~10 LOC).
- CSRF (283 LOC): Frontend concern, platform is backend-only. SKIP.
- HIBP (132 LOC): Identity-auth enhancement if needed, not platform security. SKIP.
- Error registry (1,159 LOC): ~95% gauge-specific. Generic pieces (~100 LOC) are useful but extracting is a design decision. SKIP for now.

**Investigator agreement:** All 7 agree to SKIP the bulk. Only the audit log survives.

---

### 1.4 Physical Hierarchy + Storage Locations (1,911 LOC in Fireproof)

**Source:** `crates/fireproof-erp/src/organization/` (1,165 LOC) + `storage_location/` (746 LOC)

**What Fireproof built:**
- Three-level hierarchy: Facility → Building → Zone (all tenant-scoped)
- Deactivation protection (can't deactivate facility with active buildings, etc.)
- Storage location taxonomy (bin/shelf/rack/cabinet/drawer/room/other)
- Allowed item types per location
- Hierarchy resolution JOINs (full path: zone → building → facility)

**Platform comparison:** Inventory has a flat `warehouse_id` on items/locations/ledger. No concept of facilities, buildings, or zones. The `locations` table is simpler (no type taxonomy, no item type filter).

**All code is generic** — no gauge-specific logic. But extraction risks collision with the existing warehouse model (warehouse_id appears in 20+ files across 22K LOC proven Inventory module).

**Investigator agreement:**
- CopperRiver: ADAPT-PATTERN, defer — no manufacturing phase blocked
- Claude Desktop initially said EXTRACT P0, then reversed after reading CopperRiver: "I'm reversing my initial P0 call"
- ChatGPT: "Defer. If no manufacturing phase is blocked, extracting it now is premature platform scope."
- **Consensus: DEFER**

---

### 1.5 Inventory Movement Tracking (630 LOC in Fireproof)

**Source:** `crates/fireproof-erp/src/inventory_movement/`

**What Fireproof built:**
- **MovementRecord** — Immutable, append-only evidence of physical relocation (from/to location, quantity, reason, moved_by, timestamp)
- **CurrentLocation** — Mutable projection: one row per (tenant, entity_type, entity_id) showing current location
- **Atomic move transaction** — Movement record + current_location update in single TX
- Entity type validation (gauge/tool/part with per-type quantity constraints)
- History query + items-at-location reverse lookup

**Platform gap:** Platform Inventory tracks financial truth (costs, quantities, FIFO layers). Movement tracks physical truth (where is this item right now?). These are complementary, not conflicting.

**Investigator agreement:**
- CopperRiver: EXTRACT, but no manufacturing phase requires it
- Claude Desktop initially said EXTRACT P0, then downgraded: "It's needed for Fireproof go-live (which Fireproof already has), not for the platform manufacturing build"
- ChatGPT: Build 3rd after consumer infra and state machines
- **Consensus: DEFER** until Phase E or second vertical

---

### 1.6 Quality Patterns — Calibration Batch + Status Machine (1,188 LOC in Fireproof)

**Source:** `crates/fireproof-gauge-domain/src/calibration_batch.rs` (531 LOC) + `status_machine.rs` (657 LOC)

**Calibration batch pattern:**
- Two-level state machine: batch lifecycle (Draft → PendingSend → Sent → Received → Completed) + item step lifecycle (Added → Sent → ReceivedPass/ReceivedFail → CertVerified → LocationVerified → Released)
- Const transition tables with `.any()` validation
- Ordinal-based step gating (prevents skipping steps, detects backward regression)
- Per-step input validation structs
- Terminal state detection

**Status machine pattern:**
- 12-status exhaustive transition matrix
- Per-transition error codes with metadata
- Calculated status with priority ordering
- Pre-flight eligibility checks

**Platform comparison:** Platform has 3 independent ad-hoc state machines:
- quality-inspection: match on current → allowed list
- production operations: inline status comparison
- workflow: no validation

**Application to manufacturing:**
- Phase C2 in-process/final inspection maps directly to the batch + item pattern
- InspectionBatch (Draft → InProgress → Review → Completed) + InspectionItem (Scheduled → Measured → Recorded → Verified → Dispositioned)

**Investigator agreement:** All agree on ADAPT-PATTERN. Don't extract gauge-specific code. Follow the pattern when building C2. DarkOwl recommends NOT creating a shared crate yet — wait for a fourth state machine.

---

### 1.7 Platform Service Clients (4,251 LOC in Fireproof)

**Source:** `crates/fireproof-erp/src/platform/` + `party/client.rs` + `admin/`

**What Fireproof built:** 6 typed HTTP clients all sharing identical structure:
- reqwest::Client with timeout + connect_timeout
- Exponential backoff retry (100ms × 2^attempt) on 5xx/network errors
- Fail fast on 4xx
- `ClientError` enum with 5 variants (Http, HttpWithBody, Server, Network, Decode)

**Clients:**
| Client | LOC | Platform API Match |
|--------|-----|-------------------|
| NotificationsClient | 429 | Exact match on endpoints |
| NumberingClient | 246 | DTO mismatch (pattern_name vs entity, missing idempotency_key) |
| SodClient | 336 | Exact match |
| PartyClient | 1,074 | Exact match on all routes |
| IdentityAuthClient | 211 | Used for JWKS + user lookup |
| Admin (tenant/user) | 1,487 | Admin-plane, not needed for manufacturing |

**The problem:** ~720 LOC of identical retry/error boilerplate copy-pasted 6 times. A shared `PlatformHttpClient` crate (~200 LOC) would replace this, reducing each client to thin wrappers (~50-120 LOC each).

**Also found two patterns worth documenting:**
1. **Notification template registration** — TemplateKey enum + required_variables validation + idempotent seed at startup
2. **Numbering registry** — NumberedEntity enum → SequenceMapping (pattern_name + gap_free flag)

**Investigator agreement:** EXTRACT the shared client base, but DEFER until a second consumer needs it. Document the notification and numbering patterns NOW.

---

### 1.8 Maintenance Facade + Client (1,528 LOC in Fireproof)

**Source:** `crates/fireproof-erp/src/maintenance/facade.rs` (688 LOC) + `client.rs` (840 LOC)

**What it does:** Thin facade mapping gauge concepts to platform maintenance concepts (Gauge → Asset, Calibration → WorkOrder). The client is a typed HTTP client for maintenance service.

**Investigator agreement:** All 7 agree SKIP. This is correct vertical-layer code. The facade pattern is a reference implementation, not extractable.

---

### 1.9 Other Modules (all SKIP)

| Module | LOC | Why Skip |
|--------|-----|----------|
| Validation crate | ~500 | 95% gauge-specific (thread types, gauge fields) |
| User module (badges, favorites) | 758 | Fireproof UI features |
| Frontend | ~5,000+ | Platform is backend-only |
| Gauge domain (entity, calibration, readings) | ~3,000 | 100% gauge-specific |
| Infrastructure utils (SealState, PurgeCandidate) | ~200 | Gauge-specific |

---

## Part 2: Priority Decisions

### Unanimous Agreement

| Decision | All 7 Agree? |
|----------|-------------|
| Event consumer infrastructure is #1 priority | Yes |
| Security/RBAC/rate limiting: platform is ahead, SKIP | Yes |
| CSRF/HIBP/validation/facade/frontend: SKIP | Yes |
| Batch + state machine: adopt pattern, don't extract code | Yes |
| Maintenance facade: correct as vertical code | Yes |

### Resolved Disagreements

| Item | Initial Split | Final Ruling | Why |
|------|--------------|--------------|-----|
| Org hierarchy | EXTRACT vs ADAPT | **DEFER** | No manufacturing phase blocked; warehouse collision risk |
| API error registry | EXTRACT vs SKIP | **DEFER** | Design decision, not extraction; 95% gauge-specific |
| Event crate location | New crate vs extend event-bus | **New crate** | Different concerns, different dependencies |
| Inventory movement | EXTRACT P0 vs DEFER | **DEFER** | Fireproof already has it; manufacturing doesn't need it yet |
| Security audit log | EXTRACT vs enhancement | **EXTRACT** | 60 LOC, zero dependencies, genuine gap |

---

## Part 3: Extraction Plan

### Tier 1 — Do Now (before resuming Phase B)

**1. Event Consumer Crate** — `platform/event-consumer/`

Extract Fireproof's consumer pipeline as a new platform crate:
- `client.rs` — JetStream consumer manager (~350 LOC adapted)
- `registry.rs` — HandlerRegistry + RegistryBuilder (198 LOC as-is)
- `router.rs` — Event router + RouteOutcome (201 LOC as-is)
- `context.rs` — HandlerContext (26 LOC as-is, add source_module)
- `idempotency.rs` — with_dedupe() (94 LOC as-is)
- `dlq.rs` — DLQ recording + queries (252 LOC as-is)
- `migrations/` — SQL templates for event_dedupe and event_dlq tables

Dependencies: event-bus (EventEnvelope, connect_nats), async-nats, sqlx, serde_json, tokio

Integration with existing platform: Wire to existing `consumer_retry` for retry-then-DLQ flow. Each consuming service gets its own event_dedupe and event_dlq tables (not shared DB).

Estimated LOC: ~1,000 new (from 1,191 original)

**Effort note:** DarkOwl's investigation estimated the code lift alone as small. The actual effort is larger — it includes platform build system integration (Cargo.toml, workspace member, CI job), migration templates that run against the platform's test harness, and wiring `consumer_retry` into the retry-then-DLQ flow.

**2. Security Audit Log** — `platform/security/src/security_event.rs`

Lift from Fireproof:
- `SecurityOutcome` enum (Success/Denied/RateLimited)
- `security_event()` function with structured tracing
- Re-export from `platform/security/src/lib.rs`
- Wire into existing `authz_middleware.rs` denial paths

Estimated LOC: ~60 new

**These two items are independent — different crates, can be worked in parallel.**

### Tier 2 — Document Now (no code)

**3. State Machine Convention**
When writing new state machines (Phase C2 batch inspection, ECO workflows), follow Fireproof's pattern:
- Const transition tables (not inline match arms)
- Ordinal-based step gating for multi-step workflows
- Exhaustive matrix tests (every from→to pair)
- Per-transition error codes with metadata

Reference: `fireproof-gauge-domain/src/calibration_batch.rs` and `status_machine.rs`

**4. Notification Template Registration Pattern**
Each vertical declares notification templates as an enum with:
- `key()` — template identifier string
- `required_variables()` — per-template validation
- `to_seed_request()` — generates CreateTemplateRequest
- `seed_templates()` — idempotent bulk registration at startup

Reference: `fireproof-erp/src/platform/notification_templates.rs`

**5. Numbering Registry Pattern**
Each vertical declares numbered entities with:
- Entity → SequenceMapping (pattern_name + gap_free flag)
- Registration at startup
- Note: Fireproof's client has a DTO mismatch (missing idempotency_key) — manufacturing should follow the actual platform API contract

Reference: `fireproof-erp/src/platform/numbering_registry.rs`

### Tier 3 — Build When Needed

| Item | Trigger |
|------|---------|
| Batch workflow pattern | Phase C2 start |
| Platform SDK crate (shared HTTP client) | Second vertical needs platform clients |
| Inventory movement tracking | Phase E or second vertical |
| Organization hierarchy | Phase E or second vertical |
| API error envelope | Next module scaffold (design ADR first) |
| Retry-After header on rate limiter | Whenever someone touches rate limiting |

---

## Part 4: Impact on Manufacturing Roadmap

### What Changes

The manufacturing roadmap (`docs/plans/MANUFACTURING-ROADMAP.md`) needs one update:

**Add two "Pre-B Infrastructure" rows to Phase B's deliverable table:**
1. Event consumer crate (platform/event-consumer/)
2. Security audit log extraction

These are Phase B prerequisites, not a new manufacturing phase. They're platform infrastructure that happens to be needed before Phase B resumes. Keeping them in Phase B's table avoids phase-number inflation and keeps the roadmap focused on manufacturing deliverables.

### What Doesn't Change

- Phase A: Complete, unaffected
- Phase B: Remaining work proceeds after Tier 1 extraction
- Phase C1: Receipt event bridge can be retrofitted to use event-consumer crate later
- Phase C2: Will use batch workflow pattern and state machine convention from Fireproof reference
- Phase D: Unaffected
- Phase E: Org hierarchy and movement tracking available as reference when needed

### Convention Adoption

All manufacturing beads from this point forward should reference:
- Event consumer crate for any cross-module event handling
- Const transition tables for any new state machines
- Notification template registration pattern for any notification needs
- Numbering registry pattern for any entity numbering

---

## Part 5: What We're NOT Doing (and Why)

| Item | LOC in Fireproof | Why Not |
|------|-----------------|---------|
| Security middleware consolidation | 2,790 | Platform is strictly ahead (typed UUIDs, Tower Layers, DashMap, Prometheus) |
| CSRF protection | 283 | Frontend concern; platform is backend-only |
| HIBP password checking | 132 | Identity-auth enhancement, not platform security |
| Validation crate | ~500 | 95% gauge thread/field validation |
| Error registry bulk extraction | 1,159 | 95% gauge-specific; generic piece is a design decision |
| Maintenance facade/client | 1,528 | Correct as vertical code; reference implementation only |
| Admin clients | 1,487 | Admin-plane, not manufacturing |
| User module (badges, favorites) | 758 | Fireproof UI features |
| Gauge domain logic | ~3,000 | 100% vertical-specific |
| Frontend UI components | ~5,000+ | Platform is backend-only |

**Total Fireproof LOC examined:** ~15,000
**Total LOC extracting:** ~1,060 (event consumer ~1,000 + audit log ~60)
**Total LOC documented as patterns:** ~1,100 (state machine, notification templates, numbering registry, batch workflow)
**Total LOC deferred for future extraction:** ~1,490 (movement tracking, org hierarchy, SDK client, error envelope)
**Total LOC skipped:** ~11,350

---

## Appendix A: Known Fireproof Bug

CopperRiver's platform-clients investigation found that **Fireproof's numbering client is actively broken against the current platform API.** It sends `pattern_name` where the platform expects `entity`, and it lacks `idempotency_key` entirely. This is a bug in production Fireproof code, not just a pattern discrepancy. A Fireproof-side fix bead should be created to align the DTO with the platform's actual API contract.

---

## Appendix B: Investigation Bead Tracker

| Bead | Investigator | Focus | Status |
|------|-------------|-------|--------|
| bd-6sb5k | CopperRiver | Physical hierarchy, storage, movement | Closed |
| bd-1h5cg | SageDesert | Security/RBAC | Closed |
| bd-3fjnp | DarkOwl | Events infrastructure | Closed |
| bd-1sqom | DarkOwl | Quality patterns | Closed |
| bd-3ashx | CopperRiver | Platform clients, notifications, numbering | Closed |
| — | Claude Desktop | Cross-reference + final synthesis | Complete |
| — | ChatGPT | Priority ordering + sequencing | Complete |
