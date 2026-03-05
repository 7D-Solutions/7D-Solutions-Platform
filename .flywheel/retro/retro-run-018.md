# Retro Run #018 — 2026-03-05

**Trigger:** count-based (5 closes since last retro)
**Analysis window:** 6 closes since retro 017 (retro_seq 430–435)
**Runner:** CopperRiver (manual — run-retro.sh not found, bd-296zo)

## Beads Analyzed

| Bead | Title | Agent | Commits | Key pattern |
|------|-------|-------|---------|-------------|
| bd-1odbc | RIE: Retro + CM learning — 5 bead closes | CopperRiver | 1 | Prior retro run (meta bead) |
| bd-2ilnj | Review manufacturing drill-down tabs | PurpleCliff | 0 | Independent review |
| bd-87f5a | Manufacturing lifecycle drill-down tabs | BrightHill | 1 | Plan artifact creation |
| bd-d8xl4 | Rename Engineering tab to BOM & Change Control | BrightHill | 1 | Premature rename |
| bd-zsw4m | Revert tab 2 name back to Engineering | BrightHill | 1 | Immediate revert |
| bd-p4mx2 | Phase 0: Manufacturing design lock | MaroonHarbor | 3 | Multi-round review incorporation |

## Signals

- **Closes in window:** 6
- **Avg commits per bead (code beads only):** 1.5 (bd-p4mx2: 3, bd-87f5a: 1, bd-d8xl4: 1, bd-zsw4m: 1)
- **Agent spread:** BrightHill (3), CopperRiver (1), PurpleCliff (1), MaroonHarbor (1)
- **Reopen count:** 0
- **Zero-commit beads:** 1 (bd-2ilnj review)
- **Revert pair:** bd-d8xl4 renamed, bd-zsw4m reverted immediately

## Patterns Observed

### 1. Design lock documents benefit from multi-reviewer iteration
bd-p4mx2 went through 3 commits: initial document, then fixes from DarkOwl + SageDesert review, then fixes from ChatGPT review. Each round caught different issues — contradictory WIP language, event naming confusion, NATS subject pattern errors. One-way-door decisions like cost rollup flow and WIP representation need at least two independent review rounds before sign-off.

### 2. Don't rename established terminology without stakeholder buy-in first
bd-d8xl4 renamed the Engineering tab to "Planning" (then "BOM & Change Control"), and bd-zsw4m immediately reverted it back to "Engineering." This created two unnecessary beads and two commits for zero net change. Industry-standard ERP terminology (Engineering, Production, Quality) should be kept unless the user explicitly requests a rename — don't optimize naming on your own initiative.

### 3. NATS subjects are plain event_type strings, not dotted module paths
bd-p4mx2 commit 3 corrected a recurring misconception: NATS subjects in this platform are plain strings like `inventory.item_issued`, not hierarchical `module.events.event_type` patterns. The GL consumer subscribes to these flat subjects. This was caught by ChatGPT review and is a foundational fact that keeps resurfacing.

### 4. WIP in this platform is GL-only — Inventory has no WIP state
bd-p4mx2 commit 2 removed contradictory "WIP location" language. WIP exists only as a GL account balance; Inventory tracks items as issued or received, never "in WIP." This distinction matters for every manufacturing module touching cost flows.

### 5. Review beads with zero commits still provide value
bd-2ilnj was a review bead with no code commits — PurpleCliff's feedback was incorporated into bd-87f5a's corrections. Review beads generate value through feedback, not through their own commits. Don't judge bead value by commit count alone.
