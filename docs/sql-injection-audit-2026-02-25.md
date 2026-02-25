# SQL Injection Audit — 2026-02-25

**Bead:** bd-edwv
**Auditor:** PurpleCliff
**Scope:** All 18 modules, platform crates, tools, and e2e tests
**Method:** Searched every `sqlx::query*(&format!(...))` call site and every `format!()` containing SQL keywords

## Executive Summary

**No SQL injection vulnerabilities found.** All dynamic SQL uses either:

1. Compile-time `const` values (table/column lists)
2. Allowlist-validated identifiers with regex + allowlist defense
3. Values derived from typed internal state (UUIDs, f64 from database)
4. CLI/operator-only tooling (not reachable from HTTP)

One **low-severity note** on `usage_billing.rs` is documented below as a code-quality improvement opportunity.

---

## Methodology

1. `grep` for `sqlx::query*(&format!(...))` across all `.rs` files
2. `grep` for `format!()` containing SQL keywords (SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, DROP)
3. For each hit: trace the interpolated value to its source; classify as safe/unsafe
4. Verify that parameterized `$N` binds are used for all user-supplied values

---

## Findings by Category

### 1. Const Column Lists in format!() — SAFE

These files use `format!()` to interpolate a `const &str` containing column names into SQL. The interpolated value is a compile-time constant, never user-supplied.

| File | Const | Usage |
|------|-------|-------|
| `modules/timekeeping/src/domain/approvals/service.rs` | `APPROVAL_COLS` | `RETURNING {APPROVAL_COLS}` in 8 queries |
| `modules/timekeeping/src/domain/export/service.rs` | `EXPORT_COLS` | `RETURNING {EXPORT_COLS}` in 5 queries |
| `modules/fixed-assets/src/domain/disposals/service.rs` | `DISPOSAL_COLUMNS` | `SELECT {DISPOSAL_COLUMNS} FROM ...` in 4 queries |

**Verdict:** No risk. Values are `const &str` defined at compile time.

### 2. Allowlist-Validated Projection Table Names — SAFE

The `platform/projections` crate uses `format!()` for table names but validates them through a two-layer defense:

- **Layer 1:** Regex — only `^[a-z_][a-z0-9_]*$` identifiers allowed
- **Layer 2:** Allowlist — only known table names in `ALLOWED_PROJECTION_TABLES`

Files using this validation:

| File | Validation Call |
|------|-----------------|
| `platform/projections/src/digest.rs:42-44` | `validate_projection_name()` + `validate_order_column()` |
| `platform/projections/src/admin.rs:197-199` | `validate_projection_name()` + `validate_order_column()` |
| `platform/projections/src/rebuild.rs:158,220,224,233` | Consumes already-validated `base_table` from callers |

**Verdict:** No risk. Well-designed defense-in-depth with tests covering injection attempts.

### 3. Const Table Name Arrays — SAFE

These files iterate over hardcoded `const` arrays of table names:

| File | Const Array | SQL Pattern |
|------|-------------|-------------|
| `modules/reporting/src/metrics.rs:242-271` | `CACHE_TABLES` (6 entries) | `SELECT COUNT(*) FROM {table}` |
| `modules/reporting/src/domain/kpis/mod.rs:240` | Hardcoded `&[...]` in test | `DELETE FROM {table} WHERE tenant_id = $1` |
| `modules/reporting/src/domain/forecast/cash_forecast.rs:191` | Hardcoded `&[...]` in test | `DELETE FROM {table} WHERE tenant_id = $1` |

**Verdict:** No risk. Iteration over compile-time string slices.

### 4. Parameterized Dynamic WHERE Clauses — SAFE

| File | Pattern |
|------|---------|
| `modules/ar/src/routes/events.rs:32-64` | Builds `WHERE` clause with `$N` placeholders, binds all values |

This is a correct dynamic query builder: it uses `format!("column = ${}", param_count)` to generate bind-parameter placeholders, then `.bind()` for every value. No user data is ever interpolated into SQL.

**Verdict:** No risk. Textbook parameterized query construction.

### 5. CLI/Operator-Only Tools — SAFE (not HTTP-reachable)

These tools use `format!()` for SQL but are operator-only CLI commands, not reachable from any HTTP endpoint:

