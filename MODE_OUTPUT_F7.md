# MODE_OUTPUT_F7 — Systems-Thinking Analysis

**Mode:** F7 — Systems-Thinking  
**Analyst:** GrayCove  
**Date:** 2026-04-16  
**Spec set:** bd-ixnbs migration specs (5 new modules + 7 extensions)

---

## 1. Thesis

When the six specs are read together rather than in isolation, three emergent structural problems appear that no individual spec could see: a broken manufacturing labor feed that severs costing for non-Fireproof verticals; a tri-modal customer identity problem where Party, AR, and Sales-Orders each hold a different fragment of the same "customer" with no specified creation chain; and a hold-enforcement safety gap where Shop-Floor-Gates defines the rule but leaves Production as the unnamed enforcer with no corresponding Production change bead. Beyond these three load-bearing issues, the system also accumulates four medium-severity seams — asymmetric on-hand input conventions within the BOM module, an unowned CRM→Sales-Orders handoff, an AP disqualification event that nobody downstream consumes, and unenumerated source_entity_type strings that will silently break cross-module event subscriptions. The whole system works individually; the integration points are where it will fail.

---

## 2. Top Findings

### §F1 — Manufacturing Costing's Labor Feed Is Severed

**Evidence:** `PLATFORM-EXTENSIONS-SPEC.md §4` (Production extension, consumed events):
> `shop_floor_data.labor.approved.v1` → Production computes labor cost

`bd-ixnbs-fireproof-platform-migration.md §D` (retired modules):
> `SHOP-FLOOR-DATA-MODULE-SPEC.md — banner-flagged. Only the barcode resolution portion moved to the Inventory extension; kiosks + sessions + kiosk-driven labor capture stay in Fireproof`

**Reasoning chain:** The cost accumulation engine for production work orders explicitly depends on an event named after a platform module (`shop_floor_data`) that was retired before implementation. In Fireproof this will work because Fireproof owns the kiosk/labor capture code and can emit `shop_floor_data.labor.approved.v1` from its local service. But HuberPower, the other manufacturing vertical, has no local equivalent of Shop-Floor-Data. To use manufacturing costing, HuberPower would need to produce events named after a platform module that doesn't exist, against a contract that was never published. The event name carries a false implication of platform ownership. When HuberPower tries to implement costing, the labor feed will be missing and the cost summary will show zero labor — silently wrong, not erroring.

**Severity:** Critical  
**Confidence:** 0.95  
**So What:** Before the Production costing bead is written, decide: (a) rename the consumed event to something platform-neutral like `production.labor_entry.approved.v1` and have each vertical's labor capture emit under that name; or (b) add a thin platform abstraction that represents "approved labor record" regardless of capture mechanism, and specify it in the Production extension. Option (a) costs less. Either way, the event name `shop_floor_data.*` cannot appear in a platform module's consumed events list.

---

### §F2 — Tri-Modal Customer Identity With No Creation Chain

**Evidence:** `SALES-ORDERS-MODULE-SPEC.md §3` (sales_orders table):
> `customer_id` (ref → AR customer), `party_id` (ref → Party)

`CRM-PIPELINE-MODULE-SPEC.md §9` (integration notes):
> "no automatic AR creation — vertical orchestrates this via their own event handler. Avoids tight coupling."

`CRM-PIPELINE-MODULE-SPEC.md §4.1`:
> `POST /api/crm-pipeline/leads/:id/convert` — creates Party company (via Party API)

**Reasoning chain:** The system has three distinct "customer" concepts across three modules:
1. Party (`party.companies`) — the canonical identity record; created by CRM on lead conversion
2. AR customer (`ar_customers`) — the billing account; not created by CRM, not created by Sales-Orders
3. Sales-Orders needs *both* `customer_id` (AR) *and* `party_id` (Party)

