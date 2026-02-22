# ar — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-02-22 | bd-rqbr | Initial proof. Customer lifecycle (create/update/suspend/reactivate, status-gated operations), invoice lifecycle (draft → open → paid/void/written-off, aging buckets 0-30/31-60/61-90/90+), credit notes (issue credit against invoice, balance reduction + event fired), Tilled webhook ingestion (HMAC-SHA256 verification, idempotency via `event_id` deduplication), payment allocation (partial and full), write-offs, dunning scheduler, GL journal entry emission via NATS (`ar.invoice.created`, `ar.payment.received`), reconciliation, outbox/inbox pattern for event delivery, health (`/healthz`) and readiness (`/api/ready`) endpoints. All E2E tests passing. Proof command: `./scripts/proof_ar.sh`. Staging payment loop + webhook replay proof: `./scripts/staging/payment_loop.sh`. | AR module build complete. Invoice → webhook → posting path proven idempotent: duplicate Tilled events deduplicate via `event_id`, replay returns HTTP 200 with no state corruption. | — |

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