| File | Value Interpolated | Source |
|------|-------------------|--------|
| `tools/tenantctl/src/provision.rs:167` | `tenant_db_name` | `format!("tenant_{}_{}_db", TenantId, module.name)` — TenantId is a UUID, module.name is a const |
| `tools/tenantctl/src/lifecycle.rs:352` | `db_name` | Same pattern as provision |
| `tools/tenantctl/src/fleet_migrate.rs:192` | `tenant_db_name` | Same pattern as provision |
| `tools/projection-rebuild/src/main.rs:135,242` | `projection` | CLI argument (operator-supplied) |
| `tools/stabilization-gate/src/projections.rs` | `BENCH_TABLE` const | Compile-time constant |

**Verdict:** No risk. CLI-only tooling with operator-supplied or const inputs. Note: `projection-rebuild` accepts a CLI `projection` argument that is interpolated into SQL without validation. This is acceptable because the tool is operator-only and requires RBAC authorization, but adding `validate_projection_name()` here would be a defense-in-depth improvement.

### 6. f64 Interpolation in usage_billing.rs — LOW SEVERITY NOTE

| File | Line | Value |
|------|------|-------|
| `modules/ar/src/usage_billing.rs:130-139` | `qty` (f64) | `{}::NUMERIC` in INSERT |

The `qty` value is an `f64` parsed from a database column (`row.quantity.parse::<f64>()`). Since `f64` formatting in Rust can only produce numeric characters, `-`, `.`, `e`, `inf`, `NaN`, or `-inf`, this cannot produce SQL injection. However, `NaN` or `inf` values would cause a SQL error.

**Verdict:** Not exploitable. The value is typed `f64` (not a string), so no injection is possible. However, this could be improved by using a bind parameter with an explicit CAST:

```sql
-- Current (safe but fragile):
VALUES ($1, $2, 'metered_usage', $3, {qty}::NUMERIC, $4, $5, NOW())

-- Recommended:
VALUES ($1, $2, 'metered_usage', $3, $4::NUMERIC, $5, $6, NOW())
```

This is a code-quality recommendation, not a security finding.

### 7. E2E Test Files — OUT OF SCOPE

All `e2e-tests/tests/*.rs` files use `format!()` with hardcoded const table names or test-only data. These never run in production and are not a security concern.

---

## Modules Audited

| # | Module | format!() + SQL? | Finding |
|---|--------|-------------------|---------|
| 1 | `modules/ap` | No | All queries use static strings or `sqlx::query!()` |
| 2 | `modules/ar` | Yes | events.rs: safe (parameterized). usage_billing.rs: safe (f64 typed) |
| 3 | `modules/consolidation` | No (test only) | Config test cleanup uses const table names |
| 4 | `modules/fixed-assets` | Yes | disposals: safe (const DISPOSAL_COLUMNS) |
| 5 | `modules/gl` | No | All queries use static strings |
| 6 | `modules/inventory` | No | All queries use static strings |
| 7 | `modules/notifications` | No | All queries use static strings |
| 8 | `modules/payments` | No | All queries use static strings |
| 9 | `modules/reporting` | Yes | metrics.rs: safe (const CACHE_TABLES). domain/: safe (const arrays in tests) |
| 10 | `modules/shipping-receiving` | No | All queries use static strings |
| 11 | `modules/subscriptions` | No | All queries use static strings |
| 12 | `modules/timekeeping` | Yes | approvals + export: safe (const column lists) |
| 13 | `modules/treasury` | No | All queries use static strings |
| 14 | `platform/projections` | Yes | Safe (allowlist + regex validation) |
| 15 | `platform/identity-auth` | No | All queries use static strings |
| 16 | `platform/security` | No | No SQL |
| 17 | `platform/event-bus` | No | No SQL |
| 18 | `platform/tenant-registry` | No | All queries use static strings |

**Tools (not modules, but audited):**

| Tool | Finding |
|------|---------|
| `tools/tenantctl` | Safe: UUID + const module names |
| `tools/projection-rebuild` | Safe: CLI-only, RBAC-gated. Recommend adding validation. |
| `tools/stabilization-gate` | Safe: const BENCH_TABLE |

---

## Recommendations

1. **`usage_billing.rs:130`** — Replace `f64` interpolation with a bind parameter + CAST. Not a vulnerability, but eliminates the last `format!()` value interpolation in production SQL.

2. **`projection-rebuild/src/main.rs:135`** — Add `validate_projection_name()` call before using CLI-supplied `projection` in SQL. Defense-in-depth for operator tooling.

---

## Conclusion

The codebase demonstrates strong SQL injection prevention practices:

- Production HTTP handlers use parameterized queries exclusively
- Dynamic table names are validated through allowlists
- Const column lists avoid stringly-typed queries safely
- The `platform/projections/src/validate.rs` module provides a reusable, well-tested validation layer

**Overall risk: NONE.** No user-controlled values reach SQL via string interpolation.
