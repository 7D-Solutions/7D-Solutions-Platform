# MODE_OUTPUT_F3 — Counterfactual Reasoning Analysis
**Mode:** F3 — Counterfactual Reasoning  
**Analyst:** TealElk  
**Date:** 2026-04-16  
**Scope:** Fireproof → 7D Platform migration specs (bd-ixnbs)

---

## 1. Thesis

Counterfactual reasoning surfaces decisions that look reasonable in isolation but reveal hidden costs when compared against their live alternatives. This analysis takes five spec decisions and tests them against plausible reversals — and then goes further, using each counterfactual as a lens to locate the coupling seams, concurrency gaps, and integration assumptions that would have become visible if the alternative had been chosen. Several findings confirm the current design. Several reveal genuine problems: a signoff entity that belongs on the platform but is locked inside a manufacturing module, a concurrency invariant on blanket order releases that the spec describes as trivially solved but isn't, an unspecified linkage mechanism between Outside-Processing and Shipping-Receiving, a Party API that becomes an operational gate for lead conversion, and a CRM pipeline stage model that makes cross-tenant reporting structurally impossible at the platform level. Each finding traces a concrete bead-level consequence.

---

## 2. Top Findings

---

### §F1 — Signoff entity whitelist is trapped inside the wrong module

**Evidence:** `SHOP-FLOOR-GATES-MODULE-SPEC.md` §3 Data Ownership — `signoffs.entity_type` is a "canonical whitelist: work_order/operation/traveler_hold/operation_handoff/operation_start_verification". Same spec §11 Open Questions: "If Quality-Inspection wants signoffs on inspections, they embed their own."

**Counterfactual tested:** *What if holds + signoffs had been put on platform and only verification + handoff had remained in Fireproof* — the split the user originally proposed and then rejected?

**Reasoning chain:** Under the alternative, signoffs would have been a general platform attestation service. Under the current design, signoffs are a sub-table inside a module whose `cross-vertical applicability` field explicitly reads "Fireproof + HuberPower." That means:

- TrashTech and RanchOrbit do not use `shop-floor-gates` at all
- TrashTech and RanchOrbit therefore cannot use platform signoffs for any entity they want to attest (e.g. a waste manifest sign-off, a livestock health certificate)
- Quality-Inspection (platform module, used by all 4 verticals) cannot use platform signoffs — the spec already acknowledges it must "embed its own"
- Outside-Processing's vendor review outcome (`op_vendor_reviews`) is semantically a witnessed attestation but uses a different table/pattern from signoffs
- Customer-Portal complaint closure would benefit from a customer sign-off, but customer-complaints has no signoff concept at all

The manufacturing-centric entity whitelist is the problem. The user's original proposal to split holds + signoffs onto platform, leaving verification + handoff in Fireproof, would have produced a more general signing capability. The rejection of that split consolidated four concerns into one module and in doing so trapped the most reusable concept (witnessed attestation) inside a module that two of four verticals never use.

The spec itself notes the tension and proposes "revisit if the pattern repeats." It will repeat immediately — Quality-Inspection and Outside-Processing are both in scope for this migration, and both will independently invent `op_vendor_reviews` and `inspection_signoffs` tables that are structurally identical to `signoffs` but incompatible with it.

**Severity:** High  
**Confidence:** 0.90  
**So What?** Before the `shop-floor-gates` implementation bead: widen the `entity_type` whitelist to include `inspection` and `op_review` now while the schema is still unfrozen. The migration cost to extend an enum whitelist after the module ships is a contract breaking change requiring a v2 event schema. Adding two values to the whitelist now costs zero.

---

### §F2 — Blanket order release has an unspecified concurrency invariant

**Evidence:** `SALES-ORDERS-MODULE-SPEC.md` §8 Invariant 4: "SUM(releases.release_qty) + cancelled_qty <= blanket_line.committed_qty. Over-release is forbidden." §8 Invariant 5: "Blanket line `released_qty` = SUM of non-cancelled release.release_qty. Maintained by triggers or application-level update on release create/cancel."

**Counterfactual tested:** *What if blanket orders and standard sales orders were separate modules?*

