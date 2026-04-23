# MODE OUTPUT — I4: Perspective-Taking

**Analysis date:** 2026-04-16
**Mode:** I4 — Four Distinct Stakeholder Perspectives
**Reviewer:** LavenderGlacier
**Specs reviewed:** SALES-ORDERS, OUTSIDE-PROCESSING, CUSTOMER-COMPLAINTS, CRM-PIPELINE,
SHOP-FLOOR-GATES, PLATFORM-EXTENSIONS, bd-ixnbs plan doc

---

## 1. Thesis

The five new modules and seven extensions are architecturally coherent and internally well-specified.
But each of the four stakeholder groups faces a distinct gap the spec author couldn't see from inside
the spec itself: RoseElk is blocked by an undefined overlay bootstrapping protocol, not by missing API
surfaces; HuberPower's canonical constraints (signoff roles, opportunity types) are aerospace-flavored
and will generate implementation-day friction; TrashTech's non-returning waste flow breaks the
outside-processing state machine; and implementation agents will silently drop cross-module enforcement
obligations because the specs define the behavior but don't assign ownership of the enforcement bead.
One event dependency — manufacturing costing consuming `shop_floor_data.labor.approved.v1` — is
structurally broken: shop-floor-data was ruled out of platform scope and its events are Fireproof-local.

---

## 2. Top Findings

---

### §F1 — Manufacturing Costing Consumes a Fireproof-Local Event
**Perspective:** Implementation agent (CopperRiver, PurpleCliff)
**Evidence:** `PLATFORM-EXTENSIONS-SPEC.md §4`, "Consumed events (cost-posting triggers)":
> `shop_floor_data.labor.approved.v1` → Production computes labor cost and posts

