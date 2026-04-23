# MODE_OUTPUT_L2 — Debiasing Analysis

**Mode:** L2 — Debiasing  
**Specs reviewed:** bd-ixnbs migration plan + 5 new module specs + 7 extensions spec  
**Date:** 2026-04-16  
**Analyst:** RainyRiver

---

## 1. Thesis

This spec set was drafted by one author (LavenderWaterfall) in a single-day working session with the user. The result is internally consistent and well-structured — but that consistency is itself a debiasing signal: a single author produces coherent work by unconsciously resolving ambiguities in their own favor, anchoring to familiar shapes, and accepting reasonable-sounding user rulings without stress-testing them. I found eight concrete bias patterns: aerospace domain vocabulary leaking into platform modules despite explicit rejection; the AR template being cargo-culted onto every spec including sections it renders poorly; the shop-floor-gates 4-way consolidation justified by proximity rather than cohesion; planning-fallacy framing on three "small extensions" that will immediately hit their stated deferrals; the CRM-Pipeline cross-vertical claim resting on assertion rather than evidence; an unacknowledged async consistency gap in the Sales-Orders booking flow; vertical-specific domain vocabulary embedded in OP's source_entity_type; and an authority-bias acceptance of the "quoting stays in Fireproof + opaque ref" ruling without stress-testing the downstream gap it creates.

---

## 2. Top Findings

### §F1 — Aerospace Domain Vocabulary Hardcoded Into Platform Modules
**Bias type:** Pattern matching to ISO/AS9100 frame  
**Evidence (multiple specs):**

**1a. SHOP-FLOOR-GATES-MODULE-SPEC.md, §3, `operation_start_verifications` table:**  
Three specific verification columns: `drawing_verified`, `material_verified`, `instruction_verified`. This is the AS9100 "setup packet review" checklist verbatim. The three checks map directly to AS9100 § 8.5.1 (control of production — drawing, material, work instruction confirmation before first operation). HuberPower's pre-operation checks for power generation equipment will be different (torque calibration certs, environmental parameters, safety lockout status). RanchOrbit has no analog at all.

**1b. SHOP-FLOOR-GATES-MODULE-SPEC.md, §3, `signoffs` table:**  
Canonical signoff roles: `quality/engineering/supervisor/operator/planner/material`. This is an aerospace shop's org chart. TrashTech's operational roles (dispatcher, compliance officer) and HuberPower's roles (commissioning engineer, plant safety officer) do not map. The roles are labeled "canonical" — platform-owned and not addable by tenants (only renameable). This means HuberPower and TrashTech will need to repurpose canonical aerospace roles, which creates confusion and erodes the canonical/display-label separation the platform depends on for event routing.

**1c. OUTSIDE-PROCESSING-MODULE-SPEC.md, §3, `op_return_events` table:**  
`cert_ref` appears on *every* return event as a standard field. Certificate of Conformance is a formal aerospace/defense procurement requirement (mandated by AS9100, military specs). TrashTech receiving processed waste back from a hazmat facility does not receive a CoC. RanchOrbit receiving processed livestock does not receive a CoC. This field will be null for ~50% of verticals but is embedded as a first-class column on a platform table.

**1d. OUTSIDE-PROCESSING-MODULE-SPEC.md, §2, `op_re_identifications` table:**  
The re-identification concept — recording that material's identity changed after vendor processing — is an aerospace document-control formality (material becomes "heat-treated aluminum per AMS 2770" after going out as "raw aluminum 7075-T6"). The spec presents it as cross-vertical ("any vertical where processing changes material identity"), but for TrashTech and RanchOrbit the concept is at most a loose analogy with very different semantics, traceability requirements, and actors. It's not obviously "the same thing."

**1e. CUSTOMER-COMPLAINTS-MODULE-SPEC.md, §3, `complaints` table, `source` enum:**  
Canonical source values include `letter`. In an AS9100 context, a "customer concern letter" is a formal document with a defined 8D/corrective-action response expectation. In other industries, complaints arrive by email, phone, or portal — not formal letters. This source value is an aerospace artifact in a platform enum.