**Reasoning chain:** If they were separate modules, the `released_qty` accounting would cross a module boundary and the problem would have forced a distributed-consistency answer. The authors would have been forced to choose between saga, two-phase commit, or eventual consistency — none trivial — and the problem would have been front-of-mind. By keeping them in the same module, the problem LOOKS solved ("it's all one service, one DB, easy") but the actual race condition is unchanged.

Two concurrent POST requests creating releases against the same blanket_order_line:

1. Request A reads `released_qty = 80`, `committed_qty = 100`. Remaining = 20. A wants 20. Check passes.
2. Request B reads `released_qty = 80`, `committed_qty = 100`. Remaining = 20. B wants 15. Check passes.
3. Request A inserts `release.release_qty = 20`.
4. Request B inserts `release.release_qty = 15`.
5. `released_qty` is now 115, over-committing by 15. Invariant 4 violated.

The spec says "triggers or application-level update" without specifying `SELECT FOR UPDATE` on the blanket_order_line row. At the transaction isolation levels typical in Postgres read-committed deployments, both reads in steps 1–2 see the pre-committed state. The invariant is broken.

This is not theoretical. Blanket order releases are the most frequent write path for manufacturing customers with long-term contracts — exactly Fireproof's actual customer use case. A `SELECT ... FOR UPDATE` on `blanket_order_lines` is the fix, but it must be specified in the implementation bead or the bead author will choose whichever pattern they find first.

**Severity:** High  
**Confidence:** 0.95  
**So What?** Add an explicit invariant note to the spec: "Release creation MUST acquire a row-level lock on the parent `blanket_order_line` (`SELECT ... FOR UPDATE`) before checking and updating `released_qty`. Application-level update only; no DB trigger." One sentence that prevents a subtle bug invisible in unit tests and only visible under load or in production.

---

### §F3 — OP return lacks a specified linkage mechanism at Shipping-Receiving's receipt time

**Evidence:** `OUTSIDE-PROCESSING-MODULE-SPEC.md` §5.2 Events Consumed: "shipping_receiving.shipment.received.v1 | If an inbound shipment references an OP order, create a matching return-event stub; operator completes details."

**Counterfactual tested:** *What if Outside-Processing were an extension of Shipping-Receiving rather than a standalone module?*

**Reasoning chain:** If OP lived inside Shipping-Receiving, the question "does this inbound receipt belong to an OP order?" would be answered by direct table lookup. As separate modules, it requires a cross-module convention — and the spec doesn't specify what that convention is.

Specifically: when a vendor ships back processed material, the receiving dock operator creates a receipt in Shipping-Receiving. That receipt needs to trigger OP's `return_event_stub` creation. The spec's event handler says "if an inbound shipment references an OP order." But:

- How does the Shipping-Receiving `shipment.received.v1` event indicate it references an OP order?
- Is there a `source_ref` or `po_number` field in the shipment receipt that the dock operator fills in at receiving time?
- If the operator forgets to enter the OP order reference, the event fires with no OP reference, no stub is created, and the OP order is stuck in `at_vendor` indefinitely.

The outbound direction is clean: OP emits `shipment.requested.v1` and Shipping-Receiving creates the outbound shipment with a back-reference to the OP ship-event. But the return direction requires the dock operator to associate the inbound shipment with the OP order at receipt time — and the spec doesn't specify the field on the receipt that carries this association, nor what happens when it's missing.

If OP were inside Shipping-Receiving, this would be a UI/UX concern: "select OP order for this receipt." As separate modules, it's an architectural spec gap.

**Severity:** Medium  
**Confidence:** 0.88  
**So What?** The OP spec needs a section on the return association mechanism: (a) Shipping-Receiving's inbound receipt endpoint should accept an optional `op_order_id` reference field; (b) OP's event consumer matches on this field; (c) if the field is absent, OP surfaces unmatched returns as alerts rather than silently not creating stubs. This is a cross-module contract change — it requires updating the Shipping-Receiving OpenAPI contract as well as the OP event consumer spec.

---

### §F4 — Lead conversion makes Party availability an operational gate for CRM

**Evidence:** `CRM-PIPELINE-MODULE-SPEC.md` §8 Invariant 1: "A lead can only transition to `converted` if `party_id` is set (either pre-existing or created during conversion via Party API call from the handler)." §10 Open Questions: "If Party enforces uniqueness by name or email and a duplicate exists, conversion reuses the existing Party. OK to rely on handler logic."

