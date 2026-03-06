# Manufacturing Modules Scope Review

**Date:** 2026-03-04
**Initiated by:** WhiteValley (Fireproof ERP integration team)
**Coordinated by:** BrightHill (Platform Orchestrator)
**Bead:** bd-2wahd

## Purpose

WhiteValley proposes 4 new platform modules for generic manufacturing. This is the biggest scope expansion since platform inception. We need independent reviews from multiple agents before creating any beads.

**Save your review to:** `docs/manufacturing-review-{your-name}.md`

---

## The Proposal

WhiteValley argues that manufacturing features they planned to build inside Fireproof ERP are actually generic domain modules that belong in the platform — following the same pattern as Inventory, Maintenance, and Workflow.

### Proposed New Platform Modules

#### 1. BOM (Bill of Materials)
- Multi-level BOM structure (parent/child with quantity-per)
- BOM revisions and effectivity dates
- Engineering change orders (ECO lifecycle)
- Where-used queries
- **Depends on:** Inventory (items), Numbering (part numbers)

#### 2. Production
- Production work orders (create, release, track, close)
- Routing / operations (sequence of steps to make a part)
- Shop floor tracking (operation start/complete/scrap)
- Labor collection (who worked on what, how long)
- **Depends on:** BOM, Inventory (issue/receipt), Workflow (approvals)

#### 3. Quality
- Inspection plans (what to check, acceptance criteria, sampling rules)
- First article inspection (FAI — prove the process before production run)
- Nonconformance reports (NCR lifecycle: identify → disposition → close)
- Corrective/preventive action (CAPA lifecycle: root cause → action → verify)
- Special process controls (welding, heat treat, plating — track certifications and parameters)
- **Depends on:** Inventory (lot/serial), Workforce-Competence (inspector qualifications), Maintenance (equipment calibration)

#### 4. MRP / Planning
- Material requirements planning (explode BOM against demand, net against inventory)
- Production scheduling (sequence work orders against capacity)
- Reorder point integration (partially exists in Inventory)
- **Depends on:** BOM, Inventory (on-hand/reservations), Production (capacity)

### What Stays App-Specific (Fireproof Only)
- AS9100 compliance rule sets
- ITAR/export control tracking
- Flowdown clause management
- NADCAP accreditation tracking
- AS9102 first article report format

---

## Existing Platform Context

### Modules That Would Interact

| Module | Current State | Manufacturing Relevance |
|--------|--------------|------------------------|
| **Inventory** | Mature: items, lot/serial, reservations, FIFO costing, status buckets (quarantine/damaged), item revisions with effectivity dates, UoM conversions, item classifications | Items = BOM components. Reservations support reference_type for production orders. Status quarantine = quality hold. **Gaps:** No make/buy flag, no production receipt path, no BOM hierarchy |
| **Maintenance** | Assets, work orders (8-state machine), parts, labor, calibration events, downtime events | Calibration = quality records. Workcenter downtime affects production capacity. **Gaps:** workcenter_id exists on downtime but no workcenter master table. WO parts are ad-hoc (text description), not inventory-integrated |
| **Workflow** | Entity-agnostic approval engine: sequential, parallel (N-of-M), conditional routing. Holds, escalation timers, delegation rules | Drives ECO, NCR, CAPA lifecycles. Holds primitive for quality/engineering holds. **Strength:** Fully reusable, entity_type is free-form |
| **Workforce-Competence** | Competence artifacts (cert/training/qual), operator assignments with expiry, acceptance authorities with capability_scope | Inspector qualifications, special process certifications. Authorization check answers "was this person qualified when they signed off?" |
| **Numbering** | Atomic allocation, gap-free mode, configurable patterns per entity | Part numbers, ECO numbers, NCR numbers, CAPA numbers, production order numbers. Gap-free for regulated sequences |
| **Shipping-Receiving** | Inbound/outbound lifecycle, inspection_routings (direct_to_stock or send_to_inspection), RMA | inspection_routings is the natural hook for receiving inspection. RMA disposition mirrors NCR flow |

### Platform Architecture Patterns
- **Guard → Mutation → Outbox** atomicity on all writes
- **EventEnvelope** constitutional metadata on all events
- **NATS JetStream** for inter-module communication
- **Tenant-scoped** everything (multi-tenant by default)
- **500 LOC file limit**, 4 cargo build slots for parallel compilation
- **No mocks in tests** — integrated tests against real Postgres + NATS

---

## Review Questions

Address ALL of these in your review:

### A. Module Boundaries
Is the 4-module split correct? Should any be combined or further split? Specifically: is Quality too big (inspection plans + NCR + CAPA + special processes are 4 distinct lifecycles)?

### B. Build Sequencing
The natural order is BOM → Production → Quality → MRP. MRP depends on everything. What's the minimum viable manufacturing stack? What can ship incrementally to unblock Fireproof?

### C. Platform vs App-Specific
WhiteValley says compliance stays app-specific. But are "inspection plans with acceptance criteria and sampling rules" and "special process controls" truly generic manufacturing, or aerospace wearing a generic hat? Where's the real boundary?

### D. Existing Module Retrofit
What changes are needed to existing modules? Inventory needs make/buy flag + production receipt. Maintenance needs workcenter master. Are we underestimating the retrofit work?

### E. Scope and Risk
This roughly doubles the platform's domain surface area. With 5 implementation agents, is this a 3-month or 12-month effort? What would you defer?

### F. Dependencies and Integration
Draw out the dependency graph. Are there circular dependencies? Which integrations are "must have day one" vs "wire later"?

### G. Alternative Approaches
Could any of this be achieved by extending existing modules rather than creating new ones? For example, could BOM be an extension of Inventory's item hierarchy?

---

## Review Format

Save your findings as `docs/manufacturing-review-{your-name}.md` with:
1. **Executive summary** (2-3 sentences: your overall verdict)
2. **Answers to A-G** above
3. **Top 3 risks** you see
4. **Your recommended approach** (what to build first, what to defer, what to reject)

Be direct. Disagreement is valuable. We need to find the holes before we commit to building.