The creation sequence is: CRM converts lead → creates Party → ???AR customer??? → Sales-Orders books order. The middle step has no owner. CRM explicitly delegates AR customer creation to "vertical orchestration." Sales-Orders cannot be booked without an AR `customer_id`, but no platform event or workflow creates the AR customer when CRM converts a lead. Every vertical will independently wire this bridge, producing four different implementations of the same seam. Worse: the AR module spec (§4.1) shows `POST /api/ar/customers` exists but nothing in any new spec subscribes to CRM's `lead.converted.v1` or `opportunity.closed_won.v1` to trigger it. The bridge is invisible in the specs.

**Severity:** High  
**Confidence:** 0.92  
**So What:** The gap between Party creation and AR customer creation needs an explicit owner. The simplest fix: CRM's `lead.converted.v1` payload includes `party_id`; AR subscribes and auto-creates a draft AR customer; Sales-Orders can then book against it. Alternatively, specify that the vertical's order-entry UI creates the AR customer before booking — but document this as a platform seam, not leave it to each vertical to discover independently.

---

### §F3 — Hold Enforcement Lives Nowhere

**Evidence:** `SHOP-FLOOR-GATES-MODULE-SPEC.md §8, invariant 8`:
> "Hold prevents operation start when active on that operation. Downstream — Production should check for active operation-scoped holds before allowing an operation to start. Platform Gates emits `hold.placed.v1`; Production is the enforcer, not Gates."

`bd-ixnbs-fireproof-platform-migration.md §B`:  
No Production extension for hold-checking. The only Production extension listed is manufacturing costing.

**Reasoning chain:** Shop-Floor-Gates owns the hold record. Production owns the operation execution. The safety invariant — "you cannot start an operation with an active hold" — straddles both. Gates can't enforce it (it doesn't own operation start). Production must enforce it, but no spec modifies Production to add this check. The Gates spec acknowledges this explicitly and defers it to "design detail for implementation bead." But there is no implementation bead for it in the decomposition plan, and the two options offered ("Production calls a GET endpoint" vs "Production subscribes to events") are architecturally different — one is synchronous and blocks operation start, the other is eventually consistent and cannot block. The module's core safety property is unspecified at the architectural level.

**Severity:** High  
**Confidence:** 0.90  
**So What:** The enforcement mechanism must be decided before either module is implemented. The synchronous option (Production calls `GET /api/shop-floor-gates/work-orders/:wo_id/holds?scope=operation&operation_id=X&status=active`) is simpler and consistent with how holds are expected to block work. Add this to the Production module's operation-start endpoint spec, and add a corresponding "consumed events" entry for `hold.placed.v1` so Production can maintain a local cache of active holds if the synchronous call creates latency concerns. Whichever option is chosen, it must exist in a spec before the implementation beads are cut.

---

### §F4 — BOM's On-Hand Input Is Asymmetric Between MRP and Kit Readiness

**Evidence:** `PLATFORM-EXTENSIONS-SPEC.md §1` (MRP):
> "The `on_hand` input is caller-supplied (not queried from Inventory automatically). Keeps the computation deterministic and auditable."

`PLATFORM-EXTENSIONS-SPEC.md §5` (Kit Readiness):
> "Pulls on-hand from Inventory (uses Inventory's availability query) rather than taking it as input like MRP does. Kit readiness is an operational check, so fresh data is the right default."

**Reasoning chain:** Both endpoints live in the same BOM module. Both compute inventory requirements against a BOM. But they have opposite conventions for the most important input. MRP: caller queries Inventory first, passes on_hand as a snapshot. Kit Readiness: the endpoint queries Inventory itself. The design rationale is legitimate for each endpoint in isolation (MRP = deterministic; Kit Readiness = fresh). But as a system, this creates:
- **Different failure modes**: MRP degrades gracefully if Inventory is slow (caller caches); Kit Readiness fails if Inventory is down
- **Inconsistent results**: Running MRP + Kit Readiness in sequence on the same BOM can produce different on-hand figures because Kit Readiness gets a fresher snapshot
- **Asymmetric authorization**: Kit Readiness implicitly holds a service-to-service auth relationship with Inventory; MRP does not
- **Testing asymmetry**: MRP tests are pure (no service dependency); Kit Readiness tests require a live Inventory

A vertical engineer using both endpoints will be surprised by the behavioral difference. Implementation teams will silently harmonize them, losing the original design intent.

