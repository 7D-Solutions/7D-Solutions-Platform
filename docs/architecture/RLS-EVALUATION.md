# Postgres RLS Evaluation

This note records the GAP-12 evaluation for row-level security as a defense-in-depth layer for the core financial tables.

## Decision

**Decline for now.**

The platform already enforces tenant isolation in application code with explicit `tenant_id` / `app_id` predicates, tenant-aware pool resolution, and audit trails. PostgreSQL RLS would be a valid second layer only if every connection consistently carries tenant context. That is not true in the current stack.

## Tables Reviewed

The bead called out the most sensitive financial tables. In this repository, the concrete tables are:

- `journal_entries` in `modules/gl`
- `ar_invoices` in `modules/ar`
- `vendor_bills` in `modules/ap`
- `payment_attempts` in `modules/payments`
- `audit_events` in `platform/audit`

## Why It Was Deferred

- The runtime note in `platform/platform-sdk/src/tenant_resolver.rs` already records the blocker: RLS via `SET LOCAL app.current_tenant` conflicts with `pgbouncer` statement mode.
- The codebase is not uniform on tenant scoping. Some tables use `tenant_id`, others use `app_id`, and `audit_events` is append-only rather than tenant-scoped.
- Enforcing RLS safely would require connection-level context injection everywhere a pool is acquired, plus policy work for mixed-schema tables such as `journal_lines`.
- The current application-layer isolation is consistent and easier to verify across the existing modules and tests.

## Current Defense Layers

1. Tenant-aware query predicates in module repositories and handlers.
2. Tenant-specific or module-specific database pools where needed.
3. Append-only or immutable audit protections where relevant.
4. Boundary checks preventing cross-module SQL and source coupling.

## Revisit Conditions

RLS should be revisited only after:

1. The pool layer can reliably run `SET LOCAL app.current_tenant` on every connection.
2. `pgbouncer` is moved to session or transaction mode.
3. The relevant tables have a single, consistent tenant key.
4. Integration tests can prove cross-tenant reads and writes fail closed.