`shop-floor-data` was explicitly ruled OUT of platform scope
(`bd-ixnbs-fireproof-platform-migration.md §D`: "SHOP-FLOOR-DATA-MODULE-SPEC.md — kiosks + operator
sessions + kiosk-driven labor capture stay in Fireproof"). The labor approval event is Fireproof-local.
Platform Production's manufacturing costing extension lists this as a consumed event, but that event
is never published to the shared platform NATS bus. Fireproof is a consumer of platform modules, not a
publisher into the platform event bus — at least, no spec defines that it is.

**Reasoning chain:** This isn't a semantic mismatch; it is a broken event dependency. The costing
extension will be built to subscribe to a subject that never has a publisher on the shared bus. Labor
hours accrued via Fireproof's kiosk sessions will produce no cost postings. The `labor_cost_cents`
bucket in `work_order_cost_summaries` will always be zero for Fireproof.

**Severity:** Critical
**Confidence:** 0.95
**So what:** Before any implementation bead for manufacturing costing is written, decide: (a) does
Fireproof's local labor-approval flow publish to the shared platform NATS bus? If yes, define the
publishing contract and who owns it. If no, how does labor cost reach platform Production? Options:
direct API call from Fireproof to Production's cost-posting endpoint, or a Fireproof overlay that
translates its local events into platform cost postings. This must be resolved before the costing bead
is decomposed.

---

### §F2 — Cross-Module Enforcement Is Defined But Not Assigned to a Bead
**Perspective:** Implementation agent (CopperRiver, PurpleCliff)
**Evidence:**
- `SHOP-FLOOR-GATES-MODULE-SPEC.md §8, invariant 8`: "Production should check for active
  operation-scoped holds before allowing an operation to start. Gates emits `hold.placed.v1`;
  Production is the enforcer."
- `SHOP-FLOOR-GATES-MODULE-SPEC.md §8, invariant 4`: "Accept is allowed only if handoff quantity ≤
  remaining quantity at the dest operation (per Production's op quantity tracking)."
- `OUTSIDE-PROCESSING-MODULE-SPEC.md §8, invariant 7`: "If `source_entity_type = work_order`,
  `source_entity_id` must reference a platform Production work order that exists and belongs to the
  same tenant."

Each of these is a cross-module behavioral contract where module A specifies behavior that must be
implemented inside module B. The spec correctly notes that module A cannot enforce these directly.
But the spec does not assign an implementation bead for the Production-side consumer that enforces
them.

**Reasoning chain:** When beads are decomposed per the plan (scaffolding → schema → domain+repo →
routes → events → typed client), the shop-floor-gates beads implement gates' side and call it done.
The Production module beads will similarly implement Production's existing behaviors. The "check for
active holds on operation start" logic is not in either module's natural scope — it's coordination
logic that lives in Production but depends on data from Gates. Without an explicit bead for
"Production: consume `hold.placed.v1` and block operation start," it falls between the cracks. Two
agents will independently build clean modules and nobody will notice the missing enforcement until
a shop-floor test tries to start an operation under an active hold.

**Severity:** High
**Confidence:** 0.90
**So what:** For each cross-module enforcement statement in the specs, the implementation bead
decomposition must include a named "consumer bead" in the enforcing module. These are not
automatically created by naive spec decomposition.

---

### §F3 — Outside-Processing State Machine Assumes Material Returns; Hazwaste Flow Is One-Way
**Perspective:** TrashTech / RanchOrbit
**Evidence:** `OUTSIDE-PROCESSING-MODULE-SPEC.md §6`, state machine:
```
draft → issued → shipped_to_vendor → at_vendor → returned → review_in_progress → closed
```
Invariant 3: `SUM(op_return_events.quantity_received) <= SUM(op_ship_events.quantity_shipped)`

TrashTech's primary outside-processing use case: sending hazardous waste to a licensed treatment
facility. The waste is destroyed (incinerated, neutralized, landfilled). It does not return. The
state `at_vendor` has no valid outbound transition except `returned` or `cancelled`. Sending
hazwaste to an incinerator leaves the OP order stuck at `at_vendor` with no `return_event` ever
forthcoming.

The `condition` field on return events (good/damaged/discrepancy) is meaningless when the "return"
is a certificate of destruction. The quantity invariant is violated at the type level: TrashTech
ships in gallons or tons; the manifest closure references weight destroyed, not quantity returned.

**Reasoning chain:** This is not a cosmetic fit issue. The module's state machine has no terminal
state reachable without a `return_event`. TrashTech cannot reach `review_in_progress` or `closed`
via the designed path. The workaround — pretending a certificate of destruction is a "return event"
with `quantity_received = quantity_sent` and `condition = good` — is semantically false and breaks
any downstream analytics.

**Severity:** High (for TrashTech's hazwaste use case)
**Confidence:** 0.90
**So what:** Add a `disposition_type` field on `op_orders` (canonical: `round_trip` / `destruction`
/ `transformation`). When `disposition_type = destruction`, skip or replace the `returned` state
with `disposal_confirmed`, triggered by a vendor disposition certificate record rather than a
return event. This is an incremental spec change, not a redesign.

---

### §F4 — RoseElk's Overlay Services Have No Bootstrapping Protocol
**Perspective:** Fireproof orchestrator (RoseElk)
**Evidence:** `bd-ixnbs-fireproof-platform-migration.md §"Aerospace overlay pattern"`:
> "Fireproof runs its own local service that subscribes to platform events and maintains its own
> overlay tables. Platform modules never know about vertical-specific fields."

The overlay pattern appears in every new module spec (outside-processing §10, customer-complaints
§10, shop-floor-gates §10). But no spec answers:
- Does the Fireproof overlay service use `ModuleBuilder.from_manifest()` (the platform SDK)?
- What port does it run on?
- How does it authenticate NATS subscriptions — does Fireproof get a dedicated NATS subject prefix?
- When the Fireproof frontend fetches a platform record + overlay data, what is the join protocol
  if the overlay row doesn't exist yet (event not yet processed)?
- Who writes the overlay service bead — Fireproof-vertical bead pool or platform bead pool?

**Reasoning chain:** RoseElk's 12 paused frontend beads display data that combines platform records
with Fireproof-specific fields (cert-of-conformance, AS9100 clause citations, formal response SLAs).
Today that's all in Fireproof's local module. After migration, the platform module holds the canonical
record and the overlay holds the extra fields. RoseElk's frontend must join them. If the overlay row
is empty (event delivery delayed), the UI must gracefully degrade. None of this join contract is
specified. RoseElk can't write frontend beads without knowing the overlay API surface, the join
semantics, and the error/empty-state behavior.

**Severity:** High
**Confidence:** 0.88
**So what:** The migration plan's step 2 says "mail RoseElk with concrete module-migration calls."
That mail needs to additionally include: (a) the overlay service architecture decision (platform SDK
vs. standalone), (b) the join protocol for missing overlay rows, (c) who owns the overlay service bead.
Without this, RoseElk's 12 beads remain paused for a different reason than before.

---

### §F5 — Typed Client API Shape Is Undefined; RoseElk Cannot Unblock Without It
**Perspective:** Fireproof orchestrator (RoseElk)
**Evidence:** Migration notes in every new module spec end with a form of:
> "Sample data only — drop Fireproof tables, create fresh schema, Fireproof rewires to typed
> client (`platform_client_sales_orders::*`)"

The typed clients are named but not defined. Their API surface (method names, input/output types,
async vs. sync, error types) is an artifact of code generation or hand-authoring that happens after
implementation. The migration notes describe the migration endpoint but not the transition path:
Fireproof currently calls its own local `sales_order` module. RoseElk cannot begin writing rewiring
beads until the typed client crate exists with a stable interface.

**Reasoning chain:** If any of RoseElk's 12 beads depend on Fireproof calling a new platform module
API (e.g., POSTing a new sales order), those beads need the typed client crate to exist with a known
method signature. The standard decomposition sequence puts "typed SDK client stub" last. That means
RoseElk's rewiring beads cannot start until at least the SDK stub bead is complete for each module.
The current plan doesn't model this as a dependency. If bead decomposition is parallelized without
accounting for this, Fireproof agents will start rewiring beads before the clients exist.

**Severity:** High
**Confidence:** 0.85
**So what:** The bead decomposition must explicitly mark typed client stubs as blocking dependencies
for Fireproof's rewiring beads. Mail RoseElk the resulting dependency graph so she knows which of
her 12 beads unblocks when which platform bead closes.

---

### §F6 — CRM Opportunity Type Canonical Enum Is Aerospace-Flavored
**Perspective:** HuberPower (hypothetical)
**Evidence:** `CRM-PIPELINE-MODULE-SPEC.md §3`, `opp_type` field:
> canonical: `new_business/repeat_order/contract_renewal/engineering_change/prototype`

The spec says tenants can only rename the *display label* of these values, not add or remove them.
`engineering_change` and `prototype` are concepts with strong Fireproof-aerospace resonance (ECOs,
first article inspections). HuberPower selling capital power-gen equipment would have opportunity
types like `service_contract`, `installation_project`, `spare_parts_order`. These don't map cleanly
to the canonical five.

**Reasoning chain:** HuberPower would have to rename `prototype` to `commissioning_project` or
similar. That works for display, but events carry canonical values only — so downstream analytics
and integrations would see `prototype` in event payloads for what is actually a commissioning
engagement. HuberPower's reporting team must map canonical codes to their own taxonomy at every
query boundary. More critically: if HuberPower has a deal type with a distinct close motion (e.g.,
a `maintenance_contract` which is a recurring service agreement, not a product sale), they cannot
represent it canonically and cannot route on it in downstream automations.

**Severity:** Medium
**Confidence:** 0.80
**So what:** Before finalizing the CRM-pipeline spec, test the canonical `opp_type` set against
HuberPower's and TrashTech's actual deal vocabulary. If `engineering_change` and `prototype` have no
meaningful cross-vertical analog, replace them with more neutral values (`project`, `service_agreement`)
that cover Fireproof's cases equally well. This is a pre-implementation spec edit, not a schema change.

---

### §F7 — Outside-Processing Quantity Invariant Breaks for Unit-Transforming Verticals
**Perspective:** TrashTech / RanchOrbit
**Evidence:** `OUTSIDE-PROCESSING-MODULE-SPEC.md §8, invariant 3`:
> `SUM(op_return_events.quantity_received) <= SUM(op_ship_events.quantity_shipped)`

Both `quantity_shipped` and `quantity_received` reference `unit_of_measure` but the invariant
compares raw quantities without UoM equivalence. RanchOrbit sends cattle by head (100 head
shipped), receives processed meat in pounds (48,000 lbs returned — a completely different UoM
and quantity scale). The invariant as written fails numerically: `48000 <= 100` is false, so the
system would reject the return event as exceeding ship quantity.

**Reasoning chain:** This invariant is written from the perspective of manufactured parts (heat
treating 20 pieces and getting 20 pieces back). For verticals where processing transforms the
material's unit (head → lbs, liquid waste volume → treated solids weight), the invariant is
mechanically wrong. Enforcing it as-is will cause every `return_event` to fail validation for
these use cases.

**Severity:** High (breaks RanchOrbit and some TrashTech flows where UoM changes)
**Confidence:** 0.85
**So what:** Scope the invariant to apply only when `op_ship_events.unit_of_measure =
op_return_events.unit_of_measure`. When UoMs differ (a transformation scenario), the quantity
ceiling check is inapplicable by design. Add a flag or derive from `disposition_type` (§F3 fix).

---

### §F8 — Shop-Floor-Gates Signoff Role Whitelist Is Manufacturing-Centric
**Perspective:** HuberPower (hypothetical)
**Evidence:** `SHOP-FLOOR-GATES-MODULE-SPEC.md §3`, signoffs table:
> `role` (canonical: quality/engineering/supervisor/operator/planner/material — tenant may rename
> via labels)

HuberPower's power-gen manufacturing requires electrical inspection, commissioning engineer, field
service, and safety officer sign-offs at critical control points (HV testing, factory acceptance
testing, commissioning at customer site). These role categories don't exist in the canonical six.
Since tenants can only rename — not add — canonical roles, HuberPower must map `commissioning_engineer`
to, say, `engineering`, making the audit trail semantically ambiguous.

**Reasoning chain:** In regulated power-gen environments (HV electrical, OSHA, IEC 62271 compliance),
signoff roles must be unambiguous in the audit record. An audit trail that shows `role = engineering`
when the signer was actually a licensed commissioning engineer of record creates compliance exposure.
HuberPower won't break the platform; they'll work around it via an overlay. But the workaround
requires an overlay service just to maintain role fidelity — overhead for a feature the platform
already nominally provides.

**Severity:** Medium
**Confidence:** 0.78
**So what:** Extend the canonical signoff role set with two industry-neutral additions: `inspector`
and `commissioning`. These are additive (no existing values change), cover electrical, process
safety, livestock inspection, and commissioning use cases across all verticals, and require a one-line
spec edit and a migration to add the enum values.

---

## 3. Risks Identified

| Risk | Severity | Likelihood | Perspective |
|------|----------|------------|-------------|
| Labor cost event never fires (§F1) | Critical | High | Implementation agent |
| Cross-module enforcement beads not created (§F2) | High | High | Implementation agent |
| TrashTech cannot close hazwaste OP orders (§F3) | High | High | TrashTech |
| Fireproof rewiring blocked by absent typed clients (§F5) | High | High | RoseElk |
| RanchOrbit return-event invariant violations (§F7) | High | Medium | RanchOrbit |
| RoseElk builds overlay against unanswered protocol questions (§F4) | High | Medium | RoseElk |
| HuberPower commissioning role mismatch in audit records (§F8) | Medium | High | HuberPower |
| CRM opp types wrong vocabulary for non-aerospace verticals (§F6) | Medium | Medium | HuberPower / RanchOrbit |
| Open questions resolve inconsistently across parallel beads | Medium | High | Implementation agent |
| TrashTech builds against sales-orders; abandons when fit is poor | Medium | Low | TrashTech |

---

## 4. Recommendations

### P0 — Resolve Before First Implementation Bead

**R1. Define the labor-cost event bridge (§F1)**
Effort: low. Decision only — no code. Choose: (a) Fireproof publishes
`shop_floor_data.labor.approved.v1` to shared platform NATS, owning that subject contract; or (b)
Fireproof's local labor-approval flow calls Production's `POST /cost-postings` endpoint directly
with an authenticated service token. Document the choice in PLATFORM-EXTENSIONS-SPEC.md §4 and
add it to the migration notes. Without this, manufacturing costing produces zero labor cost forever.

**R2. Add one-way disposition path to outside-processing (§F3, §F7)**
Effort: low. Add `disposition_type` field (canonical: `round_trip` / `destruction` /
`transformation`) to `op_orders`. When `disposition_type != round_trip`, allow `at_vendor →
disposition_confirmed → closed` without a `return_event`. Scope invariant 3 (quantity comparison)
to same-UoM cases only. This fixes both TrashTech hazwaste and RanchOrbit unit-transform issues in
one additive schema change.

### P1 — Resolve Before Bead Decomposition

**R3. Write the overlay bootstrapping protocol (§F4)**
Effort: medium. Produce a one-page "Fireproof overlay service pattern" document specifying: SDK
usage decision, NATS subject namespace for Fireproof-local events, join protocol for empty overlay
rows (return null vs. return partial object with `overlay_pending: true`), auth model, bead
ownership (Fireproof vertical pool vs. platform pool). This is the concrete input RoseElk's 12
beads need.

**R4. Model typed client dependencies in the bead graph (§F5)**
Effort: low. For each new platform module, the bead decomposition must include a "typed SDK client
stub" bead and mark Fireproof's rewiring beads as `depends-on: <client-stub-bead>`. Mail RoseElk
the resulting dependency graph before she unpauses her beads.

**R5. Assign a "consumer bead" for every cross-module enforcement obligation (§F2)**
Effort: low per decision, medium in aggregate. Before decomposition, enumerate all cross-module
enforcement statements in all specs. For each, name the enforcing module and create a consumer bead
in its bead set. At minimum:
- Production: consume `shop_floor_gates.hold.placed.v1` → block operation start
- Production / Shop-Floor-Gates: handoff quantity check against Production's op quantity endpoint
- Outside-Processing at `issue` time: GET Production's work order to validate `source_entity_id`

**R6. Close the spec's open questions before bead decomposition**
Effort: low (decisions only). Resolve: (a) remnant tracking Option A vs. B; (b) kit-readiness
snapshot optional vs. always; (c) backorder `stock_status` column included or not; (d) overhead
cost columns in v0.1 schema. Write one binding choice per open question into the spec. Otherwise
two parallel agents will resolve the same question differently and create inconsistent schema.

### P2 — Address Before First Customer Demo

**R7. Expand canonical signoff roles with `inspector` and `commissioning` (§F8)**
Effort: low. Two additive enum values. Covers commissioning engineer, electrical inspector, safety
officer, USDA livestock inspector across all verticals. No existing canonical value changes. Requires
one spec edit and one migration row.

### P3 — Address Before Second Vertical Onboards

**R8. Test CRM opp-type canonical set against HuberPower vocabulary (§F6)**
Effort: low (conversation + spec edit). Map HuberPower's actual deal types against the five
canonical values. If `engineering_change` and `prototype` have no clean mapping, replace them
with `project` and `service_agreement` before the first implementation bead. This is a pre-code
change, not a migration.

---

## 5. New Ideas and Extensions

### Incremental
- **`disposition_type` on OP orders** (§F3/§F7 combined fix): `round_trip | destruction |
  transformation` as a field on `op_orders`. Branches the state machine at `at_vendor` without
  breaking the existing round-trip path.
- **Kit-readiness by BOM variant** (HuberPower configured-product gap): add a `component_overrides`
  optional input to `POST /api/bom/kit-readiness/check` that substitutes variant component IDs at
  check time without requiring a separate BOM revision. Compute-only, no schema change.

### Significant
- **Fireproof overlay service standard template**: a documented pattern + example crate showing how
  a vertical bootstraps an overlay service using the platform SDK, subscribes to platform NATS
  subjects, and exposes a join endpoint with `overlay_pending` semantics. Not platform code — a
  template in `docs/patterns/`. Unblocks RoseElk and establishes the pattern for HuberPower.
- **OP order flow type + disposition certificate record**: parallel to `disposition_type`, add a
  `disposition_certificates` table for one-way flows (hazwaste manifest closures, certificates of
  destruction, livestock health certs). This gives TrashTech a structured record type that replaces
  the semantically-false workaround of encoding a cert as a `return_event`.

### Radical
- **Service-order subtype of sales-orders**: a thin extension that lets TrashTech declare a sales
  order as `order_type = service_agreement`, disabling inventory reservation events, replacing
  `shipped` status with `completed`, and attaching a service period range. Avoids forcing TrashTech
  to build directly on AR subscriptions and keeps their commercial lifecycle in the module that
  already handles order lifecycle.

---

## 6. Assumptions Ledger

1. HuberPower's product domain is capital power-gen equipment (turbines, generators, switchgear)
   with long production cycles. Their actual workflows are inferred from the plan doc description;
   no HuberPower spec was reviewed.

2. TrashTech's outside-processing use case includes hazardous waste treatment (one-way disposal). If
   TrashTech's actual OP use case is recycling only (where processed material returns as a different
   form), §F3 severity drops to medium.

3. The platform NATS bus is shared across platform modules and is NOT shared with vertical-local
   event publishing by default. If Fireproof already publishes to shared NATS today, §F1 may be
   resolved.

4. RoseElk's 12 paused beads include some that depend on Fireproof calling new platform module APIs
   (not just displaying data). If all 12 are pure UI rendering beads with no module-API call
   dependencies, §F5 severity drops to low.

5. Typed clients are auto-generated from OpenAPI contracts. If they are hand-written, their API
   shape is decided by the implementing agent at bead time and the §F5 blocking dependency timing
   remains the same.

---

## 7. Questions for Project Owner

1. **§F1**: Does Fireproof's local event bus publish to the shared platform NATS today, or is
   Fireproof a pure consumer of platform events? If it doesn't publish, how does labor cost reach
   Production's costing extension?

2. **§F3**: Is TrashTech's outside-processing use case primarily hazwaste (one-way destruction) or
   recycling (material comes back transformed)? This determines whether §F3 is a blocker or a
   medium friction item.

3. **§F4**: Is the Fireproof overlay service expected to use `ModuleBuilder.from_manifest()` (full
   platform SDK module with its own port and DB) or is it a lighter Rust service that just subscribes
   to NATS and writes to Fireproof's existing DB?

4. **§F6**: Is HuberPower's sales motion sufficiently milestone-based (capital equipment, progress
   billing) that `sales-orders` needs a `service_agreement` order type before they onboard, or is
   the blanket-release model a close-enough fit for their delivery schedules?

5. **Open questions**: Which of the deferred open questions (remnant Option A/B, kit-readiness
   snapshot, backorder `stock_status` column, overhead columns in v0.1) need a binding decision
   before bead decomposition to avoid two agents resolving the same question inconsistently?

---

## 8. Points of Uncertainty

- **RanchOrbit's actual use of CRM-pipeline**: ranching's sales motion may not involve a
  lead-to-opportunity funnel. The module may be used only for new customer acquisition, or skipped
  entirely. Uncertainty: medium.

- **Whether HuberPower's production work orders fit the discrete-op model**: shop-floor-gates is
  anchored to Production's `work_order / operation` entity hierarchy. If HuberPower's production is
  project-like (milestone phases, not discrete operations), gates' anchoring entity may be wrong.
  Uncertainty: medium-high.

- **Fireproof's current NATS publishing scope**: if Fireproof already publishes any events to the
  shared platform bus (e.g., for GL integration), §F1 may be narrower than it appears — only the
  labor-specific event path needs a decision, not the entire event publishing model.

- **Whether the overlapping barcode resolution call chain causes a practical problem**: when a module
  calls Inventory's barcode resolver and gets back a `work_order` reference, who makes the Production
  validation call? In Fireproof's kiosk flow the caller is the kiosk UI — clear. In a
  module-to-module flow it is not. Uncertainty: low-medium.

---

## 9. Agreements and Tensions with Other Perspectives

**Expected agreements with other modes:**
- A systems-thinking analysis (F7) would likely also flag the labor event dependency as a broken
  integration seam. §F1 is a systems coupling issue as much as a bead-decomposition issue.
- A first-principles analysis would likely flag the one-way material flow problem (§F3) as a
  domain-modeling gap: the model assumes round-trip material custody, not material disposition.
- A pre-mortem analysis would surface §F2 (enforcement gaps) and §F5 (blocked typed client
  dependency) as the most likely causes of a delayed or broken initial deployment.

**Expected tensions with other modes:**
- A domain-modeling purist might argue `sales-orders` for service businesses is a category error and
  push toward a separate `service-agreements` module. The I4 perspective notes the friction but does
  not recommend a new module — that would violate the "more than one vertical would use this" test.
- An implementation-first perspective might argue open questions should be resolved by the
  implementing agent's judgment, and forcing spec resolution before bead decomposition slows
  delivery. The I4 perspective disagrees: two agents independently resolving the same schema option
  creates inconsistency that is more expensive to fix post-merge than the delay in deciding upfront.

---

## 10. Confidence: 0.82

**Calibration note:** High confidence on §F1 (labor event dependency), §F2 (cross-module
enforcement gap), §F3 (one-way material flow), §F7 (UoM invariant break). These are structural
observations against the spec text, not inferences. Medium confidence on HuberPower findings (§F6,
§F8) because HuberPower's actual workflows are inferred from plan-doc descriptions, not documented
specs; the findings may be narrower or broader in practice. Lower confidence on RoseElk findings
(§F4, §F5) because the overlay protocol and typed client generation strategy may be more developed
in oral context than what appears on paper — if RoseElk already knows the overlay bootstrapping
answer, §F4 resolves. The overall rating reflects that two of the eight findings are
inference-dependent.
