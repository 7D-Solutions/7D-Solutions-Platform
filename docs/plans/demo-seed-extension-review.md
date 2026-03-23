# Demo-Seed Extension Plan — Review Request

> **For:** Claude Desktop independent review
> **Date:** 2026-03-22
> **Status:** Draft beads created, reviewed by internal adversarial pass + ChatGPT. Seeking third independent review before publishing to pool.

## Context

This plan extends the existing deterministic test data seeder (`tools/demo-seed/`) to cover manufacturing and supply chain modules. A downstream ERP product (Fireproof, aerospace/defense) needs a fully populated Platform test tenant to run exhaustive AS9100 integration tests.

**Architecture:**
- Rust/Axum microservices, one per module, each with its own Postgres database
- Multi-tenant via JWT claims (tenant_id extracted from token)
- NATS JetStream event bus with transactional outbox pattern
- demo-seed is an external CLI tool that makes HTTP requests to service APIs (reqwest, no direct DB access)
- Deterministic via ChaCha8 RNG seeded from a u64 — same seed = identical output
- Existing demo-seed only seeds AR (accounts receivable) customers and invoices
- DigestTracker computes SHA256 over sorted (type, correlation_id, value) entries for determinism verification

## DAG (8 beads, 2 roots, 1 capstone)

```
bd-3ng0x (GL account creation API)     bd-2ob3y (CLI framework + numbering)
    |                                   |         |         |
    +---> bd-3vpcd (GL seeding) <-------+         |         |
              |                                    |         |
              +---> bd-g0f3n (Inventory) <---------+         |
                      |         |                            |
                      |         +---> bd-2g3br (Production)--+
                      |                                      |
                      +---> bd-8g1ka (BOM) ------------------+
                                                             |
                   bd-20bq8 (Party) <------------------------+
                      |                                      |
                      +--------------------------------------+
                                                             v
                                              bd-56y0d (E2E test)
```

Two roots can run in parallel. BOM and Production can run in parallel after Inventory.

## Bead Details

### bd-3ng0x — GL: Add account creation API endpoint (ROOT A)

- Adds POST /api/gl/accounts to GL module (currently no account creation API exists — only SQL INSERT in tests/benchmarks)
- Request: `{ code: String, name: String, account_type: String (Asset|Liability|Equity|Revenue|Expense), normal_balance: String (Debit|Credit) }`
- 409 Conflict on duplicate (tenant_id, code) — ON CONFLICT DO NOTHING
- Existing Account struct and table already exist; this only adds the CREATE path
- Integration tests: create returns 201, duplicate returns 409, missing field returns 422, cross-tenant isolation
- **Files:** modules/gl/src/http/accounts.rs (create), modules/gl/src/repos/account_repo.rs (modify), modules/gl/src/main.rs (modify), modules/gl/tests/ (create)

### bd-2ob3y — demo-seed: CLI framework + numbering policies (ROOT B)

- Extends CLI with `--modules` flag (numbering, gl, party, inventory, bom, production, ar, all; default: all)
- Per-service URL flags with env var fallbacks: `--numbering-url` (8120), `--gl-url` (8090), `--party-url` (8098), `--inventory-url` (8092), `--bom-url` (8107), `--production-url` (8108); existing `--ar-url` (8086) unchanged
- Adds src/numbering.rs: PUT /policies/{entity} (NOTE: no /api/numbering/ prefix — routes at root level)
  - Request: `{ pattern: String (must contain {number} token), prefix: String, padding: i32 (0-20) }`
  - PUT is natively idempotent (upsert via ON CONFLICT UPDATE)
  - 8 entities: purchase-order (PO-{YYYY}-{number}), sales-order, work-order, eco, shipment, invoice, bom, receiving-report
- Module dispatch in main.rs: executes in dependency order, each module returns created IDs in a struct for downstream use
- Critical invariant: existing AR seeding RNG call sequence must not change (would break existing digests)
- **Files:** tools/demo-seed/src/main.rs, numbering.rs (create), digest.rs, Cargo.toml

### bd-3vpcd — GL chart of accounts seeding (depends: bd-3ng0x, bd-2ob3y)

- 20 accounts via POST /api/gl/accounts (port 8090):
  - Assets: 1000 Cash, 1100 AR, 1200 Raw Materials Inventory, 1210 WIP, 1220 Finished Goods, 1300 Fixed Assets
  - Liabilities: 2000 AP, 2100 Accrued Expenses
  - Equity: 3000 Retained Earnings
  - Revenue: 4000 Product Sales, 4100 Service Revenue
  - COGS: 5000 Direct Materials, 5010 Direct Labor, 5020 Manufacturing Overhead, 5030 Scrap/Rework
  - Expenses: 5100 PPV, 5120 Inventory Adjustments, 6000 SGA, 6100 R&D
- 2 FX rates via POST /api/gl/fx-rates: USD/EUR (0.92), USD/GBP (0.79)
- Idempotency: 409 treated as success
- Critical: codes 1200, 1210, 1220, 5000, 5100, 5120 must exist before inventory items can be created
- **Files:** tools/demo-seed/src/gl.rs (create), main.rs, digest.rs