**Severity:** Medium  
**Confidence:** 0.85  
**So What:** Either align the conventions (both caller-supplied, or both live-queried) or document the asymmetry explicitly in the BOM module README as a deliberate architectural choice, including the rationale and the failure-mode implications for each. The current spec mentions the difference but frames it as natural — an implementation bead engineer won't see the system-level inconsistency unless it's explicitly called out.

---

### §F5 — CRM→Sales-Orders Handoff Owns Nothing

**Evidence:** `CRM-PIPELINE-MODULE-SPEC.md §9`:
> "The handoff flow (opp close-won → SO create) can be implemented as an event subscriber on Sales-Orders side or as a manual operator action; either works."

`SALES-ORDERS-MODULE-SPEC.md §5.2` (consumed events): does not include `crm_pipeline.opportunity.closed_won.v1`

`CRM-PIPELINE-MODULE-SPEC.md §5.1` (events produced): `crm_pipeline.opportunity.closed_won.v1` payload includes `sales_order_id` (nullable — "if set")

**Reasoning chain:** The CRM spec punts to Sales-Orders ("or as an event subscriber on Sales-Orders side"). The Sales-Orders spec doesn't subscribe to CRM. `sales_order_id` on the opportunity is nullable and set "if set" — meaning it gets populated after-the-fact, not as part of the close-won action. The system has defined a clean handoff event but left the handoff bridge completely unowned. In Fireproof's workflow, a sales representative closes an opportunity as won, and then... something must create the Sales Order. Currently that something is undefined. Every vertical will build this bridge independently, and each implementation will have different semantics about what triggers SO creation. This is a cross-cutting orchestration concern that the platform should own.

**Severity:** Medium-High  
**Confidence:** 0.88  
**So What:** Add `crm_pipeline.opportunity.closed_won.v1` to Sales-Orders' consumed events, with behavior: "If payload includes a `sales_order_id`, link the existing SO. If not, create a draft SO pre-populated from the opportunity's party_id, estimated_value, and external_quote_ref." Mark SO creation as optional (the event triggers a draft, not a booking) to preserve the human confirmation step. This converts a manual vertical-specific workaround into a platform-owned seam.

---

### §F6 — AP Vendor Disqualification Mid-OP Lifecycle

**Evidence:** `PLATFORM-EXTENSIONS-SPEC.md §7` (AP extension, enforcement):
> "PO creation endpoint refuses when `vendor.qualification_status in (unqualified, disqualified)`"

`OUTSIDE-PROCESSING-MODULE-SPEC.md §5.2` (events consumed):
> Consumes `ap.po.approved.v1`, `ap.po.closed.v1`, `inventory.lot.split.v1` — does NOT consume `ap.vendor.disqualified.v1`

`PLATFORM-EXTENSIONS-SPEC.md §7` (events produced):
> `ap.vendor.disqualified.v1` is produced but has no documented consumers

**Reasoning chain:** AP's qualification gate correctly blocks new POs for disqualified vendors. But an OP order can exist in `issued` or `shipped_to_vendor` state with a PO already created, and the vendor subsequently gets disqualified. OP will continue its lifecycle (shipped → at_vendor → returned → review → closed) for a disqualified vendor because OP never receives the disqualification event. The `ap.vendor.disqualified.v1` event is produced by AP and consumed by nobody in the new spec set. For aerospace (Fireproof), vendor disqualification mid-OP is a serious compliance event — parts already at a disqualified vendor may need to be recalled. The platform emits the event but no module acts on it.

**Severity:** Medium  
**Confidence:** 0.82  
**So What:** Add `ap.vendor.qualification_changed.v1` to OP's consumed events with behavior: "If an active OP order exists for a vendor transitioning to `disqualified`, log a warning activity on the OP order and surface it in the OP list with a flag." Full automatic recall (e.g., auto-placing a hold via Shop-Floor-Gates) can be an overlay for aerospace; the platform's minimum response is surfacing the risk rather than silently continuing.

---

### §F7 — source_entity_type Is Unenumerated Across Three Modules

