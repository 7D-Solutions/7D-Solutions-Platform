# Doc-Mgmt Migration Safety (Phase 58 Gate A)

This module follows a forward-fix migration policy. We do not run destructive
down-migrations in production.

## Pre-Deploy Validation

1. Run migrations against a production-like clone:
   - `sqlx migrate run`
2. Run module test suites:
   - `cargo test -p doc_mgmt`
   - `cargo test -p doc_mgmt --test real_e2e`
   - `cargo test -p doc_mgmt --test gate_a_hardening`
3. Verify all new objects exist and are tenant-scoped:
   - `documents`, `revisions`, `retention_policies`, `legal_holds`,
     `document_distributions`, `document_distribution_status_log`, `doc_outbox`.

## Incident Rollback Strategy

If a deployment with new migration(s) introduces a defect:

1. Stop serving writes for `doc-mgmt`.
2. Roll application binary back to previous known-good version.
3. Apply a forward-fix SQL migration that:
   - Preserves existing data.
   - Restores endpoint compatibility.
   - Keeps tenant isolation constraints and unique indexes intact.
4. Redeploy fixed binary and run smoke tests.

## Forward-Fix Rules

- No `DROP TABLE` or destructive column removal in hotfixes.
- Prefer additive schema changes (`ADD COLUMN`, `CREATE INDEX`, compatibility triggers).
- Keep guard -> mutation -> outbox behavior unchanged in write paths.
- Any schema hotfix must include an integration test reproducing the original failure.
