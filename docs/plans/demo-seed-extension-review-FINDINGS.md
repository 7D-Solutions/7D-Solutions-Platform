# Demo-Seed Extension Plan — Independent Review Findings

> **Reviewer:** Claude (independent third review)
> **Date:** 2026-03-22
> **Method:** Every API claim cross-referenced against actual source code in `modules/*/src/`

---

## Executive Summary

The plan is solid overall. The previous adversarial passes caught real issues and the fixes are sound. However, this review found **5 issues that will cause implementation failures** if not addressed, plus several minor inaccuracies. The single biggest risk is the Party idempotency claim — the mechanism described in the plan does not exist in the running code.

### Critical Findings (will block implementation)

| # | Bead | Issue | Severity |
|---|------|-------|----------|
| C1 | bd-20bq8 | Party `Idempotency-Key` header is NOT wired to handlers — infrastructure only (table exists, deferred to v2 per spec) | **BLOCKER** |
| C2 | bd-8g1ka | BOM has no GET-by-natural-key endpoints — check-before-create strategy is unimplementable as described | **BLOCKER** |
| C3 | bd-3vpcd | GL FX rate creation requires `idempotency_key`, `effective_at`, and `source` fields — plan omits all three | **HIGH** |
| C4 | bd-2g3br | Production routing `item_id` is `Option<Uuid>` not `Uuid` — also has additional optional fields | **MEDIUM** |
| C5 | bd-2g3br | Production routing steps `setup_time_minutes` and `run_time_minutes` are `Option<i32>` not required | **LOW** |

---

## Per-Bead Verification

### bd-3ng0x — GL: Add account creation API endpoint

**API accuracy: VERIFIED CORRECT**

- Confirmed: No `POST /api/gl/accounts` exists. All 18 POST routes in `modules/gl/src/main.rs` are for periods, FX rates, revrec, accruals, and exports. No account creation path.
- Confirmed: `Account` struct exists in `modules/gl/src/repos/account_repo.rs` with fields: `id` (Uuid), `tenant_id`, `code`, `name`, `account_type` (AccountType enum), `normal_balance` (NormalBalance enum), `is_active`, `created_at`.
- Confirmed: AccountType enum values: `asset`, `liability`, `equity`, `revenue`, `expense`. NormalBalance enum: `debit`, `credit`.
- Confirmed: Migration `20260213000001_create_accounts_table.sql` has `UNIQUE (tenant_id, code)` constraint, which supports the 409/ON CONFLICT DO NOTHING strategy.
- Confirmed: Accounts are only created via direct SQL INSERT in test helpers (`modules/gl/tests/common/mod.rs`).
- Confirmed: Port 8090 (default in `modules/gl/src/config.rs`).
- Module version: 0.1.0 (unproven) — no version bump required.

**Verdict:** This bead is well-specified and implementable without clarification. The request body fields match the existing struct. The 409 strategy aligns with the database constraint.

---

### bd-2ob3y — demo-seed: CLI framework + numbering policies

**API accuracy: VERIFIED with one clarification**

- Confirmed: PUT `/policies/{entity}` at root level (no `/api/numbering/` prefix). Route in `modules/numbering/src/main.rs` line 107-110.
- Confirmed: Request body `UpsertPolicyRequest`: `pattern` (String), `prefix` (String, serde default), `padding` (i32, serde default).
- Confirmed: Validation — `pattern` must contain `{number}` token, 1-255 chars; `prefix` max 50 chars; `padding` 0-20 inclusive.
- Confirmed: Idempotent PUT via `ON CONFLICT (tenant_id, entity) DO UPDATE` in `modules/numbering/src/policy.rs`.
- Confirmed: Port 8120 (default in `modules/numbering/src/config.rs`).