**Evidence:** `CUSTOMER-COMPLAINTS-MODULE-SPEC.md §3` (complaints table):
> `source_entity_type` (nullable; e.g. `sales_order/shipment/invoice/service_visit`)

`OUTSIDE-PROCESSING-MODULE-SPEC.md §3` (op_orders table):
> `source_entity_type` (work_order/collection_batch/livestock_batch/standalone)

`SHOP-FLOOR-GATES-MODULE-SPEC.md §3` (signoffs table):
> `entity_type` (canonical whitelist: work_order/operation/traveler_hold/operation_handoff/operation_start_verification)

**Reasoning chain:** Three modules independently define string-valued entity-type fields that reference cross-module entities. Each module documents examples but:
- Customer-Complaints treats `source_entity_type` as open/nullable with illustrative examples
- OP treats it as open with fixed examples but non-canonical
- Shop-Floor-Gates calls its whitelist "canonical" but only in field description prose, not in any shared schema

The Customer-Complaints module subscribes to `sales_orders.order.shipped.v1` and matches complaints by `source_entity_id` against the event's `sales_order_id` — implicitly requiring `source_entity_type = 'sales_order'`. If Fireproof uses `'sales_order'` and TrashTech uses `'so'` (abbreviated from a UI default), the subscription behavior silently does nothing for TrashTech's complaints. There is no platform-canonical vocabulary for cross-module entity type identifiers, so every module using this pattern will diverge over time.

**Severity:** Medium  
**Confidence:** 0.80  
**So What:** Define a platform-canonical entity type vocabulary in `contracts/entity-types.v1.json` (or equivalent). Modules that use a source/entity/target type string field reference this vocabulary. Values like `work_order`, `sales_order`, `invoice`, `shipment`, `operation` become canonical constants. This costs one artifact and prevents a category of silent integration failures.

---

## 3. Risks Identified

| Risk | Severity | Likelihood | Notes |
|------|----------|------------|-------|
| HuberPower cannot use manufacturing costing — no source for labor events | High | Certain if not fixed | §F1 |
| AR customer creation skipped; SO booking fails or is un-linked to Party | High | Likely without explicit spec | §F2 |
| Operation starts despite active holds — safety invariant not enforced | High | Possible | §F3 |
| BOM endpoint behavioral divergence surprises vertical engineers | Medium | Likely | §F4 |
| CRM close-won triggers no SO; each vertical builds its own bridge | Medium | Likely | §F5 |
| Vendor disqualification mid-OP unsurfaced; compliance gap for aerospace | Medium | Possible | §F6 |
| Cross-module entity type string divergence breaks event subscriptions silently | Medium | Likely over time | §F7 |
| Overlay service pattern underdefined; Fireproof's 3+ overlay services designed inconsistently | Medium | Certain if not addressed | §9 |
| Signoff/attestation pattern independently re-implemented 4+ times | Low-Med | Likely | §9 |
| Revenue recognition timing gap: shipment event ≠ AR invoice issuance | Low-Med | Possible | §9 |
| Training completion → competence assignment API call has no atomicity guarantee | Low | Possible | §9 |
| Daily sweep jobs accumulate across modules with no coordination | Low | Possible at scale | §9 |

---

## 4. Recommendations