**Reasoning chain:** Despite the explicit user ruling "no ISO-like QMS features for platform," the author systematically carried aerospace vocabulary into platform specs wherever the source module originated in Fireproof's aerospace codebase. The mechanisms are subtle — not explicit QMS references, but domain vocabulary in table columns, canonical enum values, and field sets that encode aerospace assumptions.

**Severity:** High  
**Confidence:** 0.9  
**So What:** Before writing implementation beads for shop-floor-gates, OP, and customer-complaints: replace the hardcoded `drawing_verified`/`material_verified`/`instruction_verified` columns with a configurable checklist table; audit every canonical signoff role against non-aerospace verticals; remove `letter` from the complaints source enum or reclassify as `formal_written`; reconsider whether `cert_ref` should be a first-class platform column or a Fireproof overlay field.

---

### §F2 — Sales-Orders Async Reservation Consistency Gap
**Bias type:** Confirmation bias (inconvenient complication glossed over)  
**Evidence:**  
SALES-ORDERS-MODULE-SPEC.md, §5.1 Events Produced, §5.2 Events Consumed, §8 Invariants, §10 Open Questions:

The booking flow is:
1. `POST /orders/:id/book` → SO transitions `draft → booked` → emits `reservation.requested.v1` per line
2. Inventory asynchronously emits `reservation.confirmed.v1` or `reservation.rejected.v1`
3. Sales-Orders "Mark SO line as stock-confirmed"

Invariant #8 says: "Cannot edit lines of a booked SO." But there is no invariant, state, or event that describes what the SO's effective status is *between booking and reservation response*. Questions that are unaddressed:

- If a line gets `reservation.rejected.v1`, is the SO still `booked`? Or does it transition to a partial/backorder state?
- Can the `book` endpoint be called before the prior booking's reservation dance has resolved? (Retry double-book scenario)
- What is the order UI supposed to show during the async window?

The Open Questions section mentions "stay in `booked` with line-level `stock_status` column" as a recommendation — but that column does not appear in the `sales_order_lines` data model. The column is required to implement the described behavior, but it was not added to the schema.

**Reasoning chain:** The author knew this was messy (it surfaces in Open Questions), but presented the booking → reservation handoff as a solved design decision when the state model is incomplete. The confirmation bias pattern: "the event is emitted; downstream handles it" — deferring the consistency semantics to implementation.

**Severity:** High  
**Confidence:** 0.85  
**So What:** Add `stock_confirmation_status` enum (`pending_confirmation / confirmed / rejected / partial`) to `sales_order_lines`. Define explicitly what SO state and operations are permitted during the pending window. Define whether the `in_fulfillment` transition triggers on booking or on all-lines-confirmed.

---

### §F3 — Shop-Floor-Gates 4-Way Consolidation by Proximity, Not Cohesion
**Bias type:** Availability bias (familiar from Fireproof) + motivated reasoning (avoid "too many small modules")  
**Evidence:**  
SHOP-FLOOR-GATES-MODULE-SPEC.md, §1 Mission:

> "All four are operational controls — they govern *whether* and *how* work flows through the shop"

Source modules consolidated: `traveler_hold/` (~1,050 LOC), `operation_handoff/` (~910 LOC), `operation_start_verification/` (~1,760 LOC), `signoff/` (~530 LOC). These are four separate modules in Fireproof with different actors, different frequencies, and different lifecycle owners:

- Traveler holds: placed by quality/engineering/material teams; can span hours to days
- Operation handoffs: placed by production operators; resolve in minutes
- Start verifications: pre-operation checklist; resolves before first piece runs
- Signoffs: attestation records; append-only, no lifecycle to manage

The justification "they all gate work flow" is a thematic similarity, not a DDD cohesion reason. By the same logic, "invoices and payments both relate to money" would justify merging AR and Payments.

The platform standard is one Rust module = one Cargo crate = one service = one Postgres DB. Merging four concerns into one service creates a larger surface, more event types, more tables, and more role-gate combinations in one codebase. Fireproof kept them separate because they evolved separately. The merger is not obviously better — it looks tidier in a spec table.

The spec itself acknowledges the tension in Open Questions §11: "Signoff cross-module — if Quality-Inspection wants signoffs on inspections, they embed their own. Revisit if the pattern keeps repeating across modules — could extract to a platform-level signing service." This is the author half-noticing the consolidation may have been premature.

