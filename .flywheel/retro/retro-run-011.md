# Retro Run #011 — 2026-03-03

**Trigger:** count-based (5 closes since last retro)
**Analysis window:** last 5 closes (retro_seq 368–372)
**Runner:** CopperRiver (manual — run-retro.sh not found, bd-2s9fq)

## Beads Analyzed

| Bead | Title | Agent | Commits | Key pattern |
|------|-------|-------|---------|-------------|
| bd-7nvjh | AR progress billing / milestone invoicing | CopperRiver | 1 | Over-billing guard, cumulative invariant in single TX |
| bd-268ml | Phase 57 coordination | BrightHill | 2 | Dockerfile fixes, Orchestrator bead (not implementation) |
| bd-2h1ng | Inventory: item classifications + commodity codes | PurpleCliff | 2 | CHECK constraint extension needed a follow-up migration |
| bd-2nddc | Reporting: scheduled delivery | CopperRiver | 1 | Schedule/execution lifecycle, Guard→Mutation→Outbox |
| bd-120qw | PDF editor: rich formatting (tables) | MaroonHarbor | 1 | Deterministic render primitives, golden fixture tests |

## Signals

- **Avg commits per bead:** 1.4
- **Agent spread:** CopperRiver (2), PurpleCliff (1), BrightHill (1), MaroonHarbor (1)
- **Reopen count:** 0 across 5 beads
- **Fix-after-commit count:** 1 (bd-2h1ng needed CHECK constraint migration)
- **Cross-bead commit bundling:** 1 (commit 119fc1e7 contained changes from 3 beads)

## Patterns Observed

### 1. DB CHECK constraints break when adding new enum values
bd-2h1ng added a `classification_assigned` change type to the item change history, but the existing `item_change_history_change_type_check` constraint blocked the INSERT. A follow-up migration was needed to DROP and re-ADD the constraint with the new value. When adding new domain event types or status values, always check for CHECK constraints on the target table before writing code.

### 2. Multiple beads' changes bundled in a single commit
Commit 119fc1e7 included changes for bd-2h1ng (inventory CHECK migration), bd-120qw (PDF editor tables), and bd-2nddc (reporting schedules). This makes git bisect ineffective and violates the one-bead-one-commit principle. Each bead's changes should be in a separate commit with its own `[bead-id]` prefix.

### 3. Cumulative business invariants belong in the Guard phase
bd-7nvjh's over-billing guard validates `cumulative_billed + new_amount <= contract_total` inside a `SELECT ... FOR UPDATE` lock. This is the correct pattern — cumulative invariants must be checked under a row lock in the Guard phase, not as an after-the-fact validation. Protects against race conditions from concurrent billing requests.

### 4. Proven module version bumps are happening consistently
bd-7nvjh correctly bumped AR from v1.0.55 to v1.0.56 with a REVISIONS.md entry in the same commit. This is the versioning protocol working as intended.

### 5. Deterministic output assertions use golden fixture comparison
bd-120qw's table render tests assert byte-identical output from the same inputs, using golden fixture comparison. This is the right pattern for any rendering/serialization code where output stability matters. The test proves format stability across code changes.

### 6. Schedule lifecycle models follow a clear state machine
bd-2nddc models delivery schedules with an active/disabled status and an executions table that records every trigger attempt (success/failure). This execution audit trail pattern prevents silent schedule failures — the same pattern used by bd-12j00's shipping doc state machine.

### 7. Integration with cross-cutting audit trail via helper functions
bd-2h1ng's classification assignment calls `record_change_in_tx()` from the change history module to append an audit row atomically within the same transaction. This shared helper pattern ensures cross-cutting concerns (audit, events) are consistently applied without duplicating SQL across features.

### 8. Idempotency guard runs before the transaction begins
Both bd-7nvjh and bd-2nddc check idempotency_key existence before opening a transaction. The reporting service even returns the existing record early. This avoids holding a transaction open for the idempotency lookup, reducing lock contention under retries.