| Priority | Recommendation | Effort | Expected Benefit |
|----------|---------------|--------|-----------------|
| P0 | Rename `shop_floor_data.labor.approved.v1` → `production.labor_entry.approved.v1`; document that verticals produce this from their labor capture code (§F1) | Low | Unblocks HuberPower costing; prevents silent zero-labor WO costs |
| P0 | Specify CRM→AR customer creation: add AR subscription to `crm_pipeline.lead.converted.v1`, or document the creation as a required platform seam with a defined trigger (§F2) | Low | Removes four independent vertical bridge implementations |
| P0 | Decide hold-check mechanism before Gates/Production beads; specify synchronous GET or event-cached check in a Production extension (§F3) | Low | The core safety invariant of Shop-Floor-Gates requires this decision |
| P1 | Publish `contracts/entity-types.v1.json` with canonical cross-module entity type vocabulary (§F7) | Low | Prevents divergent string conventions; enables reliable event subscription matching |
| P1 | Add `crm_pipeline.opportunity.closed_won.v1` to Sales-Orders consumed events with draft-SO creation behavior (§F5) | Low | Closes the handoff gap with a platform-owned seam |
| P1 | Add `ap.vendor.qualification_changed.v1` to OP consumed events; specify OP warning behavior on disqualification (§F6) | Low | Surfaces mid-OP vendor risk; prevents silent compliance gap |
| P2 | Document MRP vs Kit Readiness on-hand asymmetry in BOM README with failure mode implications (§F4) | Low | Prevents implementation-team confusion and silent behavioral divergence |
| P2 | Write an "Overlay Service Pattern" doc: port convention, data flow, write-back prohibition, UI join strategy | Low | Three+ overlay services will otherwise be designed inconsistently |
| P3 | Specify training completion → competence assignment atomicity: outbox or compensating event on API failure | Low | Prevents silent competence gaps after passing training |
| P4 | Monitor signoff/attestation duplication across Gates, OP, Quality-Inspection, Workforce-Competence; extract when count reaches 3 with proven pattern | Low | Incremental cost; extract when pattern is stable across modules |

---

## 5. New Ideas and Extensions

### Incremental

**Platform Entity Type Registry** — A single `contracts/entity-types.v1.json` enumerating all first-class cross-module entity identifiers (`work_order`, `sales_order`, `invoice`, `lot`, `shipment`, `complaint`, `operation`, `employee`, etc.). Each module using an entity-type string field references this file in its contract. CI validates no module uses an unregistered entity type string. Cost: one JSON file and one CI rule.

**Verified CRM→AR Customer Bridge** — AR subscribes to `crm_pipeline.lead.converted.v1` and creates a draft AR customer (name from Party lookup, status: draft). The vertical promotes to active when billing setup is complete. This replaces four vertical-specific implementations with one platform-owned seam.

### Significant

**`production.labor_entry.approved.v1` as a Platform Contract** — Define a canonical platform event for "an operator's labor record for a work order has been verified and is ready for costing." Any vertical's labor capture system (Fireproof kiosk, HuberPower time clock, RanchOrbit mobile app) emits this event. Production costing subscribes once to a stable contract rather than to a Fireproof-specific source. This normalizes the labor feed across all manufacturing verticals without requiring a platform Shop-Floor-Data module.

**Unified Signoff/Attestation Surface** — Defer until Gates, OP, and Quality-Inspection are all live. At that point, extract `signoffs` as a platform-level append-only attestation service with polymorphic entity refs. The cost of extraction is much lower before modules are entrenched in production usage.

### Radical

**Customer Lifecycle Orchestrator** — An explicit platform concept (thin module or documented event sequence) that owns the progression: CRM lead → Party creation → AR customer creation → Sales-Orders eligibility. Today this progression is implied by four modules each delegating the step on their right, creating the tri-identity gap in §F2. Making the lifecycle explicit as a platform-owned event sequence would prevent similar gaps as more modules join the customer-facing surface.

---

## 6. Assumptions Ledger

1. Fireproof's local kiosk/labor code will emit events on the NATS bus under some agreed name — assumed, not stated in any spec.
2. Production module currently has an "operation start" endpoint that can be modified to check holds — assumed present, not verified against the existing Production codebase.
3. AR module's `POST /api/ar/customers` is callable by other platform modules as a service-to-service call — assumed, not stated in the AR spec.
4. Multiple daily sweep jobs running concurrently on the same NATS bus do not create ordering hazards for event consumers — assumed benign at current scale.
5. The "overlay service" runs as a separate Rust service in Fireproof's deployment, not as an in-process library alongside platform modules — assumed from the architectural description across multiple specs.
6. `source_entity_type` string values are case-sensitive and lowercase by convention — assumed from examples, not specified anywhere.
7. Kit Readiness calling Inventory's API synchronously is acceptable latency for pre-production checks — assumed reasonable, not benchmarked.

---

## 7. Questions for Project Owner