**Reasoning chain:** The author knew Fireproof had four modules here and consolidated them into one new spec, likely to avoid having four "small module" spec documents. The availability bias drove the initial grouping; motivated reasoning justified it with "they're all gates."

**Severity:** Medium  
**Confidence:** 0.7  
**So What:** Before decomposing implementation beads, explicitly decide: should signoffs become a standalone platform-level signing service (the author's own Open Question)? Should operation-start-verifications be a Production extension rather than a Gates concern? This merger deserves a deliberate architectural choice, not a convenience default.

---

### §F4 — Planning Fallacy on Three "Small Extensions"
**Bias type:** Planning fallacy  
**Evidence (PLATFORM-EXTENSIONS-SPEC.md, three extensions):**

**4a. Manufacturing Costing without overhead = not actually costing**  
§4 Production Extension, consumed events section: "Overhead: configurable allocation rules per tenant (time-based or material-based); deferred to v0.2."

The Work Order Cost Summary table has an `overhead_cost_cents` column but no event that feeds it until v0.2.

In standard manufacturing accounting (GAAP, IFRS), manufacturing cost has three components: direct materials, direct labor, and manufacturing overhead. A cost summary showing zero overhead is not a manufacturing cost — it's a direct cost report. Any attempt to value WIP inventory or compute cost of goods manufactured using this module at v0.1 will produce materially wrong numbers. Calling this "manufacturing costing" when it deliberately excludes overhead creates false expectations. For Fireproof (aerospace contracts often require cost visibility), this gap will be hit on the first WO close attempt.

**4b. AP Supplier Eligibility = a flag, not a process**  
§7 AP Extension: adds `qualification_status`, `qualification_notes`, `qualified_by`, `qualified_at`, `preferred_vendor` columns + one audit table.

For Fireproof (AS9100 compliance), supplier qualification is a documented procedure: survey responses, quality system evidence, approval committee sign-off, periodic re-evaluation, performance scoring. The platform spec delivers a boolean-equivalent flag with a free-text notes field. The enforcement blocks PO creation for disqualified vendors — but who approves qualification, by what criteria, with what documentation? All of this is deferred to "verticals build their own overlay."

The risk: both Fireproof and HuberPower will independently build local qualification workflows on top of this flag, diverging into two implementations of the same process. A "small extension" that two verticals will immediately extend is not actually small.

**4c. MRP single-point = academic exercise**  
§1 BOM Extension: "Lead-time-based time-phased MRP is out of scope for v0.1. Current explosion is single-point."

The plan doc describes MRP as "time-phased material requirements: demand × BOM × on-hand → **what to buy/make and when**." The v0.1 spec delivers "demand × BOM × on-hand → what to buy/make" — the "when" (the scheduling answer, which is the primary planning value) is explicitly deferred. For any vertical that has asked for MRP, the "when" is the point. A single-point explosion is a useful building block but should be named "BOM net requirements" rather than "MRP."

**Severity:** High (4a critical for financial integrity; 4b/4c high for under-delivery)  
**Confidence:** 0.9  
**So What:** For manufacturing costing: either rename v0.1 to "Direct Cost Accumulation" or define a v0.1 overhead allocation approach (even a flat-rate percentage per tenant). For AP eligibility: decide whether the platform delivers a flag (rename to "vendor approval gate") or a process (scope accordingly). For MRP: rename the extension to "BOM net requirements" and revise the plan doc to match what's actually being built.

---

### §F5 — CRM-Pipeline Cross-Vertical Test Asserted, Not Demonstrated
**Bias type:** Availability bias (Fireproof has CRM; therefore platform needs CRM)  
**Evidence:**  
CRM-PIPELINE-MODULE-SPEC.md, preamble: "All verticals — B2B sales motion applies to Fireproof (aerospace contracts), HuberPower (power-gen equipment), TrashTech (commercial waste contracts), RanchOrbit (livestock/breeding services)"

The cross-vertical test requires "two or more verticals would plausibly use this." Examining the four:

- **Fireproof:** Aerospace manufacturing with known large customers. CRM pipeline exists in Fireproof now. Clear yes.
- **HuberPower:** "In-house manufacturing for power generation." In-house manufacturing typically serves the parent organization's plants. There is likely no outbound sales team, no external lead funnel, no opportunity stage management. This is not a B2B sales-to-external-customers context.
- **TrashTech:** Commercial waste hauling. Sales cycles for commercial waste contracts are real, but waste hauling companies typically use external CRM (Salesforce, HubSpot) precisely because their sales motion is territory-based with a field team — not inside the ERP.
- **RanchOrbit:** Ranching and livestock management. "Sales" here is livestock auctions, direct processor sales, and breeding bookings — closer to commodity transactions than B2B pipeline management.

Only Fireproof clearly needs this. The author assembled the other three verticals to clear the cross-vertical bar, but the actual use cases are thin.

**Reasoning chain:** CRM was already built in Fireproof (3,500 LOC). The author was motivated to migrate it to platform. The cross-vertical examples are plausible-sounding but not verified against actual stakeholder needs for the other three verticals.

**Severity:** Medium  
**Confidence:** 0.75  
**So What:** Before implementation beads enter the pool: explicitly ask whether HuberPower and RanchOrbit have a sales pipeline use case. If the answer is no, CRM-Pipeline stays in Fireproof per the user's own cross-vertical ruling. This would eliminate a full module (~3,500 LOC migration + platform code) from scope.

---

### §F6 — AR Template Over-Applied; Missing Structural Sections in All New Specs
**Bias type:** Anchoring (AR-MODULE-SPEC.md as the template)  
**Evidence:**

**Over-applied AR pattern — label tables in Shop-Floor-Gates:**  
SHOP-FLOOR-GATES-MODULE-SPEC.md, §3: Eight separate label tables: `hold_type_labels`, `hold_scope_labels`, `hold_release_authority_labels`, `hold_status_labels`, `handoff_initiation_labels`, `handoff_status_labels`, `verification_status_labels`, `signoff_role_labels`.

The per-tenant display label pattern was invented in AR for billing statuses that customers see on invoices — where one tenant's "Issued" is another's "Outstanding." Shop-floor gates are used internally by production operators, quality engineers, and supervisors. These users are employees of the tenant, operating in one consistent internal vocabulary. The business case for letting a tenant rename an "active hold" to a custom display label in a shop floor system is far weaker than in a customer-facing billing context. Eight label tables adds 8 tables, 16+ endpoints, and non-trivial maintenance for a use case that may never be exercised.

**Under-applied AR pattern — missing sections in every new spec:**  
AR-MODULE-SPEC.md has these sections that no new spec includes:

- **Testing Strategy** (AR §11): Unit/integration/contract/e2e/invariant breakdown. Absent from all 5 new module specs. For modules with non-trivial state machines (OP with 8 states and complex partial-return flows; Customer-Complaints with conditional transitions), the absence means the implementation bead author starts with no testing charter.
- **Error Taxonomy** (AR §9): HTTP status codes, retry safety, business rule violations vs. transient failures. New specs define invariants but not what HTTP status/error code a violation produces.
- **Versioning** (AR §12): How breaking changes are handled for this specific module. All new specs omit this entirely.
- **Background job mechanics:** AR describes nightly jobs at 2AM UTC and 15-minute retry loops. Sales-Orders has a daily `blanket.expired.v1` sweep; Customer-Complaints has a daily `complaint.overdue.v1` sweep. Neither spec defines frequency, failure mode, or idempotency for missed runs.

**Reasoning chain:** The author copied AR's section numbering as the template but applied it selectively — keeping financial invariant patterns (integer cents, row-level tenant_id) everywhere, and the label-table pattern everywhere, while dropping the sections that were less visible in the template (testing strategy, error taxonomy, versioning). The result is specs that look like AR but lack AR's operational completeness.

**Severity:** Medium  
**Confidence:** 0.88  
**So What:** For shop-floor-gates: audit which canonical enums actually need tenant-facing display customization vs. purely internal operational vocabulary; remove label tables for the latter. For all new specs: add a brief testing strategy section and a background job section before bead decomposition.

---

### §F7 — Vertical-Specific Domain Vocabulary Embedded in Platform OP
**Bias type:** Availability bias (author drew from Fireproof + known vertical use cases)  
**Evidence:**  
OUTSIDE-PROCESSING-MODULE-SPEC.md, §3, `op_orders` table, `source_entity_type` field description:

"(work_order/collection_batch/livestock_batch/standalone)"

`collection_batch` is TrashTech vocabulary. `livestock_batch` is RanchOrbit vocabulary. These are domain terms from two specific verticals that have not yet built on this platform. They appear as example values in what is labeled a canonical-ish field.

The spec note says "for non-manufacturing source types, the ID is opaque to platform." But listing `collection_batch` and `livestock_batch` as named values gives them a false canonical status — future verticals may take these as the "platform-approved" terms for their concepts, even though the platform has no authority over those domains.

Additionally, Invariant #7 says: "If `source_entity_type = work_order`, `source_entity_id` must reference a platform Production work order." The implication that other named values have defined semantics is misleading when the platform intentionally doesn't enforce FKs for them.

**Reasoning chain:** The author thought of examples while writing the spec and embedded them as illustrative values. The availability of "I know what TrashTech and RanchOrbit need" colored what should have been a fully open extension point.

**Severity:** Medium  
**Confidence:** 0.85  
**So What:** `source_entity_type` should be fully open (tenant-defined free string or enum) with `work_order` as the only platform-enforced recognized value. Remove `collection_batch` and `livestock_batch` from the field spec. Keep the FK-enforcement behavior for `work_order` only; document all other values as opaque by design.

---

### §F8 — Authority Bias on Quoting Ruling: The Opaque-Ref Gap
**Bias type:** Authority bias (user ruling accepted without stress-testing downstream effects)  
**Evidence:**  
SALES-ORDERS-MODULE-SPEC.md, §10: "`external_quote_ref` is an opaque string; no FK constraint."  
CRM-PIPELINE-MODULE-SPEC.md, §3, `opportunities` table: "`external_quote_ref` (opaque string for vertical's quoting system)"

The user ruled quoting stays in verticals. The author accepted this and introduced `external_quote_ref` as an opaque string in two modules without examining the downstream gap:

1. **Opportunity → Quote → Order navigation is broken platform-wide.** A user looking at an opportunity in any vertical cannot navigate to the linked quote through the platform API — `external_quote_ref` is an unresolvable string. They must switch to the vertical's local quoting UI. This UX seam is guaranteed, not contingent.

2. **Verticals without quoting modules have nowhere to go.** HuberPower and RanchOrbit — if they ever need to show a customer a price before booking an order — have no platform surface for that concept. The ruling forecloses without defining what should exist for verticals that lack Fireproof's sophisticated quoting module.

3. **The ruling may be right for wrong reasons.** The user likely ruled "quoting stays in Fireproof" because Fireproof's quoting is aerospace-specific (custom pricing, customer approval chains). But this doesn't preclude a minimal cross-vertical "quote reference registry" — a module that stores opaque references with party_id and status for navigation, without owning quote logic.

The author did not push back on whether the opaque-ref approach was sufficient, nor suggest a lightweight bridge design.

**Severity:** Medium  
**Confidence:** 0.7  
**So What:** The quoting ruling may still be correct. But the implementation of "opaque string" should be revisited before both modules lock in. Options: (a) explicitly document that quote navigation is a vertical responsibility with vertical-built UI bridges; or (b) define a lightweight `quote_references` table in Sales-Orders (stores `external_quote_ref`, `party_id`, `amount_cents`, `status`) that lets the platform resolve the reference without owning quote logic.

---

## 3. Risks Identified

| Risk | Severity | Likelihood |
|------|----------|------------|
| Manufacturing costing v0.1 produces incomplete financial data (no overhead); Fireproof ships cost reports with systematic understatement | Critical | High |
| Shop-floor-gates start-verification is aerospace-specific; HuberPower requires significant rework at first use | High | Medium |
| CRM-Pipeline built on platform then only used by Fireproof; cross-vertical bar not actually cleared | High | Medium |
| SO booking/reservation async gap produces "booked but stock-unconfirmed" orders with undefined behavior | High | Medium |
| AP supplier eligibility ships as a flag; Fireproof and HuberPower independently build local qualification processes, diverging | High | High |
| Signoff module prematurely consolidated; extracted later at high cost when Quality-Inspection also needs signoffs | Medium | Medium |
| OP `cert_ref` and `re_identification` tables become dead weight for TrashTech/RanchOrbit | Low | High |

---

## 4. Recommendations

| Priority | Recommendation | Effort | Expected Benefit |
|----------|---------------|--------|-----------------|
| P0 | Add `stock_confirmation_status` to `sales_order_lines`; define SO state semantics during async reservation window | Low | Prevents ambiguous order state at v0.1 |
| P0 | Rename "Manufacturing Costing" to "Direct Cost Accumulation" or define a v0.1 overhead allocation method | Low | Prevents systematically wrong financial reports |
| P1 | Replace `operation_start_verification`'s 3 hardcoded boolean columns with a configurable checklist table | Med | Unlocks HuberPower use; removes AS9100 hardcoding |
| P1 | Audit all canonical signoff roles and OP `source_entity_type` for vertical-specific vocabulary; replace with genuinely generic terms | Low | Removes Fireproof-shaped assumptions from platform API |
| P1 | Verify CRM-Pipeline cross-vertical need with HuberPower and RanchOrbit before beads enter pool | Low (research) | May eliminate a full module from scope |
| P2 | Add error taxonomy and background-job sections to all five new module specs | Low | Implementation bead authors get a testing and ops charter |
| P2 | Reduce shop-floor-gates label tables to only those with genuine customer-facing display variation need | Low | Reduces implementation surface without functional loss |
| P3 | Decide explicitly whether to split shop-floor-gates signoffs into a standalone signing service | Low (design) | Prevents premature-consolidation debt |
| P3 | Define `external_quote_ref` resolution strategy (vertical bridge, quote registry, or explicit non-platform) | Low | Closes UX navigation gap before implementation locks it in |

---

## 5. New Ideas and Extensions

**Incremental:**

- **`sales_order_lines.stock_confirmation_status`** — Add enum `(pending_confirmation / confirmed / rejected / waived)`. Small schema addition; large architectural clarity gain for the async reservation dance.

- **Setup Verification Checklist Table** — Replace the three boolean columns in `operation_start_verifications` with a `verification_checklist_items` table (`id`, `tenant_id`, `item_code`, `description`, `required`). The current three columns become default seed data for manufacturing tenants. HuberPower seeds a different list.

**Significant:**

- **Quote Reference Registry** — A `quote_references` table in Sales-Orders: (`id`, `tenant_id`, `external_quote_ref`, `party_id`, `amount_cents`, `currency`, `status` (open/accepted/expired), `external_system`). Verticals populate it on quote creation. The platform can then resolve `external_quote_ref` to a status and amount without owning quote logic. Costs one table and three endpoints; buys cross-vertical navigability without breaking the quoting-stays-in-vertical ruling.

- **Lightweight Vendor Qualification Process** — Rather than just a flag, define a `vendor_qualification_reviews` table with reviewer, date, document_refs, outcome, and next_review_date. Costs one table and ~4 endpoints; prevents Fireproof and HuberPower from building divergent local qualification processes. Still not a full QMS workflow, but enough that the platform surface is reusable.

**Radical:**

- **Re-evaluate Shop-Floor-Gates as four separate modules** — `traveler-holds`, `operation-handoffs`, `operation-verifications`, `shop-signoffs`. The signoffs module in particular could become platform-wide (not shop-floor-scoped), serving Quality-Inspection, Production, and future compliance modules. The consolidation was a presentation convenience; the platform standard supports fine-grained modules with separate DBs.

---

## 6. Assumptions Ledger

1. The four verticals are at roughly equal planning maturity. If HuberPower and RanchOrbit are effectively hypothetical at this stage, the cross-vertical test should explicitly be forward-looking, not present-tense.

2. The "canonical enum, tenant can rename" pattern assumes tenants need consistent vocabulary for operational concepts. This holds for financial modules (billing status, payment state). It is less obvious for internal shop-floor concepts where the platform vocabulary may not match any vertical's natural language.

3. The overlay pattern (Fireproof subscribes to platform events and maintains AS9100-specific overlay tables) works without coupling. This is architecturally sound but adds non-trivial operational complexity to Fireproof's codebase — it is not free from Fireproof's perspective.

4. "Sample data only, no ETL" eliminates data migration risk. Accepted. It does not eliminate the risk that Fireproof's current code shape introduces latent assumptions about what the new module must support.

5. The AR-MODULE-SPEC.md was chosen as the template because it is the most mature platform spec, not because it is the best template for operational modules. The choice may have been situational rather than deliberate.

---

## 7. Questions for Project Owner

1. **CRM-Pipeline for HuberPower and RanchOrbit:** Has either been consulted on whether they need a sales pipeline module? If not, should CRM-Pipeline be staged as "Fireproof-first, platform promotion pending second-vertical confirmation"?

2. **Manufacturing Costing v0.1 scope:** Is the intent for v0.1 to produce usable manufacturing costs for financial reporting, or to produce a direct-cost foundation? These are different deliverables with different acceptance criteria and different liability.

3. **AP Supplier Eligibility depth:** For Fireproof's AS9100 compliance, does supplier qualification require documented evidence (uploaded audit results, certificates) at platform level, or will Fireproof's overlay handle that? If Fireproof will build a qualification *process* locally on top of the platform flag, should the platform surface be designed to support that overlay from day one?

4. **Operation Start Verification hardcoded fields:** Were the three verification checkboxes (`drawing_verified`, `material_verified`, `instruction_verified`) chosen because they're the right universal platform abstraction, or because they reflect what Fireproof currently has? HuberPower cannot use these as-is.

5. **Shop-floor-gates consolidation:** Was the 4-way consolidation a deliberate architectural decision or a convenience for the spec-drafting session? Would four focused modules be preferable for independent deployability and evolution?

6. **Quoting navigation:** When a user looks at a Sales-Order and wants to find the originating quote, is the answer "go to the vertical's local quoting UI" by design — or is there an expectation that the platform provides that navigation?

---

## 8. Points of Uncertainty

- **CRM-Pipeline cross-vertical validity (0.75 confidence in §F5):** If HuberPower has an external sales team selling power management services to third parties, the need is real. I'm not certain.

- **Outside-Processing re-identification cross-vertical value:** I believe this is primarily an aerospace concept, but I have not modeled TrashTech's or RanchOrbit's processing workflows in detail. There may be a genuine analog in livestock processing that I'm underweighting.

- **Manufacturing costing overhead deferral:** It's possible the stakeholders fully understand v0.1 is a direct-cost foundation. The planning fallacy finding assumes the name "manufacturing costing" creates false expectations; if all parties understand the scope, the risk is lower.

- **Shop-floor-gates consolidation trade-offs:** A single module for all gate types has genuine benefits (one deployment, one database for joins across holds/handoffs/verifications on a work order). The right answer depends on usage patterns not predictable from the spec alone.

---

## 9. Agreements and Tensions with Other Perspectives

**Likely agreements with other modes:**

- Systems-Thinking (F7) will likely flag the async consistency gap in §F2 (SO booking → reservation dance) as a distributed systems seam. This analysis finds the same gap via incomplete state model.

- Domain-Driven Design modes will likely independently surface the shop-floor-gates consolidation concern in §F3: four different bounded contexts merged into one module.

- Risk/Complexity modes may flag manufacturing costing (§F4) from a financial integrity angle, aligning with this finding.

**Likely tensions with other modes:**

- Technical modes may propose SDK extension points to solve the gaps found here (configurable checklists, quote registries). This analysis recommends solving within existing SDK patterns (tables + events), not proposing SDK extensions — per the explicit anti-pattern in the context pack.

- Modes analyzing module granularity may push for fewer modules overall; this analysis's suggestion to potentially split shop-floor-gates into four modules goes in the opposite direction. The trade-off should be explicitly decided.

---

## 10. Confidence: 0.82

**Calibration note:** The highest-confidence findings (§F1 at 0.9, §F4 at 0.9, §F6 at 0.88) are supported by direct textual evidence where the bias pattern is visible in specific field names, column types, or missing spec sections — no inference required. Medium-confidence findings (§F3 at 0.7, §F5 at 0.75, §F8 at 0.7) require inference about author intent and vertical use-cases where I lack requirements documents for HuberPower, TrashTech, and RanchOrbit. §F2 (reservation consistency) is high-confidence as an architectural gap but medium-confidence that it's bias-driven rather than a deliberate deferral the author simply didn't document. Overall 0.82 reflects strong evidence on six findings balanced against genuine uncertainty about three verticals whose actual needs haven't been directly observed.