### bd-20bq8 — Party seeding (depends: bd-2ob3y)

- 5 customers via POST /api/party/companies (port 8098): Boeing Defense, Lockheed Martin, Northrop Grumman, Raytheon, General Dynamics
- 5 suppliers: Bodycote, Alcoa, Carpenter Technology, Precision Castparts, Hexcel
- Party type is "company" ONLY — customer/supplier via tags in metadata field (no customer/supplier enum)
- Required fields: display_name, legal_name; optional: tax_id, registration_number, etc.
- Contacts via POST /api/party/parties/{id}/contacts (required: first_name, last_name)
- Addresses via POST /api/party/parties/{party_id}/addresses (required: line1, city; type: registered)
- Idempotency: via `Idempotency-Key` HTTP HEADER (not body-level), value: `{tenant}-company-{seed}-{idx}`
- Returns party IDs in a struct for downstream reference
- **Files:** tools/demo-seed/src/party.rs (create), main.rs, digest.rs

### bd-g0f3n — Inventory seeding (depends: bd-3vpcd, bd-2ob3y)

- 5 UoMs via POST /api/inventory/uoms (port 8092): EA, KG, LB, M, IN. Required: code, name. 409 on duplicate.
- 7 locations via POST /api/inventory/locations: RECV-DOCK, RAW-WH, WIP-FLOOR, FG-WH, SHIP-DOCK, QA-HOLD, MRB
  - Requires warehouse_id: Uuid — no warehouse API exists, generate deterministic UUID v5 from `(NAMESPACE_DNS, "{tenant}-warehouse-{seed}")`
- 13 items via POST /api/inventory/items:
  - Required fields: sku (unique/tenant), name, inventory_account_ref, cogs_account_ref, variance_account_ref, tracking_mode (none|lot|serial)
  - 5 raw materials (buy, lot): TI64-BAR-001, INC718-FRG-001, AL7075-SHT-001, 4130-TUB-001, HXL8552-PPG-001
  - 5 manufactured parts (make, lot): TBB-ASSY-001, EMB-ASSY-001, SRA-ASSY-001, FLC-ASSY-001, LGA-ASSY-001
  - 3 fasteners (buy, none): AN3-BOLT, MS21042-NUT, NAS1149-WASH
  - GL refs: buy items use 1200 (inventory), make items use 1220 (FG); all use 5000 (COGS), 5100 (variance)
- Idempotency: 409 on duplicate SKU; agent must GET by SKU to retrieve existing UUID
- Returns item UUIDs, location UUIDs, warehouse UUID for downstream
- **Files:** tools/demo-seed/src/inventory.rs (create), main.rs, digest.rs

### bd-8g1ka — BOM seeding (depends: bd-g0f3n)

- BOM per make item via POST /api/bom (port 8107): `{ part_id: Uuid }`
- Revision A via POST /api/bom/{bom_id}/revisions: `{ revision_label: "A" }`
- Component lines via POST /api/bom/revisions/{revision_id}/lines: `{ component_item_id: Uuid, quantity: f64, scrap_factor: Option<f64> }`
- BOM structures:
  - Turbine Blade Blank: Ti-6Al-4V (1, 5% scrap)
  - Engine Mount Bracket: AL-7075-T6 (2), AN3 Bolt (4), MS21042 Nut (4), NAS1149 Washer (8)
  - Structural Rib: 4130 Steel (3), Inconel 718 (1, 3% scrap)
  - Fuel Line Connector: 4130 Steel (1), AN3 Bolt (2)
  - Landing Gear Housing: Inconel 718 (2), Ti-6Al-4V (1)
- Effectivity: FIXED date 2026-01-01T00:00:00Z (not current date — determinism requirement)
- NO idempotency on BOM endpoints. Check-before-create via GET:
  - Natural key: (tenant_id, part_id) for BOMs, (bom_id, revision_label) for revisions
- **Files:** tools/demo-seed/src/bom.rs (create), main.rs, digest.rs

### bd-2g3br — Production seeding (depends: bd-g0f3n)

- 6 work centers via POST /api/production/workcenters (port 8108):
  - CNC-MILL-01 ($150/hr), CNC-LATHE-01 ($120/hr), HEAT-TREAT ($80/hr, capacity 4), GRIND-01 ($100/hr), ASSEMBLY-01 ($50/hr, capacity 2), NDT-01 ($200/hr)
  - Required: code (unique/tenant), name; optional: description, capacity, cost_rate_minor
- 5 routings via POST /api/production/routings: `{ name, item_id: Uuid, revision: "1" }`
  - Steps via POST /api/production/routings/{id}/steps: `{ sequence_number: i32, workcenter_id: Uuid, operation_name, setup_time_minutes, run_time_minutes }`
  - Turbine Blade: rough mill (30/45), finish mill (15/60), heat treat (10/480), grind (15/30), NDT (5/20)
  - Engine Mount: CNC mill (20/35), heat treat (10/360), NDT (5/15)
  - Structural Rib: CNC mill (25/50), lathe (15/30), heat treat (10/240), assembly (10/45)
  - Fuel Line: lathe (15/20), grind (10/15), NDT (5/10)
  - Landing Gear: CNC mill (30/90), heat treat (10/480), grind (15/45), NDT (5/30), assembly (15/60)
