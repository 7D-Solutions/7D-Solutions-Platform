# {Product Name} — Module Manifest

> **What this file is:** The source of truth for which platform module versions this product runs. The product's deployment configuration must match this file exactly. No module version changes without updating this file first.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Pinned Module Versions

| Module | Version | Last Validated | Notes |
|--------|---------|---------------|-------|
| {module-name} | 1.0.0 | YYYY-MM-DD | Initial proof |

## Platform Component Versions

| Component | Version | Last Validated | Notes |
|-----------|---------|---------------|-------|
| identity-auth | 1.0.0 | YYYY-MM-DD | Initial proof |
| event-bus | 1.0.0 | YYYY-MM-DD | Initial proof |
| tenant-registry | 1.0.0 | YYYY-MM-DD | Initial proof |

## How to read this table

- **Module / Component:** The platform module or component this product depends on.
- **Version:** The exact version this product has been tested and validated against. This is the version deployed to production.
- **Last Validated:** The date someone ran the product's E2E tests against this version and confirmed it works.
- **Notes:** Why this version was adopted, any known issues, or migration notes for breaking changes.

## Rules

- Every module and platform component this product calls (HTTP or NATS) must have a row in this file.
- The deployment configuration must reference the exact versions listed here. No `latest` tags in production.
- To adopt a new version:
  1. Read the module's `REVISIONS.md` for all changes between your current version and the new one.
  2. If any change is breaking, update product code first.
  3. Update the version and date in this file.
  4. Run the product's E2E tests.
  5. Commit this file (and any code changes) together.
- Do not update multiple modules in a single manifest change unless they are coupled (e.g., a platform component update that requires a module update). One module adoption per commit makes rollback straightforward.
- If a module in the registry is newer than your pinned version, that is normal. You adopt when ready, not when available.

## Adoption Log

Record each adoption decision below. This provides an audit trail of why the product moved to each version.

| Date | Module | From | To | Reason | Bead |
|------|--------|------|----|--------|------|
| YYYY-MM-DD | {module-name} | 1.0.0 | 1.0.1 | Tax rounding fix needed for Q1 invoicing | bd-xxxx |