1. **Labor events from non-platform sources**: When HuberPower implements manufacturing labor capture (not using Fireproof's kiosk), what event should they produce to trigger cost accumulation? Should the platform define a canonical `production.labor_entry.approved.v1` contract, or is the Fireproof-named event intentionally Fireproof-only?

2. **AR customer creation authority**: Is it acceptable for the platform (via CRM's lead conversion event) to auto-create AR customers, or must AR customer creation always be an explicit operator action? This determines whether §F2 has a platform-side solution or remains a vertical-side responsibility.

3. **Hold enforcement mechanism**: For Shop-Floor-Gates, is the preferred enforcement of "cannot start operation with active hold" a synchronous REST check (hard block), an event-based cache in Production (soft block), or a UI-only advisory (warning but no server-side block)? The safety level of the invariant depends on this choice.

4. **Overlay service scope**: Three+ specs delegate AS9100 specifics to a Fireproof overlay service. Is there a risk that the overlay becomes a second Fireproof monolith? Should there be one overlay service or one per platform module being extended?

5. **Revenue recognition timing**: When Sales-Orders emits `invoice.requested.v1` on shipment, is AR expected to auto-issue the resulting invoice (immediate revenue recognition) or hold it in draft for manual issuance? This policy affects every vertical using Sales-Orders + AR.

---

## 8. Points of Uncertainty

- **Production module's current operation-start API surface**: Whether Production already has an operation-start endpoint that Gates could integrate with cannot be verified from the specs alone. If it doesn't, §F3 is larger than described.
- **NATS subject naming for Fireproof-local events**: If Fireproof's local shop-floor-data service emits events on the shared NATS bus, `shop_floor_data.labor.approved.v1` may function for Fireproof even with no platform module owning that subject. Whether HuberPower could produce the same subject name from a different labor system is the real question.
- **AR-to-Party linkage**: Whether `ar_customers` has a `party_id` FK is not stated in the AR spec. If it does, §F2 is milder (Party and AR customer are already linked); if it doesn't, the customer identity triangle has no cross-reference at all.
- **Inventory availability query API**: Kit Readiness calls Inventory for on-hand data. Whether Inventory currently exposes a multi-item availability query endpoint (or whether Kit Readiness would need N individual calls per component) is not clear from the existing specs.

---

## 9. Agreements and Tensions With Other Perspectives

**Expected agreements (with likely other modes):**
- An Adversarial/Red-Team mode would almost certainly flag §F1 (severed labor feed) and §F3 (unenforced hold invariant) as the same critical gaps — they are obvious failure paths under intentional stress.
- A Data-Architecture mode would likely surface §F2 (tri-modal customer identity) and §F7 (entity type strings) from a normalization and referential integrity angle.
- A Security/Compliance mode would likely flag §F6 (vendor disqualification not propagated) as a compliance gap for both aerospace and any regulated vertical.

**Expected tensions:**
- A Minimalism/Lean mode might push back on §F5 (CRM→SO handoff) — "verticals should own this; the platform doesn't need to be paternalistic about every workflow step." Counter-argument: it's a structural gap that every vertical will fill independently with different semantics; platform consistency prevents fragmentation of what should be a common flow.
- A Lean mode might also resist the overlay service spec (Recommendations P2) as premature documentation. Counter-argument: three specs already assume the pattern exists; the cost of not specifying it is borne immediately by whoever implements the first overlay service.
- A Forward-Looking/Innovation mode might expand §F7 (entity type registry) into a full "cross-module entity graph" concept. This analysis deliberately keeps it incremental; the full graph concept is a significant architectural investment with uncertain ROI at this stage.

---

## 10. Confidence: 0.82

**Calibration note:** Findings §F1–F3 (critical/high severity) are based on unambiguous spec text — the gaps are clearly present and the cross-spec dependencies are explicit. Confidence on these three is 0.90+. Findings §F4–F7 (medium severity) rely on inferring what will happen during implementation based on the design choices made; actual implementation teams might resolve them differently without producing the exact failure mode described. The uncertainty is not in whether the seam exists, but in how consequential the consequences will be in practice. The overall 0.82 reflects high confidence in gap identification and moderate confidence in severity prediction for the medium findings.

---

*End of MODE_OUTPUT_F7.md*