- Idempotency: 409 on duplicate workcenter code, (item_id, revision), step sequence — all treated as success
- **Files:** tools/demo-seed/src/production.rs (create), main.rs, digest.rs

### bd-56y0d — E2E test + convenience script (depends: bd-8g1ka, bd-2g3br, bd-3vpcd, bd-20bq8)

- E2E test (e2e-tests/tests/demo_seed_manufacturing_e2e.rs) with 6 test cases:
  1. Full pipeline: assert exact counts (20 accounts, 8 policies, 10 parties, 5 UoMs, 7 locations, 13 items, 5 BOMs, 6 work centers, 5 routings)
  2. Deterministic rerun: same seed = identical digest
  3. Idempotent rerun: second run creates zero new resources
  4. Different seeds: different digests
  5. Module selection: --modules numbering,party only runs those
  6. Backwards compatibility: --modules ar with existing flags works
- scripts/seed-manufacturing.sh: convenience wrapper with preflight health checks
- README update: all modules, flags, ports, data counts
- **Files:** e2e-tests/tests/demo_seed_manufacturing_e2e.rs (create), e2e-tests/Cargo.toml (modify), tools/demo-seed/README.md (modify), scripts/seed-manufacturing.sh (create)

## Issues Already Found and Fixed

| # | Issue | Fix |
|---|-------|-----|
| 1 | GL account creation API doesn't exist | New prerequisite bead bd-3ng0x |
| 2 | Inventory depends on GL (account refs) but no dependency declared | Added bd-g0f3n -> bd-3vpcd |
| 3 | BOM+Production was one bead (two concerns) | Split into bd-8g1ka + bd-2g3br |
| 4 | Shipping referenced in CLI but no shipping bead | Removed shipping references |
| 5 | Party idempotency wrong (said correlation IDs) | Fixed to Idempotency-Key header |
| 6 | Party has no customer/supplier type | Fixed: company only, tags for role |
| 7 | Inventory idempotency wrong (said correlation IDs) | Fixed to 409 + GET fallback |
| 8 | Inventory tracking_mode required but not mentioned | Added lot/serial/none per item |
| 9 | Warehouse has no creation API | Documented UUID v5 approach |
| 10 | BOM has no idempotency | Documented check-before-create with natural keys |
| 11 | Production 409 handling undocumented | Documented per endpoint |
| 12 | Service ports missing | All ports documented |
| 13 | BOM effectivity used current date (non-deterministic) | Fixed to constant 2026-01-01 |
| 14 | GL missing inventory adjustments account | Added 5120 (now 20 accounts) |
| 15 | Fasteners on turbine blade unrealistic | Moved to engine mount + fuel line |
| 16 | E2E assertions vague | Added exact count assertions + idempotent rerun test |

## Review Task

Analyze this plan with extreme skepticism. For each bead, evaluate:

1. **Single-concern scope**: Is any bead doing two unrelated things? Would splitting improve parallelism or reduce risk?
2. **Executable without clarification**: Could an agent pick up any bead and implement it from the text alone, with zero questions? Where would they get stuck?
3. **Dependency correctness**: Are there missing or unnecessary dependencies? Could an agent pick up a bead prematurely and waste effort? Are there circular dependencies?
4. **Acceptance criteria**: Are they measurable and unambiguous? Can each one be verified with a concrete command?
5. **Verification**: Are tests real integration tests against real services? Any mocks or stubs hiding?
6. **Idempotency correctness**: For each service, is the documented idempotency mechanism actually correct? Will re-runs actually work as described?
7. **API accuracy**: Are the documented request bodies, response codes, and endpoints correct? Cross-reference with the actual source code in modules/*/src/.
8. **Determinism**: Is there any path where the same seed could produce different output? Any use of current time, random UUIDs, or non-deterministic ordering?
9. **Cross-module data flow**: Do the UUID/ID handoffs between modules work? Could any module receive an ID that doesn't exist?
10. **Manufacturing realism**: Are the parts, BOMs, routings, and work centers realistic for an aerospace job shop?

Also answer:
- What is the single biggest risk in this plan?
- What is the most likely bead to fail during implementation?
- If you had to cut one bead to ship faster, which would it be and what would break?

Do not hold back. The goal is to find problems now, not during implementation.

## Source Code References

To verify API claims, check these files:
- GL routes: `modules/gl/src/main.rs`
- GL account repo: `modules/gl/src/repos/account_repo.rs`
- Numbering routes: `modules/numbering/src/main.rs`
- Party routes: `modules/party/src/http/mod.rs`
- Inventory routes: `modules/inventory/src/main.rs`
- BOM routes: `modules/bom/src/main.rs`
- Production routes: `modules/production/src/main.rs`
- Existing demo-seed: `tools/demo-seed/src/`
- Existing E2E tests: `e2e-tests/tests/`
