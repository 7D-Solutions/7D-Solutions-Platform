# ttp — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.1 | 2026-02-24 | bd-217r | Wired `optional_claims_mw` JWT verification middleware into TTP router chain. Middleware extracts verified claims from Authorization header and makes them available to route handlers via request extensions. Layer placed after rate_limit_middleware, before AuthzLayer — matching GL's proven pattern. | Security audit finding C1: TTP had no JWT verification middleware, meaning all requests bypassed token validation. | No |
| 1.0.0 | 2026-02-22 | bd-2dq8 | Initial proof. Metering ingestion (idempotent, keyed by `idempotency_key`), price trace computation (deterministic, tenant-scoped), billing run execution (one run per tenant+period, one-time charges marked post-invoice, trace_hash linkage from metering to AR invoice line items). Health (`/healthz`) and readiness (`/api/ready`) endpoints. All E2E tests passing (metering_integration + billing_metering_integration). Proof command: `./scripts/proof_ttp.sh`. | TTP module build complete. Billing idempotency proven: billing runs are replay-safe with `was_noop:true` on re-invocation, and metering events deduplicate via `idempotency_key`. | — |

## How to read this table

- **Version:** The version in `Cargo.toml` after this change.
- **Date:** When the change was committed.
- **Bead:** The bead ID that tracked this work.
- **What Changed:** A concrete description of the change. Name specific endpoints, fields, events, or behaviors affected. Do not write "various improvements" or "minor fixes."
- **Why:** The reason the change was necessary. Reference the problem it solves or the requirement it fulfills.
- **Breaking?:** `No` if existing consumers are unaffected. `YES` if any consumer must change code to handle this version. If YES, include a brief migration note or reference a migration guide.

## Rules

- Add a new row for every version bump. One row per version.
- Do not edit old rows. If a previous change is reversed, add a new row explaining the reversal.
- The commit that bumps the version in the package file must also add the row here. Same commit.
- If the change is breaking (MAJOR version bump), the "Breaking?" column must describe what consumers need to change.