**Counterfactual tested:** *What if CRM-Pipeline had kept its own lightweight contact/company table rather than delegating entirely to Party?*

**Reasoning chain:** The spec's choice — Party as the sole contact record — is correct for data integrity. The counterfactual confirms this: CRM's own contact table would duplicate Party, diverge over time, and create reconciliation work. That reversal is bad. But applying counterfactual pressure surfaces the failure mode the current design inherits:

**Lead conversion is a synchronous call to an external service (Party) in the hot path of a sales rep workflow.** If Party is unavailable, `POST /api/crm-pipeline/leads/:id/convert` fails. The lead stays in `qualified` status. The sales rep cannot create the opportunity. No fallback, no retry-with-continuation, no "convert when Party comes back" saga.

The spec says "OK to rely on handler logic" for duplicate Party records. That phrase carries hidden complexity:

1. Party returns a "duplicate exists" response with the existing party_id → CRM uses it (happy path)
2. Party returns 503 → CRM's convert endpoint fails → rep retries manually
3. Party returns 200 with a new party_id → CRM uses it, but a duplicate Party record now exists

Path 3 is the dangerous one: if Party's deduplication is name+email-based and the lead has a slightly different email than the existing Party record (typo, alias), Party creates a second record. CRM links to the new one; AR might already be linked to the old one. The merge must happen in Party but nothing in CRM catches it.

**Severity:** Medium  
**Confidence:** 0.82  
**So What?** Two spec additions: (a) specify that `convert` must handle Party 5xx with a retriable error and must NOT partially apply state (if Party call fails, no state change on the lead); (b) add invariant: "If Party returns a 409/duplicate response, use the provided existing party_id and surface the match for operator confirmation before creating the opportunity." This makes duplicate-Party detection a user confirmation step rather than silent auto-merge.

---

### §F5 — Tenant-defined pipeline stages make platform-level CRM reporting structurally unresolvable

**Evidence:** `CRM-PIPELINE-MODULE-SPEC.md` §3: "pipeline stages are tenant-defined, NOT canonical." §5.1 Events: `crm_pipeline.opportunity.stage_advanced.v1` carries `from_stage_code` and `to_stage_code` — both are tenant-scoped strings.

**Counterfactual tested:** *What if pipeline stages had been canonical (platform-defined) with optional tenant label overrides — same as the status/severity pattern elsewhere?*

**Reasoning chain:** The spec's rationale for tenant-defined stages is sound: aerospace has 8-stage contract cycles; TrashTech has 3-stage route contracts; a canonical set would either be too long for simple cases or too short for complex ones. The decision is correct.

But the counterfactual reveals what you give up: every event carrying a stage transition now carries a tenant-opaque code. A downstream consumer (platform Reporting module, a multi-vertical dashboard, a future platform analytics extension) that wants to answer "what percentage of opportunities make it from first contact to proposal across all verticals?" cannot answer this question without knowing each tenant's stage taxonomy.

The spec's `GET /api/crm-pipeline/pipeline/summary` provides aggregate counts by stage. But there is no canonical concept of "early stage" vs. "late stage" vs. "closing stage" that survives across tenants. If platform Reporting ever wants to show a funnel view across the platform's tenant population, it needs either:

(a) A platform-level "stage type" canonical enum mapping tenant stages to (awareness/consideration/proposal/negotiation/commitment/won/lost) — absent from the spec  
(b) Each tenant to explicitly configure this mapping — creates admin burden  
(c) Per-tenant reporting only — fine for verticals, blocks cross-tenant platform analytics

The existing canonical enums (lead status, opp type, priority) in events allow platform-level consumers to reason about deal health without knowing tenant labels. Stage position does not.

**Severity:** Medium  
**Confidence:** 0.75  
**So What?** Add an optional `stage_type` canonical field to `pipeline_stages`: values (awareness/consideration/proposal/negotiation/commitment/won/lost/unknown; nullable). Tenants that care about cross-tenant or cross-product reporting populate it; others leave null. `stage_advanced` events carry `stage_type` alongside `stage_code`. Zero cost to add now as a nullable column; significant cost to retrofit after the first multi-vertical customer asks for consolidated pipeline views.