**Minor issue:** The plan lists 8 specific entity names, but the numbering service accepts **any string** 1-100 characters as an entity. There is no server-side validation restricting to those 8. This is not a problem (the plan's entities will work fine), but an implementing agent might incorrectly assume the server validates entity names and waste time investigating if they see unexpected behavior.

**Existing demo-seed state verified:**
- Cargo.toml confirms `reqwest`, `rand`, `rand_chacha`, `sha2` dependencies.
- `ChaCha8Rng::seed_from_u64(seed)` confirmed in `src/seed.rs`.
- DigestTracker in `src/digest.rs`: sorts by `(resource_type, correlation_id)`, builds canonical JSON with fields `type`, `correlation_id`, `value`, then SHA256.
- Current CLI args: `--tenant`, `--seed` (default 42), `--ar-url` (default `http://localhost:8086`), `--customers` (default 2), `--invoices-per-customer` (default 3), `--print-hash`.
- Only seeds AR: `mod ar;` is the only module. Execution loop creates customers then invoices per customer.

**Verdict:** Implementable. The critical invariant about preserving AR RNG call sequence is well-stated. The entity name detail is cosmetic.

---

### bd-3vpcd — GL chart of accounts seeding

**ISSUE C3: FX rate request body is incomplete**

The plan says "2 FX rates via POST /api/gl/fx-rates: USD/EUR (0.92), USD/GBP (0.79)" but doesn't document the full request body. The actual `CreateFxRateRequest` in `modules/gl/src/http/fx_rates.rs` requires:

```rust
pub struct CreateFxRateRequest {
    pub base_currency: String,
    pub quote_currency: String,
    pub rate: f64,
    pub effective_at: DateTime<Utc>,  // REQUIRED - not in plan
    pub source: String,               // REQUIRED - not in plan
    pub idempotency_key: String,      // REQUIRED - not in plan
}
```

An implementing agent needs to know:
1. What `effective_at` timestamp to use (should be deterministic — e.g., the same `2026-01-01T00:00:00Z` constant used for BOM effectivity)
2. What `source` value to use (e.g., "demo-seed")
3. What `idempotency_key` format to use (this is how FX rate creation achieves idempotency — duplicate key returns 200 with `created: false`)

The plan says "idempotency: 409 treated as success" for GL seeding, but FX rates don't use 409 — they use idempotency key dedup with a 200 response. The account creation endpoint (to be built in bd-3ng0x) will use 409, but FX rates are different.

**Recommended fix:** Add explicit FX rate request body to the bead description, including `effective_at: "2026-01-01T00:00:00Z"`, `source: "demo-seed"`, and `idempotency_key: "{tenant}-fx-{base}-{quote}-{seed}"`.

---

### bd-20bq8 — Party seeding

**ISSUE C1 (BLOCKER): Idempotency-Key header is NOT wired**

The plan states: "Idempotency: via `Idempotency-Key` HTTP HEADER (not body-level), value: `{tenant}-company-{seed}-{idx}`"

**Source code reality:** The Party module's spec (`docs/PARTY-MODULE-SPEC.md`) explicitly states HTTP idempotency key infrastructure is "deferred to v2." The database table `party_idempotency_keys` exists but **no HTTP handler checks or processes this header**. The headers currently extracted by party handlers are only `x-correlation-id` and `x-actor-id`.

This means:
- Re-running the seeder will attempt to create duplicate companies
- Without a dedup mechanism, the seeder will either create duplicates or fail

**Options to fix:**
1. **Add idempotency-key support to party handlers** (new prerequisite bead, or expand bd-20bq8 scope)
2. **Use a check-before-create pattern** — GET companies by name/legal_name and skip if exists
3. **Use the existing `x-correlation-id` header** if the service actually deduplicates on it (it doesn't appear to)

Other Party claims verified correct:
- `POST /api/party/companies` exists at `modules/party/src/http/mod.rs` line 21.
- Company type only — `PartyType` enum is `company` | `individual`, no customer/supplier.
- Tags as `Vec<String>` on the party model.
- Required fields: `display_name`, `legal_name`. Optional: `tax_id`, `registration_number`, plus many more.
- Contacts: `POST /api/party/parties/{party_id}/contacts`. Required: `first_name`, `last_name`. Optional: `email`, `phone`, `role`, `is_primary`, `metadata`.
- Addresses: `POST /api/party/parties/{party_id}/addresses`. Required: `line1`, `city`. `address_type` is optional (not required to be "registered").
- Port 8098 confirmed.

---

### bd-g0f3n — Inventory seeding

**API accuracy: VERIFIED CORRECT**

All inventory claims checked out:
- `POST /api/inventory/uoms`: requires `code`, `name`. Returns 409 on duplicate (PostgreSQL error 23505 mapped to `UomError::DuplicateCode`).
- `POST /api/inventory/locations`: requires `warehouse_id: Uuid`, `code`, `name`. Optional: `description`.
- `POST /api/inventory/items`: requires `sku`, `name`, `inventory_account_ref`, `cogs_account_ref`, `variance_account_ref`, `tracking_mode`. Optional: `description`, `uom` (defaults to "ea"), `make_buy`.
- 409 on duplicate SKU confirmed.
- No warehouse creation API — only `GET /api/inventory/warehouses/{warehouse_id}/locations` (read-only).
- Port 8092 confirmed.
- `tracking_mode` enum: `none`, `lot`, `serial` — matches plan's per-item assignments.

**Minor note:** The plan doesn't mention the optional `make_buy` field on items. Setting this would improve data quality (raw materials as "buy", manufactured parts as "make"), and the plan already knows which items are buy vs make. An implementing agent should set this field.

**UUID v5 warehouse strategy:** Viable. The location endpoint accepts any UUID for `warehouse_id` — it's a foreign key in concept but there's no FK constraint to a warehouse table (since no warehouse creation API exists). Deterministic UUID v5 from `(NAMESPACE_DNS, "{tenant}-warehouse-{seed}")` will work.

---

### bd-8g1ka — BOM seeding

**ISSUE C2 (BLOCKER): Check-before-create via GET is not possible as described**

The plan states: "Check-before-create via GET: Natural key (tenant_id, part_id) for BOMs, (bom_id, revision_label) for revisions."

**Source code reality:** The BOM module has these GET endpoints:
- `GET /api/bom/{bom_id}` — by primary key UUID only
- `GET /api/bom/{bom_id}/revisions` — lists ALL revisions for a BOM (no filter by label)
- `GET /api/bom/revisions/{revision_id}/lines` — lists all lines for a revision

There is **no endpoint to look up a BOM by `(tenant_id, part_id)`**. The `{bom_id}` parameter is a UUID primary key, not the `part_id`. An implementing agent cannot check whether a BOM already exists for a given part without knowing its UUID.

**However**, the database has unique constraints:
- `UNIQUE (tenant_id, part_id)` on BOMs
- `UNIQUE (bom_id, revision_label)` on revisions
- `UNIQUE (revision_id, component_item_id)` on lines

So duplicates will return **409 Conflict** (the handler catches error code 23505). The practical solution is:

**Recommended fix:** Change the idempotency strategy to "409-as-success + parse response for existing ID" or "add a GET-by-part-id query endpoint" (new prerequisite). The 409 approach is simpler but requires the 409 response body to include the existing BOM's UUID so downstream operations (add revision, add lines) can proceed.

**Verify:** Does the BOM 409 response include the existing resource's ID? If not, the seeder has no way to get the BOM UUID after a conflict. This would require either:
1. Adding a GET-by-part-id endpoint (service change)
2. Modifying the 409 handler to return the existing ID (service change)
3. Using a list endpoint with filtering (if available)

Other BOM claims verified:
- `POST /api/bom`: accepts `{ part_id: Uuid, description: Option<String> }`. Plan omits optional `description`.
- `POST /api/bom/{bom_id}/revisions`: accepts `{ revision_label: String }`. Correct.
- `POST /api/bom/revisions/{revision_id}/lines`: accepts `{ component_item_id: Uuid, quantity: f64, uom: Option<String>, scrap_factor: Option<f64>, find_number: Option<i32> }`. Plan omits optional `uom` and `find_number`.
- Effectivity: `POST /api/bom/revisions/{revision_id}/effectivity` with `{ effective_from: DateTime<Utc>, effective_to: Option<DateTime<Utc>> }`. Correct.
- Port 8107 confirmed.

---

### bd-2g3br — Production seeding

**ISSUE C4: Routing `item_id` is Optional, not required**

The plan says: `POST /api/production/routings: { name, item_id: Uuid, revision: "1" }`

**Source code reality** (`modules/production/src/domain/routings.rs`):

```rust
pub struct CreateRoutingRequest {
    pub tenant_id: String,
    pub name: String,                           // required
    pub description: Option<String>,            // optional (not in plan)
    pub item_id: Option<Uuid>,                  // OPTIONAL (plan says required)
    pub bom_revision_id: Option<Uuid>,          // optional (not in plan)
    pub revision: Option<String>,               // optional, defaults to "1"
    pub effective_from_date: Option<NaiveDate>,  // optional (not in plan)
}
```

The plan's usage will still work (sending an `item_id` to an `Option<Uuid>` field is fine), but the plan should note that `item_id` is optional at the API level. The unique constraint is on `(tenant_id, item_id, revision)`, so 409 dedup still works when `item_id` is provided.

**ISSUE C5: Step time fields are optional**

The plan says steps have `setup_time_minutes` and `run_time_minutes` as if required, but they are `Option<i32>` in the actual struct. The plan's usage will work (sending values to Optional fields is fine), but an implementing agent should know the API won't reject a step missing these fields.

**Additional finding:** Routing steps have a validation that the routing must be in "draft" status and the workcenter must be active. The plan doesn't mention this, but since work centers are created fresh in this same bead, they'll be active by default. Not a problem in practice.

Other Production claims verified:
- `POST /api/production/workcenters`: `code` (unique/tenant), `name` required; optional: `description`, `capacity` (i32), `cost_rate_minor` (i64). Correct.
- 409 on duplicate workcenter code, routing (item_id, revision), step sequence — all confirmed via PostgreSQL 23505 error handling.
- Port 8108 confirmed.

---

### bd-56y0d — E2E test + convenience script

**No source code to verify** (this creates new files). Evaluation based on plan consistency:

- Count assertions (20 accounts, 8 policies, 10 parties, 5 UoMs, 7 locations, 13 items, 5 BOMs, 6 work centers, 5 routings) are consistent with the bead descriptions.
- Deterministic rerun test depends on ALL seeding being deterministic — the FX rate `effective_at` and BOM effectivity date being constants is critical.
- Idempotent rerun test will fail if Party idempotency (C1) and BOM idempotency (C2) aren't resolved.
- Module selection test is straightforward.

---

## Answers to Specific Questions

### What is the single biggest risk in this plan?

**Party idempotency (C1).** The plan claims an idempotency mechanism that doesn't exist in the running service. Unlike the BOM issue (which has a workaround via 409 responses), the Party module has no dedup mechanism at all — no idempotency header processing, no unique constraint on company names, nothing. A re-run will create duplicate companies, which will cascade into duplicate contacts and addresses, breaking the determinism and idempotency E2E tests. This requires either a service change or a fundamentally different seeding strategy (e.g., list-and-match before create).

### What is the most likely bead to fail during implementation?

**bd-8g1ka (BOM seeding).** It has the most complex data flow (5 BOMs, each with a revision, each with multiple component lines requiring UUIDs from inventory), the most fragile idempotency story (no natural-key GET, 409 response may not include existing ID), and the deepest dependency chain. An implementing agent will need to solve the "how do I get the existing BOM UUID after a 409" problem, which may require reading list endpoints or requesting a service change.

### If you had to cut one bead to ship faster, which would it be and what would break?

**bd-2g3br (Production seeding).** Production routings and work centers are the most downstream data — nothing else depends on them. The E2E test would need its assertions adjusted (remove work center and routing counts), but all other modules would seed correctly. BOM seeding would still work (BOMs don't reference routings). The only loss is that downstream ERP tests requiring routing data for work order generation would need to create their own test routings.

---

## Dependency Correctness

The DAG is correct. No circular dependencies. All declared dependencies are necessary:

- bd-3vpcd needs bd-3ng0x (GL account creation API) and bd-2ob3y (CLI framework) — correct
- bd-g0f3n needs bd-3vpcd (GL accounts for item refs) and bd-2ob3y — correct
- bd-8g1ka needs bd-g0f3n (item UUIDs for BOMs) — correct
- bd-2g3br needs bd-g0f3n (item UUIDs for routings) — correct
- bd-20bq8 needs bd-2ob3y (CLI framework) — correct
- bd-56y0d needs all of the above — correct

**One potential missing dependency:** bd-20bq8 (Party) is shown depending only on bd-2ob3y, which is correct — Party doesn't need GL accounts or inventory items. The DAG correctly allows Party to run in parallel with GL+Inventory.

---

## Determinism Assessment

Determinism is well-handled overall:

- ChaCha8 RNG from u64 seed — deterministic
- Correlation IDs from `{tenant}-{type}-{seed}-{idx}` — deterministic
- BOM effectivity fixed at 2026-01-01T00:00:00Z — deterministic
- UUID v5 for warehouses — deterministic

**One gap:** The FX rate `effective_at` field isn't specified in the plan. If an implementing agent uses `Utc::now()`, determinism breaks. The plan should specify `2026-01-01T00:00:00Z` (same constant as BOM effectivity).

---

## Manufacturing Realism

The aerospace job shop scenario is realistic:

- Material choices (Ti-6Al-4V, Inconel 718, AL-7075-T6, 4130 steel, HexPly 8552 prepreg) are standard aerospace alloys.
- Work center types and rates are reasonable for a mid-size job shop.
- BOM structures make physical sense (turbine blade from titanium bar, engine mount from aluminum sheet with fasteners).
- Routing sequences follow standard manufacturing flow (machine → heat treat → grind → NDT).
- Previous fix moving fasteners off the turbine blade was correct — turbine blades are monolithic forged/machined parts.

**One minor quibble:** The Structural Rib BOM uses both 4130 steel (3 units) and Inconel 718 (1 unit). In practice, a structural rib would typically be a single material. This isn't wrong (bi-metallic assemblies exist), but it's unusual. Not worth changing.

---

## Versioning

All affected modules are v0.1.0 (unproven). No version bumps or REVISIONS.md entries are required for any bead. This simplifies implementation.

---

## Summary of Required Actions

**Must fix before publishing:**

1. **C1 — Party idempotency:** Either add idempotency-key handler support to the Party service (new prerequisite bead) or change the seeding strategy to list-and-match. Document whichever approach is chosen.
2. **C2 — BOM check-before-create:** Either add a GET-by-part-id endpoint to the BOM service, or change to 409-as-success strategy with confirmed response body containing existing ID. Verify what the current 409 response body looks like.
3. **C3 — FX rate request body:** Add `effective_at`, `source`, and `idempotency_key` to the GL seeding bead description.

**Should fix (improves implementability):**

4. **C4 — Production routing fields:** Note that `item_id` and `revision` are optional at API level (with defaults).
5. **C5 — Step time fields:** Note these are optional.
6. **FX rate idempotency model:** Correct from "409 treated as success" to "idempotency_key dedup returns 200 with created=false."
7. **Inventory `make_buy` field:** Recommend setting this optional field since the plan already knows which items are buy vs make.
8. **Numbering entities:** Note that entity names are not validated server-side (any 1-100 char string accepted).
