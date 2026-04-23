# Modes of Reasoning — Context Pack

> **READ THIS ENTIRE FILE BEFORE ANALYZING.** It defines scope, constraints, and what NOT to discover.

## What You Are Doing

You are one of 10 agents analyzing the Fireproof→7D Platform migration spec set through a specific reasoning-mode lens. Your mode assignment comes in a separate prompt. Your job is to surface blind spots, seam decisions, coupling issues, and leaky abstractions **before implementation beads are decomposed**.

## Project

**7D Solutions Platform** — multi-vertical ERP backend, Rust modules + SDK.

**Verticals:**
- Fireproof (aerospace manufacturing, first signed customer, pre-launch)
- HuberPower (power-generation, in-house manufacturing)
- TrashTech (waste hauling + processing)
- RanchOrbit (ranching, livestock management)

**Current state:** 27 existing platform modules (ap, ar, bom, inventory, production, maintenance, workforce-competence, etc.). This session drafted 5 new module specs + 7 extensions for migrating generic code from Fireproof into 7D Platform.

## Specs Under Your Review

Read all of these:
1. `docs/plans/bd-ixnbs-fireproof-platform-migration.md` — the plan doc
2. `docs/architecture/SALES-ORDERS-MODULE-SPEC.md`
3. `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md`
4. `docs/architecture/CUSTOMER-COMPLAINTS-MODULE-SPEC.md`
5. `docs/architecture/CRM-PIPELINE-MODULE-SPEC.md`
6. `docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md`
7. `docs/architecture/PLATFORM-EXTENSIONS-SPEC.md`

Read these for architectural context:
- `docs/architecture/AR-MODULE-SPEC.md` — the template the new specs follow
- `docs/architecture/LAYERING-RULES.md`
- `docs/architecture/BOUNDARY-ENFORCEMENT.md`
- `docs/architecture/CONTRACT-STANDARD.md`
- `contracts/events/README.md`
- `platform/platform-sdk/README.md`

**DO NOT analyze these (retired from platform scope):**
- `NONCONFORMANCE-MODULE-SPEC.md`
- `CORRECTIVE-ACTION-MODULE-SPEC.md`
- `SHOP-FLOOR-DATA-MODULE-SPEC.md`

## Core Substrate — DO NOT RECOMMEND ABSTRACTING THESE

These define what the platform IS. Filter out any recommendation to decouple, abstract, or replace them:

1. **Platform SDK v1.0** — frozen, additive-only. ModuleBuilder.from_manifest().migrator().consumer().routes().run(). No overlay extension points.
2. **Module-per-service pattern** — one Rust module = one Cargo crate = one service = one Postgres DB = one port.
3. **Multi-tenancy models:**
   - Default: shared-DB + `tenant_id` row isolation (all platform modules use this)
   - Verticals: database-per-tenant via `DefaultTenantResolver` (Fireproof uses this)
4. **Contract-driven boundaries** — CI-enforced: no source imports across modules, no path deps, no cross-module DB writes, no cross-module foreign keys. All integration via OpenAPI + event schemas.
5. **NATS event bus with standardized envelope:** `event_id`, `occurred_at`, `tenant_id`, `source_module`, `source_version`, `correlation_id`, `causation_id`, `payload`. Event names: `<domain>.<entity>.<action>`.

## User Rulings (already decided — don't re-litigate)

**The cross-vertical test:** Platform owns modules that **more than one vertical** would use. Single-vertical needs stay in that vertical.

Already ruled 2026-04-16:
- **Stays in Fireproof** (don't re-propose for platform): NCR, CAPA, concession, containment, MRB, internal audit, management review, revision ack, contract review, risk register, process validation, product safety, preservation, customer property, SPC, calibration cluster (12 Fireproof modules, beyond platform Maintenance's basic calibration), CNC machine comm, SFDC kiosk UI + sessions + kiosk-driven labor, CSAT/NPS surveys, AS9100/AS9102/ITAR-specific logic, Fireproof glue code (ap_extension, ar_extension, etc.), quoting/RFQ.
- **Platform owns** (these are what you're reviewing): sales-orders, outside-processing, customer-complaints, crm-pipeline, shop-floor-gates. Plus 7 extensions: BOM (MRP + kit readiness), Inventory (barcode resolution + remnants), Production (manufacturing costing), Workforce-Competence (training delivery), AP (supplier eligibility).

## Fireproof Data Reality

Fireproof has a signed customer but **no production data yet** — everything is sample. Migrations can freely delete and rebuild. Do not raise "data migration risk" or "strangler pattern" concerns.

## Known Open Questions (already documented in specs — don't restate as findings)

Each spec has its own "Open questions" section. Examples:
- Tax calc timing (on book vs on invoice)
- PO optionality at OP issue
- SLA config per complaint severity
- Verification 2-step vs 1-step
- Multi-op handoffs
- Time-phased MRP (future)
- Overhead allocation rules
- Team-based ownership
- Attachments via doc-mgmt
- Source-entity FK enforcement

If your finding restates one of these, flag it as "confirmed known" rather than presenting as a discovery.

## Project Values (from CLAUDE.md)

- No tech debt, do it right
- Real services in tests, no mocks
- Separation of concerns over line count
- Platform ships plug-and-play modules + SDK; verticals are plug-and-play consumers
- No frontend work on this repo

## Output Contract

Write your analysis to `MODE_OUTPUT_<MODE_ID>.md` in the project root (e.g., `MODE_OUTPUT_F7.md` for Systems-Thinking).

Required sections:
1. **Thesis** — one-paragraph summary
2. **Top Findings** — 5-8 findings, each with:
   - §F[N] ID for cross-referencing
   - Evidence: specific file, section, line
   - Reasoning chain: how your mode specifically reveals this
   - Severity: critical / high / medium / low (calibrated to pre-launch architecture context, not theoretical worst case)
   - Confidence: 0.0-1.0
   - So What?: concrete next-day action
3. **Risks Identified** — severity + likelihood
4. **Recommendations** — priority (P0-P4), effort (low/med/high), expected benefit
5. **New Ideas and Extensions** — incremental/significant/radical
6. **Assumptions Ledger** — unstated assumptions this analysis depends on
7. **Questions for Project Owner**
8. **Points of Uncertainty**
9. **Agreements and Tensions with Other Perspectives**
10. **Confidence: 0.0-1.0** with calibration note

## Anti-Patterns (don't do these)

- **Don't recommend abstracting the core substrate** (SDK, module-per-service, tenant_id, contract boundaries)
- **Don't re-discover user rulings** (QMS stays in Fireproof, quoting out of scope, etc.)
- **Don't rate severity against hypothetical worst-case** — calibrate to pre-launch architecture stage
- **Don't restate documented open questions** as discoveries
- **Don't recommend ISO-like QMS features** for platform — user explicitly ruled them out
- **Don't recommend runtime extension points** in the frozen SDK without a concrete proven conversion need

## Your Goal

Find what I (the spec author) missed. Blind spots. Coupling I didn't see. Assumptions that don't hold. Seams between modules that will cause pain. Integration points that will leak. Anything that will make implementation beads difficult to write cleanly.

Think hard. Apply your specific framework rigorously. Don't summarize what the specs say — surface what they don't say.