---

### §F6 — Kit Readiness and MRP have inverted Inventory coupling semantics with no failure mode specified for the real-time path

**Evidence:** `PLATFORM-EXTENSIONS-SPEC.md` §1 (MRP): "The `on_hand` input is caller-supplied. Keeps the computation deterministic and auditable." §5 (Kit Readiness): "Pulls on-hand from Inventory (uses Inventory's availability query) rather than taking it as input... fresh data is the right default."

**Counterfactual tested:** *What if both MRP and Kit Readiness used caller-supplied on-hand?*

**Reasoning chain:** The asymmetric design is intentionally correct — MRP is a planning artifact (deterministic), Kit Readiness is an operational gate (real-time). The counterfactual of forcing both to be caller-supplied would make Kit Readiness awkward: the caller would have to query Inventory first and pass the result immediately, which is just a manual version of what Kit Readiness does internally.

But the counterfactual makes visible what the spec omits: **Kit Readiness's behavior when the Inventory query fails.**

Kit Readiness is called immediately before a work order starts. If Inventory is unavailable:
- **Fail closed (return error):** Production is blocked. A running shop floor stops processing new WOs until Inventory recovers. In a 24/7 manufacturing context this is a production stoppage.
- **Fail open (warn but proceed):** Kit readiness check passes without verifying availability. A WO might start work on a material shortage, discovering mid-process that parts are missing.

Neither failure mode is specified. The spec says "sensible defaults" for policy knobs in v0.1 but doesn't classify this as a policy knob or specify the default behavior. MRP doesn't have this problem because it's caller-supplied. Kit Readiness does because it calls Inventory internally.

**Severity:** Medium  
**Confidence:** 0.85  
**So What?** Add to Kit Readiness spec: "If the Inventory availability query fails (5xx or timeout), Kit Readiness returns `{overall_status: 'check_unavailable', error: '...'}`. Callers (Production) must treat `check_unavailable` as a soft warning, not a pass. Default timeout: 500ms. Verticals requiring fail-closed behavior configure this via per-tenant policy (v0.2 scope)." Specifying this now prevents two different implementers choosing incompatible defaults.

---

### §F7 — Manufacturing costing as a Production extension aggregates events from three other modules, which is a ledger pattern, not a production concern

**Evidence:** `PLATFORM-EXTENSIONS-SPEC.md` §4 (Manufacturing Costing) Consumed events: `shop_floor_data.labor.approved.v1` (from Shop-Floor-Data — a Fireproof-local module), `inventory.lot.issued.v1` (Inventory), `outside_processing.order.closed.v1` (Outside-Processing).

**Counterfactual tested:** *What if Manufacturing Costing were a standalone `costing` module rather than a Production extension?*

**Reasoning chain:** The spec's rationale: "Production already owns work orders, operations, time entries, and workcenter cost rates. The composition engine belongs with the data it composes."

This holds for labor. But the consumed events show Production subscribing to events from Shop-Floor-Data (Fireproof-local), Inventory, and Outside-Processing. Production is being asked to become a cost-aggregation ledger that listens to the entire platform for cost-producing events. As new cost types appear (tooling costs, setup costs), Production grows as the aggregator even when the cost source has nothing to do with production execution.

More concretely: `shop_floor_data.labor.approved.v1` is Fireproof's kiosk-driven labor capture — it is explicitly a Fireproof-local module. HuberPower does not use Shop-Floor-Data. HuberPower's operators log time directly via Production's time-entry endpoints. So how does HuberPower's labor get costed?

The extension's labor costing path is wired to `shop_floor_data.labor.approved.v1`. Production's own time entries do not appear as a cost trigger. This means HuberPower's manufacturing costing — a second-vertical use case — has no labor cost trigger event at all. The extension as written only works for Fireproof.

**Severity:** Medium  
**Confidence:** 0.83  
**So What?** Two paths needed: (a) for kiosk-based verticals (Fireproof): `shop_floor_data.labor.approved.v1` → cost posting; (b) for direct-entry verticals (HuberPower and others): `production.time_entry.approved.v1` → cost posting. Path (b) requires adding a `time_entry.approved.v1` event to Production's spec. Without this, HuberPower's manufacturing costing is unimplemented at launch. This is a bead blocker for the manufacturing costing extension unless Production's time-entry approval already emits a compatible event.

