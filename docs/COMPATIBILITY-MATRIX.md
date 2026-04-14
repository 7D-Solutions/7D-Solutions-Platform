# Compatibility Matrix

This document records known-good version combinations for the current release
train. Use it when adopting a snapshot of the platform or checking whether a
set of module versions has already been exercised together.

Keep this file aligned with [docs/PLATFORM-SERVICE-CATALOG.md](PLATFORM-SERVICE-CATALOG.md)
and the per-crate `REVISIONS.md` files.

## Current Release Train

**Snapshot date:** 2026-04-14

| Layer | Modules | Known-good versions |
|-------|---------|---------------------|
| Auth + runtime | `identity-auth`, `security`, `platform-sdk`, `health`, `event-bus`, `platform-contracts` | `1.10.2`, `1.8.1`, `0.1.0`, `1.1.0`, `2.1.0`, `1.0.1` |
| Finance core | `ap`, `ar`, `gl`, `treasury`, `fixed-assets`, `reporting`, `subscriptions`, `party` | `3.6.0`, `6.6.0`, `3.3.0`, `2.1.10`, `2.1.8`, `3.0.0`, `5.2.1`, `3.2.5` |
| Manufacturing + fulfillment | `inventory`, `shipping-receiving`, `production`, `quality-inspection`, `bom`, `maintenance`, `workflow` | `2.7.0`, `3.4.2`, `3.5.1`, `3.1.1`, `2.4.1`, `2.3.0`, `2.2.1` |
| Customer-facing + integrations | `customer-portal`, `integrations`, `notifications`, `timekeeping`, `pdf-editor`, `consolidation`, `workforce-competence` | `2.3.2`, `2.7.1`, `3.3.1`, `3.0.2`, `2.2.0`, `2.3.1`, `2.2.0` |
| Control plane + platform ops | `control-plane`, `tenant-registry`, `doc-mgmt`, `audit`, `projections`, `auth-kit`, `blob-storage`, `config-validator`, `event-consumer`, `tax-core` | `1.6.1`, `1.2.0`, `1.2.1`, `1.0.0`, `1.1.0`, `0.1.0`, `0.1.0`, `0.1.0`, `1.0.0`, `1.0.0` |

## How To Use This Matrix

1. Start from the current release train if you want a known-good baseline.
2. If you change one version in a row, treat the whole row as a new snapshot.
3. Update the matching `REVISIONS.md` files before you publish the new set.
4. Re-run the relevant module tests and product adoption tests before promoting.

## Notes

- This matrix is intentionally coarse-grained. It documents the sets that have
  been exercised together, not every theoretical combination.
- For the exact deployed inventory, use [docs/PLATFORM-SERVICE-CATALOG.md](PLATFORM-SERVICE-CATALOG.md).
- For per-module change history, use the module's `REVISIONS.md`.
