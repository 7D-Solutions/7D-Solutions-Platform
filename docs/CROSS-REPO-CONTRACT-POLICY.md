# Cross-Repo Contract Change Policy

This document defines how contract shape changes are announced to consumers,
how long old versions remain supported, and how to recover when a change breaks
a downstream consumer.

"Contract" here means any shared payload type defined in `platform-contracts`
or emitted by a platform module — `EventEnvelope` variants, SDK request/
response structs, NATS subject schemas, and HTTP API bodies.

See also: [docs/COMPATIBILITY-MATRIX.md](COMPATIBILITY-MATRIX.md)

---

## 1. Contract Semver Rules

Apply these rules to any change in a contract type. The rules mirror standard
semver but are stated explicitly for payload shapes, which carry stricter
backward-compatibility obligations than internal code.

| Change type | Version bump | Examples |
|-------------|-------------|---------|
| **PATCH** | Field rename where the old name is preserved as an alias; internal validation tightening that cannot be observed by a well-formed consumer | Adding a `#[serde(alias = "old_name")]` annotation |
| **MINOR** | New optional field added; new enum variant added to a non-exhaustive enum; new subject or endpoint added without altering existing ones | Adding `line_item_notes?: Option<String>` to an invoice event |
| **MAJOR** | Field removed; field renamed without alias; required field added; enum variant removed; subject or endpoint renamed or removed; serialization format changed | Removing `legacy_ref` from `PaymentPosted`; changing NATS subject from `ap.invoice.created` to `ap.event.invoice.created` |

**Rule:** A MAJOR change requires a new contract version in the type name or
subject path (e.g., `InvoiceCreatedV2`, `ap.v2.invoice.created`) so that old
and new consumers can coexist during the deprecation window.

---

## 2. Deprecation Window

The deprecation window is **two release trains (~4 weeks)**.

Timeline for a MAJOR contract change:

```
Week 0   — New contract version ships. Old version marked deprecated in code
           (Rust: #[deprecated], doc comment with removal target date).
           Announcement sent (see §3).

Week 2   — First reminder sent to any consumer orchestrator that has not yet
           migrated. Cross-watcher integration tests must pass against both
           old and new contract versions during this period.

Week 4   — Old contract version removed. Any consumer still on the old version
           will fail to compile or receive events. No further extensions
           without explicit approval from the Platform Orchestrator.
```

MINOR changes carry no deprecation window; old consumers simply ignore the new
field and remain functional.

PATCH changes are transparent; no announcement required beyond a REVISIONS.md
entry.

---

## 3. Announcement Channel

When a MAJOR contract change is released:

1. **Platform release notes** — Add an entry to `docs/PLATFORM-RELEASE-NOTES.md`
   under the current release train heading. Mark it with `[BREAKING CONTRACT]`.

2. **Direct mail to downstream orchestrators** — Send an agent-mail message to
   every orchestrator whose modules consume the changed contract type. Use the
   subject format:
   ```
   [CONTRACT BREAKING] <TypeName> — migration required by <ISO date>
   ```
   Body must include: what changed, the new type/subject name, the removal
   date, and a link to the REVISIONS.md entry.

3. **COMPATIBILITY-MATRIX.md update** — Add a note in the Notes section
   recording the old and new type names and the removal date.

For MINOR changes, a release notes entry is sufficient. No direct mail is
required unless the change affects a high-traffic subject.

---

## 4. Rollback Procedure

If a contract change breaks a consumer after the old version has been removed:

1. **Identify the break** — Check consumer module's compile errors or runtime
   panics against the event log. Confirm the contract mismatch is the cause.

2. **Revert or hotfix** — Two options in order of preference:
   - **Preferred:** Hotfix the consumer to accept the new contract. This keeps
     the platform on the forward path and avoids re-introducing a deprecated
     type.
   - **Fallback:** Reintroduce the old contract type as a compatibility shim
     (suffixed `Compat`, e.g., `InvoiceCreatedV1Compat`) in `platform-contracts`
     behind a feature flag. Ship a PATCH release of `platform-contracts`. This
     buys the consumer time to migrate but must itself be removed within one
     additional release train.

3. **Post-mortem bead** — Create a `bug` bead with type `P1` documenting why
   the consumer was not migrated within the window. Root cause must be
   addressed before the next MAJOR change in the same contract family.

4. **Never re-open the removal window** — Extending the deprecation window
   retroactively is not permitted. The fallback shim path above is the only
   sanctioned recovery.

---

## 5. Quick Reference

```
PATCH  — transparent; REVISIONS.md only
MINOR  — additive; release notes entry
MAJOR  — versioned type + 4-week window + announcement mail + matrix note
```

Rollback order: hotfix consumer → compat shim (one extra train) → P1 post-mortem bead.