---

### §F8 — NCR's absence creates an unacknowledged gap in the Customer-Complaints downstream resolution path for non-Fireproof verticals

**Evidence:** `CUSTOMER-COMPLAINTS-MODULE-SPEC.md` §1 Non-Goals: "Own corrective-action workflow when a complaint triggers a CAPA (CAPA is Fireproof-only per user ruling)." §10 Open Questions: "Platform complaints don't have [a CAPA] FK. Fireproof's overlay stores the link on Fireproof's CAPA side."

**Counterfactual tested:** *What if a stripped-down nonconformance module (without QMS formality) had been placed on platform despite the ruling?*

**Reasoning chain:** The user's ruling that QMS stays in Fireproof is correct when "QMS" means ISO 9001/AS9100 workflow formality. But the ruling conflated the base concept (internal investigation record tied to a quality event) with the aerospace-specific formality surrounding it.

A stripped-down platform `nonconformance` module — entity ID, type, disposition (use-as-is/scrap/rework/return), status, resolved-by — would have let Fireproof layer AS9100 formality on top via overlay, HuberPower track generator overhaul defects, TrashTech log spill-event incidents, and Customer-Complaints reference a downstream investigation record.

Under the current design: Customer-Complaints closes with `action_taken` text in a resolution record. "Action taken" is a free-text field. If HuberPower wants to tie a customer complaint resolution to a formal internal investigation, there is no canonical platform record to link to. Each vertical invents its own.

The ruling was right to keep AS9100 formality in Fireproof. But the seam between "customer complaint received" → "internal investigation triggered" → "investigation resolved" → "complaint closed with proof" is now unspecified for HuberPower, TrashTech, and RanchOrbit. Fireproof uses overlay. Others have no path.

**Severity:** Low (pre-launch Fireproof-only context) / Medium architectural debt  
**Confidence:** 0.70  
**So What?** No code change required now. Add a note to Customer-Complaints §9 Cross-module integration: "Non-Fireproof verticals that need complaint-resolution linked to a formal internal investigation record should consider a future platform `quality-events` module (a stripped-down nonconformance without QMS formality) when a second vertical demonstrates the need." This preserves the ruling while documenting the gap so it's not rediscovered at HuberPower's onboarding.

---

## 3. Risks Identified

