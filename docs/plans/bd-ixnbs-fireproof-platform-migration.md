# Fireproof → Platform Migration Program

**Bead:** bd-ixnbs
**Status:** Scope settled 2026-04-16 — ready for per-module spec drafting
**Author:** LavenderWaterfall (Platform Orchestrator)
**Last updated:** 2026-04-16

## Context

User ruled 2026-04-16: 7D Platform owns modules that more than one vertical needs. Modules only one vertical needs stay in that vertical. Fireproof's data is sample/throwaway — no ETL, no strangler pattern, no stability gates required.

Verticals under consideration: Fireproof ERP (aerospace), HuberPower (in-house manufacturing for power generation), TrashTech (waste hauling + processing), RanchOrbit (ranching).

Cross-vertical test for each Fireproof module: **Would two or more verticals plausibly use this?** If yes, platform owns it. If no, Fireproof keeps it.

## Scope (after walkthrough with user)

### A. New platform modules to build

| Module | Purpose | Notes |
|--------|---------|-------|
| `sales-orders` | Pre-invoice order lifecycle (standard + blanket orders) | Every B2B vertical needs this. AR owns invoicing, not orders. |
| `crm-pipeline` | Leads, opportunities, stages + stage history, activities, quote-order linkage | Sits on top of existing Party Master. |
| `outside-processing` | Send-out to external vendors for specialized work; track, receive, reconcile against work order | Manufacturing verticals subcontract specialized work (heat treat, anodizing, plating, EDM). Fireproof + HuberPower. |
| `customer-complaints` | Inbound complaint intake, categorization, investigation, resolution, outcome | Every vertical gets complaints; formality differs but data shape is common. |
| `mrp` (or extension to BOM) | Time-phased material requirements: demand × BOM × on-hand → what to buy/make and when | Small computation engine. Platform Inventory has reorder policies, not MRP. Scope TBD: own module vs BOM extension. |

Manufacturing-ops items are in platform scope because Fireproof + HuberPower both run shop-floor production. Split settled 2026-04-16:

- **New platform module `shop-floor-gates`** — traveler holds, operation-handoff, operation-start-verification, signoff. Gating and state transitions at the operation level.
- **Production extension** — manufacturing-costing (cost accumulation on work orders, uses Production's existing time entries).
- **BOM extension** — kit-readiness endpoint (kit readiness = BOM explosion × Inventory availability; computation, not persistent state).

**Machine comm STAYS IN FIREPROOF.** CNC machine integration (DNC transfers, FANUC/Heidenhain/Siemens protocol specifics) is bespoke to the specific machines in the shop. HuberPower would need different code for its machines anyway — no cross-vertical reuse. Fireproof keeps its `machine_comm/` local.

### B. Extensions to existing platform modules

| Existing module | Extension |
|-----------------|-----------|
| Inventory | Deepen lot genealogy to match Fireproof's depth (applies to cattle parentage, food traceability, aerospace AS9100 equally). |
| Production | Add shop-floor items above (or split into new `shop-floor` module). |
| Workforce-Competence | Add training delivery: plans, assignments, completion records. Competence artifacts + assignments already covered. |
| AP | Add supplier eligibility/qualification flag + preferred-vendor list. |

### C. Fireproof-only — not migrating to platform

Formal ISO 9001 / AS9100 / aerospace-specific workflows. Stays in Fireproof as Fireproof's local code. No platform equivalent built.

- **Remnant tracking:** sub-lot tracking for bar stock/sheet metal offcuts. Fireproof-specific manufacturing concern.
- **Shop-floor data capture (SFDC):** kiosks, operator sessions, kiosk-driven labor capture. Bespoke to shop-specific hardware. Barcode resolution extracted to platform Inventory extension; the rest stays in Fireproof.
- **Quality management cluster (14 modules):** NCR, CAPA, concession, containment, MRB disposition, internal_audit, management_review, contract_review, revision_ack, risk_register, process_validation, product_safety, preservation, customer_property
- **Full calibration cluster (12 modules):** calibration_batch, calibration_fallout, calibration_lab, metrology, gauge_cert, gauge_gate, gauge_metadata, gauge_seals, gauge_sets, gauge_tracking, gauge_transfers, gauge_v2. Platform Maintenance has two basic calibration endpoints — that's enough for non-aerospace verticals. Fireproof keeps its depth.
- **SPC:** Statistical Process Control. Fireproof-specific formality.
- **AS9100 / AS9102 / ITAR specific:** fai_link (First Article Inspection), clause_mapping, export_control, counterfeit_prevention
- **Customer satisfaction surveys / NPS:** not a platform module; use external tools if needed.
- **Fireproof glue code:** ap_extension, ar_extension, customer_facade, gl_export, accounting_export, etc.

Fireproof's `procurement/` module doesn't migrate — it just gets retired and replaced with calls to platform AP (POs) + Shipping-Receiving (receipts). The supplier-eligibility gap is the only genuine new work, and that's a small AP extension.

### D. Already in platform — Fireproof wires up

Fireproof stops having its own copies and calls platform modules over HTTP using typed clients. Nothing to build on platform side.

- `ap` (purchase orders, vendors, bills, 3-way match, payment runs)
- `ar` (customers, invoices)
- `gl`, `consolidation`, `fixed-assets`, `treasury`
- `inventory` (items, lots, locations, UoM, valuation, cycle counts, reorder policies)
- `bom` (headers, revisions, lines, explosion, ECO)
- `production` (workcenters, work orders, routings, operations, time entries, workcenter downtime)
- `quality-inspection` (plans, inspections, disposition)
- `shipping-receiving` (shipments, receipts, inspection routing)
- `maintenance` (assets, basic calibration, downtime, meters, plans, maintenance work orders)
- `doc-mgmt` (replaces Fireproof's `document_control/`)
- `party` (companies, individuals, contacts, addresses)
- `numbering`, `workflow`, `notifications`, `timekeeping`, `integrations`, `customer-portal`, `pdf-editor`, `reporting`, `workforce-competence`

## Architecture standards each new module must follow

1. **AR-MODULE-SPEC.md shape** — Mission, Non-Goals, Domain Authority, Data Ownership with tables, OpenAPI surface, Events Produced/Consumed, State Machines, Tenant Isolation, Invariants.
2. **Platform SDK v1.0** — `ModuleBuilder.from_manifest().migrator().consumer().routes().run()`. No new SDK extension points unless a real conversion proves the need.
3. **Multi-tenancy: shared-DB, row-level isolation by `tenant_id`.** Verticals using database-per-tenant (via `DefaultTenantResolver`) remain vertical concerns; platform modules use the shared-DB pattern.
4. **Cross-module communication: contracts only.** OpenAPI at `contracts/<module>/`, event schemas at `contracts/events/<name>.v1.json`. No source imports between modules, no path deps, no cross-module DB writes.
5. **Event envelope standard:** `event_id`, `occurred_at`, `tenant_id`, `source_module`, `source_version`, `correlation_id`, `causation_id`, `payload`. Names use `<domain>.<entity>.<action>` dot notation.
6. **Tenant-configurable labels** (per Option B decision 2026-04-16): where modules have canonical enums (status, type, disposition, etc.), tenants can rename the display label but cannot add, remove, or reroute canonical codes. Events carry canonical codes only.

## Aerospace overlay pattern (Fireproof-side, not a platform feature)

When Fireproof needs to add AS9100-specific fields on top of a platform module (e.g. AS9100 clause refs on a platform customer-complaint record), Fireproof runs its own local service that subscribes to platform events and maintains its own overlay tables. Platform modules never know about vertical-specific fields. Fireproof's overlay service joins platform records with its local overlay data for its own UI.

This is a Fireproof-side architectural decision — platform doesn't ship extension points, doesn't define overlay schemas, doesn't gate on overlay presence. Platform modules are complete and correct without any overlay.

## Specs drafted 2026-04-16

All primary module specs complete in AR-MODULE-SPEC.md shape (Mission, Non-Goals, Domain Authority, Data Ownership, OpenAPI Surface, Events, State Machines, Invariants, Integration, Migration Notes):

### New platform modules (spec files)
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md` — sales orders + blanket orders + releases
- `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md` — send-out to external vendors, ship/return/review lifecycle
- `docs/architecture/CUSTOMER-COMPLAINTS-MODULE-SPEC.md` — intake, investigation, resolution, closure
- `docs/architecture/CRM-PIPELINE-MODULE-SPEC.md` — leads, opportunities, tenant-defined pipeline stages, activities (references Party for contacts; does not duplicate contact master)
- `docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md` — traveler holds, operation handoffs, operation start verification, signoffs

### Extensions to existing platform modules
- `docs/architecture/PLATFORM-EXTENSIONS-SPEC.md` — consolidated spec for six extensions:
  - BOM: MRP explosion + kit readiness
  - Inventory: barcode resolution service
  - Production: manufacturing costing
  - Workforce-Competence: training delivery (plans, assignments, completions)
  - AP: supplier eligibility + qualification

### Retired drafts (not platform scope)
- `docs/architecture/NONCONFORMANCE-MODULE-SPEC.md` — banner-flagged, kept for historical reference
- `docs/architecture/CORRECTIVE-ACTION-MODULE-SPEC.md` — banner-flagged
- `docs/architecture/SHOP-FLOOR-DATA-MODULE-SPEC.md` — banner-flagged. Only the barcode resolution portion moved to the Inventory extension; kiosks + operator sessions + kiosk-driven labor capture stay in Fireproof as shop-specific hardware + bespoke workflow (same reasoning as machine-comm). Platform's Production already owns Time Entries for cross-vertical labor tracking.

## Next steps

1. **Circulate specs to agents for review.** Send the five module specs + extensions spec to CopperRiver, PurpleCliff, MaroonHarbor, SageDesert, DarkCrane for sign-off on the architecture. Same information to all; explicit APPROVED/BLOCKED from each.
2. **Mail RoseElk with concrete module-migration calls.** Per her pause mail — the 12 paused Fireproof frontend beads depend on knowing which platform modules will exist. The five specs + extensions give her the answer.
3. **Adversarial review by Grok.** Send the combined spec set to Grok for a stress-test pass. Fold critiques into revisions before any implementation bead hits the pool.
4. **Decompose into implementation beads.** Each spec becomes a set of implementation beads: (1) module scaffolding via ModuleBuilder, (2) schema migrations, (3) domain + repo layer, (4) routes, (5) events + outbox, (6) typed SDK client stub, (7) contract tests. Extensions follow the same shape but layered onto existing modules. Decomposition via Codex agent.
5. **Publish beads to pool.** After user sign-off on both architecture specs and bead decomposition. Agent swarm (CopperRiver, PurpleCliff, MaroonHarbor, SageDesert, DarkCrane) executes.

## Open issues retained in specs (deferred to implementation)

- **Sales-orders:** tax calculation timing (on book vs. on invoice), partial-ship state semantics, backorder handling
- **Outside-processing:** PO required vs. optional at issue time, service_type taxonomy scope
- **Customer-complaints:** SLA configuration per severity, attachment storage (defer to doc-mgmt), duplicate detection
- **CRM-pipeline:** confirmation of drop-CrmContact / use-Party-contacts approach, team-based ownership, opportunity split semantics
- **Shop-floor-gates:** verification two-step vs. one-step flexibility, multi-operation handoffs, signoff cross-module reuse
- **Extensions:** time-phased MRP (future), overhead allocation rules for manufacturing costing (future), kit-readiness policy knobs (future)

## Retired from scope (superseded by user decisions 2026-04-16)

The following spec drafts in `docs/architecture/` were written before the user ruled QMS stays in Fireproof. They are retained as historical reference only; not being built on platform.

- `NONCONFORMANCE-MODULE-SPEC.md` — stays in Fireproof
- `CORRECTIVE-ACTION-MODULE-SPEC.md` — stays in Fireproof

Both specs get a "NOT PLATFORM SCOPE" banner at the top pointing back to this plan.