| Risk | Severity | Likelihood | Finding |
|------|----------|------------|---------|
| Over-release race on blanket orders causes over-commitment in production | High | High (concurrent release creation is a normal use case for manufacturing customers) | §F2 |
| Signoff pattern duplicated 3+ times before platform extraction, creating incompatible tables | High | High (QI and OP both need it in this migration wave) | §F1 |
| OP return stubs silently not created when dock operator doesn't associate receipt to OP order | Medium | Medium (dock workflow gap; no prescribed field in SR receipt) | §F3 |
| Party service down during peak sales blocks all lead conversions; duplicate Party records created silently | Medium | Low (SR unavailability is low, but blocking when it happens; duplicates are higher) | §F4 |
| Multi-vertical reporting blocked by tenant-opaque stage codes | Medium | High (happens when second vertical onboards) | §F5 |
| HuberPower manufacturing costing has no labor trigger event | Medium | High (HuberPower doesn't use shop_floor_data) | §F7 |
| Kit Readiness silent behavior under Inventory failure blocks production starts | Medium | Low (Inventory unlikely to be down, but not zero) | §F6 |
| Non-Fireproof verticals have no canonical investigation record to link to complaint resolution | Low/Medium | High (emerges when HuberPower integrates) | §F8 |

---

## 4. Recommendations

| Priority | Finding | Action | Effort | Benefit |
|----------|---------|--------|--------|---------|
| P0 | §F2 | Add explicit `SELECT ... FOR UPDATE` requirement to blanket release creation spec before implementation bead | Low (one sentence in spec) | Prevents silent over-release invariant violation in production |
| P1 | §F7 | Add `production.time_entry.approved.v1` event to Production spec; add as second labor cost trigger in manufacturing costing extension | Medium (Production spec update + event contract) | HuberPower manufacturing costing works at launch |
| P1 | §F1 | Extend signoff entity_type whitelist to include `inspection` and `op_review` before shop-floor-gates ships | Low (additive enum values) | Avoids v2 contract break later; enables QI and OP to share platform signoffs |
| P2 | §F3 | Add optional `op_order_id` field to Shipping-Receiving inbound receipt; specify OP alert behavior when association is missing | Medium (cross-module contract change) | OP return stubs reliably created; no permanently stuck OP orders |
| P2 | §F5 | Add nullable `stage_type` canonical field to `pipeline_stages` table and `stage_advanced` event | Low (additive column) | Enables cross-tenant pipeline funnel reporting without breaking current design |
| P2 | §F6 | Specify Kit Readiness failure mode under Inventory 5xx/timeout | Low (one section addition to spec) | Prevents incompatible default behaviors in different bead implementations |
| P3 | §F4 | Specify Party API failure handling and duplicate-response behavior on CRM `convert` endpoint | Low (spec note) | Prevents stuck leads and silent Party duplicate creation |
| P4 | §F8 | Document the non-Fireproof investigation-record gap in Customer-Complaints integration notes | Low (documentation) | Sets expectation for HuberPower onboarding, prevents gap being rediscovered |

---

## 5. New Ideas and Extensions

**Incremental:**

- **Stage type enum on pipeline stages** — The optional nullable `stage_type` field on `pipeline_stages` is a single additive column that costs nothing in v0.1 and unlocks platform-level funnel analytics. Recommend adding in v0.1 even if no tenant configures it initially.

- **OP return association field on Shipping-Receiving receipt** — The `source_ref` pattern (already used elsewhere in platform) extended to carry `op_order_id` as an optional typed field. This is a one-column addition to SR's receipt model that closes the OP return linkage gap.

- **`SELECT FOR UPDATE` advisory in bead templates** — Given that the blanket order race is a classic pattern that will recur (inventory reservation checks, commitment tracking on any parent-child quantity relationship), the bead template should include a checklist item: "Does this endpoint modify a running aggregate on a parent record? If yes, specify row-level lock."

**Significant:**

- **Blanket order as a pattern** — The committed-qty + released-qty accounting model in blanket orders is structurally identical to what a future `contracts` or `framework-agreements` module would implement (RanchOrbit multi-year breeding contracts, TrashTech multi-year hauling contracts). Consider naming the pattern now in the spec even if not generalizing it. When the second vertical needs it, the seam is already documented.

- **Signoff service extraction** — When QI and OP both inevitably create their own signoff tables, extract to a shared `platform.signing` capability with the same entity_type whitelist pattern. Zero code now; flag as a known future bead.

**Radical:**

- **Cost event bus vs. cost-inside-Production** — Long-term, manufacturing costing inside Production means Production listens to events from 3+ other modules. A lightweight `cost-ledger` module that all platform modules post cost events to would centralize GL posting for all cost-type transactions and simplify per-module GL posting logic. Worth evaluating when overhead allocation requirements (currently deferred) prove complex enough to warrant it. Not for v0.1, but the pattern should be on the radar.

---

## 6. Assumptions Ledger

1. Postgres read-committed isolation is the default deployment. If serializable isolation is used, §F2's race is prevented automatically — but serializable is not mentioned anywhere in the specs and should not be assumed.

2. NATS durable subscribers are used for Fireproof overlay event consumption. The overlay pattern's reliability depends on durable subscriptions; this is not specified in the overlay architecture docs.

3. HuberPower runs multi-operation work orders and needs manufacturing costing. If HuberPower's manufacturing is simpler (no costing required at launch), §F7 is low priority.

4. The platform intends to support cross-tenant analytics at some future point. If analytics is always per-tenant vertical, §F5 is low priority.

5. Party module has synchronous create-or-find semantics for the lead conversion path. This is not confirmed from the Party spec (not reviewed for this analysis). If Party only has create (no find-or-create), the duplicate Party risk in §F4 is higher.

6. Production currently does not emit a `time_entry.approved.v1` event. If it does, §F7 is already partially solved for HuberPower.

---

## 7. Questions for Project Owner

1. **Blanket order concurrency:** Is `SELECT ... FOR UPDATE` acceptable, or is optimistic concurrency (version column check) preferred? The choice affects performance under high-volume blanket release scenarios (e.g. Fireproof's customer releasing against a long-term contract weekly).

2. **Signoff entity whitelist:** Is there a specific reason QI and OP review patterns shouldn't share the platform signoffs table now? The open-question wording ("revisit if pattern repeats") reads like a scope decision, not a design decision — and the pattern already repeats within this migration.

3. **HuberPower labor costing path:** Does HuberPower have kiosk-driven labor capture or do operators log time directly via Production's time-entry endpoint? The answer determines whether the manufacturing costing extension needs a second labor trigger event before launch.

4. **Party find-or-create semantics:** Does Party's contact/company creation endpoint support idempotent find-or-create by email? If not, CRM lead conversion will create duplicate Party records on transient failures and on slightly inconsistent email addresses.

5. **Kit Readiness failure mode:** Fail-closed (error on Inventory unavailable, blocking WO start) or fail-open (warn but proceed)? This is a production floor policy decision, not an implementation detail, and different verticals likely want different defaults.

---

## 8. Points of Uncertainty

- Whether Shipping-Receiving's receipt payload currently has a typed optional `source_ref` or `op_order_id` field is not confirmed from the reviewed specs. §F3's fix depends on this — if SR already has an extensible source_ref, the change is additive; if not, it's a new field requiring a contract minor version bump.

- Whether Production currently emits a `time_entry.approved.v1` event is not confirmed. If it does, §F7 may already be handled.

- The Party module's deduplication semantics (by email? by name? manual resolution required?) are not confirmed from the reviewed specs. §F4's risk severity depends directly on this.

- The `shop_floor_data.labor.approved.v1` event — the manufacturing costing extension's labor cost trigger — is emitted by Shop-Floor-Data, which is a Fireproof-local module (not platform scope per the ruled spec). Whether the platform costing extension is intended to consume a Fireproof-local event, or whether this is an error in the extension spec, is ambiguous.

---

## 9. Agreements and Tensions with Other Perspectives

**Expected agreements with Systems-Thinking (F7):**
- The OP → Shipping-Receiving return linkage gap (§F3) is likely visible as a missing interface specification from a systems coupling perspective.
- The manufacturing costing event aggregation concern (§F7) likely appears as a cohesion violation from a systems boundary view.

**Expected agreements with Devil's Advocate (F8):**
- The blanket order concurrency race (§F2) should appear under adversarial testing of invariant enforcement.
- The Party availability dependency (§F4) should appear as an operational coupling concern.

**Expected tensions with First-Principles (F1) or Pragmatist (F9):**
- §F5's recommendation to add `stage_type` to pipeline stages may be challenged as premature optimization for a problem that doesn't exist at Fireproof-only launch. The tension is "zero cost to add now vs. don't design for hypothetical requirements." Counter: platform analytics is an explicit platform value proposition, not a hypothetical, and the cost of adding a nullable column now is genuinely zero.
- §F7's concern about costing as ledger may be viewed as over-engineering if HuberPower's manufacturing scope at launch is narrow. Counter: the labor cost trigger problem (kiosk-only event) is a launch blocker for the second vertical, not a future concern.

**Where counterfactual reasoning failed:**
- The §F8 (NCR gap) finding is the weakest: the user ruling is well-reasoned, and the gap is acknowledged in the specs. Counterfactual pressure does not change the conclusion — just documents what was traded away.
- The standalone-OP-vs-SR-extension counterfactual confirms the current design is correct. Shipping-Receiving's domain is physical movement; OP's domain is service work. These are genuinely different.

---

## 10. Confidence: 0.82

**Calibration note:** High confidence on §F1 (signoff entity whitelist — mechanical architectural consequence) and §F2 (blanket order race — deterministic concurrency bug given Postgres default isolation). High confidence on §F7 (labor cost trigger missing for HuberPower — traceable to the Fireproof-local module dependency). Medium confidence on §F3 (SR return linkage — depends on SR's current receipt schema, which was not reviewed directly), §F5 (CRM reporting gap — depends on whether cross-tenant analytics is a real near-term requirement), and §F6 (Kit Readiness failure mode — depends on operational SLAs). Lower confidence on §F8 (NCR gap) — the ruling is correct and the finding is more of a documentation note than a spec change recommendation.
