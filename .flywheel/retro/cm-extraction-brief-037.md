# CM Extraction Brief — Retro Run #037

## Instructions

# Session Analysis Instructions

You are analyzing a coding session to extract reusable rules for the playbook.

## What to Look For

1. **Patterns that led to success**
   - What approaches worked well?
   - What debugging strategies helped?
   - What architectural decisions paid off?

2. **Patterns that caused problems**
   - What mistakes were made?
   - What approaches failed?
   - What should be avoided?

3. **Workflow insights**
   - How was work prioritized?
   - How were tasks coordinated?
   - What communication patterns helped?

4. **Tool-specific knowledge**
   - CLI quirks or gotchas
   - API format surprises
   - Configuration patterns

## Rule Formulation Guidelines

- Write rules as **imperative statements** ("Always...", "Never...", "When X, do Y")
- Be **specific** enough to be actionable
- Include **context** about when the rule applies
- For anti-patterns, prefix with "AVOID:" or "PITFALL:"

## Categories to Use

- debugging
- testing
- architecture
- workflow
- documentation
- integration
- collaboration
- git
- security
- performance

## Example Rules

- [debugging] "Before implementing a fix, search the codebase to verify the issue still exists"
- [workflow] "When claiming a task, first check its current status - another agent may have completed it"
- [integration] "When parsing JSON from external CLIs, handle both arrays and wrapper objects"
- [testing] "Always run the full test suite before committing"
- [architecture] "Use centralized constant files instead of hardcoding magic strings"
- [testing] "AVOID: Mocking entire modules in tests - prefer mocking specific functions"

## After Analysis

**Recommended (batch add):** Create a JSON array and add all rules at once:

```bash
# Create rules.json with extracted rules:
# [{"content": "Rule 1...", "category": "debugging"}, {"content": "Rule 2...", "category": "testing"}]

cm playbook add --file rules.json --check
```

**Alternative (single add):** Add rules one at a time:

```bash
cm playbook add "Your rule content" --category "category" --check
```

The `--check` flag validates rules before adding (similarity, quality, category match).

## Sessions (3 loaded)

### Session 1: /Users/james/.claude/projects/-Users-james-Projects-7D-Solutions-Platform
- Path: `/Users/james/.claude/projects/-Users-james-Projects-7D-Solutions-Platform/fee63ba7-a52c-4062-8b24-9d6eb22ece5a.jsonl`
- Messages: 934

[user] Bead bd-3fy is available: "Tax commit/void lifecycle tied to invoice finalize/refund". Claim it and begin work:
  br update bd-3fy --status in_progress --assignee DustySpring
  br show bd-3fy
[user] DustySpring
[user] Inbox for DustySpring:
====================
(No messages)
[user] Updated bd-3fy: Tax commit/void lifecycle tied to invoice finalize/refund
  assignee: LilacFinch → DustySpring
2026-02-17T20:57:21.755685Z  INFO beads_rust::sync: Auto-flush complete exported=180
◐ bd-3fy · Tax commit/void lifecycle tied to invoice finalize/refund   [● P1 · IN_PROGRESS]
Owner: PurpleCliff · Type: task
Created: 2026-02-17 · Updated: 2026-02-17
Assignee: DustySpring

Wave 2. Depends on provider adapter seam. On invoice finalize, commit tax via provider and persist commit id; emit tax.committed. On refund/void, void tax via provider and persist void result; emit tax.voided. Finalize commits exactly once; retries do not double-commit. Refund/void triggers tax void exactly once with auditable linkage. how_to_think: Tax commit/void must align with financial lifecycle and be idempotent. Avoid committing twice; persist provider commit ids and enforce once-only transitions. Files: modules/ar/src/finalization.rs, modules/ar/src/routes.rs, modules/ar/src/events/, e2e-tests/src/tax_commit_void_e2e.rs. Verify: cargo test -p e2e-tests tax_commit_void_e2e -- --nocapture

Dependencies:
  -> bd-29j (blocks) - Provider adapter seam + cached quote storage (local-tax provider)

Dependents:
  <- bd-jsko (blocks) - Stabilization Gate harness (bench runner + metrics schema + report writer)
[assistant] I see there's already a commit `[bd-3fy]` in the recent history. Let me check the current state of the work.
[user] c6c7d37c [bd-1jc] Add Phase 24a integrated lifecycle E2E (contract → schedule → recognition → amendment → amended recognition)
88f6624c [bd-1qi] Contract amendments v1: register E2E test, fix idempotency assertion
4630ae8b [bd-2il] Remove old flat report_query_repo.rs (replaced by module dir)
0e8800b6 [bd-2il] Split gl/src/repos/report_query_repo.rs (594 LOC) into 3-file module
7c28c858 [bd-3fy] Tax commit/void lifecycle: service functions, HTTP routes, migration, E2E tests
3ff2e619 [bd-2yi.3] Fix broken include_str! path: migration 000009 → 000010 for tax_jurisdictions
84ded726 [bd-2w0] Split fx_revaluation_service.rs: extract helpers into fx_helpers.rs
d4b17724 [bd-2yi] Add Phase 24b integrated E2E: accrual → close → reversal → cash flow reconciliation
99052459 [bd-1v5] Split ar/src/dunning.rs (757 LOC) into 3-file module
9b9c448c [bd-2yi.2] Add missing match arms for DuplicateModification and ContractNotFound in revrec route
d9dee55f [bd-2yi.1] Fix second broken include_str! path in reconciliation_matching_e2e.rs
0017c1a2 [bd-2yi.1] Fix broken include_str! path: migration 000008 → 000009 for recon_scheduled_runs
28d9a0f7 [bd-2w3] Add cash flow statement v1: classification, service, route, E2E tests
138ede7b [bd-iry] Split ar/src/tax.rs (1318 LOC) into 5-file module
2c17f4b2 [bd-2fu] Remove unused Utc import from reporting currency E2E tests
1117184c [bd-2fu] Add reporting currency statement endpoints and E2E tests
6b0671a2 [bd-344] Split ar/src/events/contracts.rs (1498 LOC) into 6 domain modules
cbb7fe8a [bd-2fmq] Split currency_conversion.rs: extract types into currency_types.rs
59412567 [bd-18x] Split ar/src/routes.rs (5091 LOC) into 16 domain modules
e26045a7 [bd-a7g] Split gl/src/services/period_close_service.rs into 3 modules
[assistant] The bd-3fy commit is in history. Let me check if it passes verification or if there's remaining work.
[queue-operation] {"task_id":"b9b2334","description":"Run the bd-3fy E2E verification test","task_type":"local_bash"}
[user] .
[user] The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.
[user] [Request interrupted by user fo

[... truncated, 43821 chars total ...]

### Session 2: /Users/james/.claude/projects/-Users-james-Projects-7D-Solutions-Platform
- Path: `/Users/james/.claude/projects/-Users-james-Projects-7D-Solutions-Platform/bec230d9-01a2-4177-8f66-c8fbd1239e7c.jsonl`
- Messages: 4616

[user] Bead bd-1wuh has been claimed for you: "Maintenance scheduler tick: evaluate due plans (calendar/meter/both), emit maintenance.plan.due". Begin work:
  br show bd-1wuh
[user] PurpleCliff
[user] Inbox for PurpleCliff:
====================
[○ normal] From: BrightHill | PDF Editor beads available
12 new beads for the Fireproof PDF Editor module (Rust conversion) are in the pool. Scaffold (bd-2o6u) already claimed by SageDesert. bd-1fwv (S3 storage) and bd-7r4l (DB schema) are unblocked and ready to claim. Run bv-claim to pick up work.
---
[○ normal] From: BrightHill | Vision doc beads available
16 vision doc beads in the pool — one per platform module. All unblocked, no dependencies. Claim with bv-claim. Template: modules/maintenance/docs/MAINTENANCE-MODULE-SPEC.md. Read the module source, document what exists.
---
[○ normal] From: BrightHill | Stand by — Phase 54 planning
Phase 53 (Password Reset) is complete. Pool is empty. Awaiting user direction for Phase 54. Stand by — new beads will appear once planning is done. No action needed.
---
[user] ◐ bd-1wuh · Maintenance scheduler tick: evaluate due plans (calendar/meter/both), emit maintenance.plan.due   [● P2 · IN_PROGRESS]
Owner: PurpleCliff · Type: task
Created: 2026-02-24 · Updated: 2026-02-24
Assignee: PurpleCliff

Dependencies:
  -> bd-1dcy (blocks) - Maintenance plans + assignments: create, assign to assets, compute next due fields
  -> bd-1lmd (blocks) - Maintenance HTTP: assets + meter types/readings CRUD with rollover validation

Dependents:
  <- bd-16az (blocks) - Maintenance notification integration: stable NATS subjects + event envelope compliance
  <- bd-2x15 (blocks) - Maintenance overdue detection: emit overdue events with idempotent dedup
  <- bd-22f2 (blocks) - Maintenance auto-create work orders from due plans + approval gate config
[user] ◐ bd-1wuh · Maintenance scheduler tick: evaluate due plans (calendar/meter/both), emit maintenance.plan.due   [● P2 · IN_PROGRESS]
Owner: PurpleCliff · Type: task
Created: 2026-02-24 · Updated: 2026-02-24
Assignee: PurpleCliff

Dependencies:
  -> bd-1dcy (blocks) - Maintenance plans + assignments: create, assign to assets, compute next due fields
  -> bd-1lmd (blocks) - Maintenance HTTP: assets + meter types/readings CRUD with rollover validation

Dependents:
  <- bd-16az (blocks) - Maintenance notification integration: stable NATS subjects + event envelope compliance
  <- bd-2x15 (blocks) - Maintenance overdue detection: emit overdue events with idempotent dedup
  <- bd-22f2 (blocks) - Maintenance auto-create work orders from due plans + approval gate config
[user] agent-activity.jsonl
bash-enforcement-log.jsonl
beads.db
close-events.jsonl
issues.jsonl
last-touched
mail-read.jsonl
reserve-pending
retro-seq
runner-cycles.jsonl
[user] Exit code 2
[user] Agent-first issue tracker (SQLite + JSONL)

Usage: br [OPTIONS] <COMMAND>

Commands:
  init         Initialize a beads workspace
  create       Create a new issue
  q            Quick capture (create issue, print ID only)
  list         List issues
  show         Show issue details
  update       Update an issue
  close        Close an issue
  reopen       Reopen an issue
  delete       Delete an issue (creates tombstone)
  ready        List ready issues (unblocked, not deferred)
  blocked      List blocked issues
  search       Search issues
  dep          Manage dependencies
  label        Manage labels
  epic         Epic management commands
  comments     Manage comments
  stats        Show project statistics
  status       Alias for stats
  count        Count issues with optional grouping
  stale        List stale issues
  lint         Check issues for missing template sections
  defer        Defer issues (schedule for later)
  undefer      Undefer issues (make ready again)
  config       Configuration management
  sync         Sync database with JSONL file (export or import)
  doctor       Run read-only diagnostics
  info         Show diagnostic metadata about the 

[... truncated, 193116 chars total ...]

### Session 3: /Users/james/.claude/projects/-Users-james-Projects-7D-Solutions-Platform
- Path: `/Users/james/.claude/projects/-Users-james-Projects-7D-Solutions-Platform/cc38a02b-2d2f-4f50-adf8-8f82b6389c30.jsonl`
- Messages: 2628

[user] Bead bd-2fof has been claimed for you: "Ph41: Implement per-tenant entitlement override grant/revoke with justification and E2E". Begin work:
  br show bd-2fof
[assistant] 


[user] PurpleCliff
[user] Inbox for PurpleCliff:
====================
[○ normal] From: BrightHill | Re: Idle — bd-1j64 status
PurpleCliff — MaroonHarbor has bd-1j64 (Foundation). Once Foundation closes, all 24 downstream Phase 41 beads unblock. Run bv-claim after Foundation closes. Standing by is the correct action.

— Platform Orchestrator
---
[○ normal] From: BrightHill | bd-1j64 is open — claim it
bd-1j64 (Phase 41 Foundation) is OPEN in the pool. Run bv-claim to pick it up. All 24 downstream beads unblock when Foundation closes. — Platform Orchestrator
---
[○ normal] From: BrightHill | STOP — do not build yet, release bd-1j64
URGENT: Stop work on bd-1j64 immediately. The user wants to review the plan before any building starts. Do not write any code. Release bd-1j64 back to OPEN status: br update bd-1j64 --status open --owner '' — BrightHill
---
[○ normal] From: BrightHill | bd-1j64 released — claim it now
bd-1j64 (Foundation) was accidentally claimed during a pool check. It is now OPEN. Run bv-claim.sh to claim it. — BrightHill
---
[○ normal] From: BrightHill | Phase 41 pool open — 25 beads live
Phase 41 TCP UI is fully planned. 25 beads are now in the pool.

Entry point: bd-1j64 (FOUNDATION) — claim it with bv-claim.sh. Everything else depends on Foundation. Once Foundation is done, 8 parallel beads open up.

Stack: Next.js App Router + TypeScript, shadcn/ui + Tailwind, TanStack Query, Playwright. Location: apps/tenant-control-plane-ui/. Reference: docs/reference/fireproof/ and docs/frontend/.

Key invariants every agent must protect:
- Browser never calls Rust directly (BFF routes only)
- Staff JWT in httpOnly cookie only (never client JS)
- platform_admin enforced in middleware + every BFF handler
- All persistent state in Zustand stores (ESLint rules prevent ad-hoc useState)
- Tests hit real backend (no mocks)

— BrightHill
---
[○ normal] From: BrightHill | Pool empty — awaiting frontend planning
Pool is empty pending ChatGPT planning for the next phase (Tenant Control Plane frontend). No beads to create yet. Stand by.
---
[○ normal] From: BrightHill | 2 new beads in pool
PurpleCliff — 2 new beads added. Claim with bv-claim:

- bd-1iqz: Treasury cash position — bank accounts + transactions → daily cash position report
- bd-exxn: Fixed assets depreciation run — asset created, depreciation run fires GL journal entry

—BrightHill
---
[○ normal] From: BrightHill | 3 new beads in pool
PurpleCliff — 3 more beads just added:

- bd-cxyg: Write-off — mark AR invoice uncollectible, verify GL write-off event
- bd-1xtm: Inventory reservation — reserve, fulfill, cancel, verify quantities
- bd-1e5j: AP bill approval — submit/approve/reject state machine

Claim with bv-claim.

—BrightHill
---
[○ normal] From: BrightHill | New beads — bd-28dx and bd-35dm
PurpleCliff — 2 more beads added. Claim with bv-claim:

- bd-28dx: AR aging report — invoices in 0-30, 31-60, 61-90, 90+ day buckets
- bd-35dm: Credit note — issue credit against invoice, verify balance reduced + event fired

Great work on bd-nn3g — fast close.

—BrightHill
---
[○ normal] From: BrightHill | New beads in pool + uncommitted changes handled
PurpleCliff — 4 new E2E beads just added to the pool. Claim one with bv-claim:

- bd-3hzd: GL automation — AR invoice.created fires GL journal entry via NATS
- bd-nn3g: Payments → AR settlement — payment received marks invoice paid
- bd-25wo: Tenant lifecycle — active → suspended → reactivated, login blocked while suspended
- bd-1v9t: Notification delivery — AR invoice.created triggers customer notification

Regarding your uncommitted changes in modules/ap/src/main.rs and modules/treasury/src/http/import.rs — I committed those under bd-n8ze (commit 18811ba9). Your working tree for those files should now be clean. scripts/visual-session-manager.sh changes 

[... truncated, 126821 chars total ...]

## Current Playbook (935 rules)

- [workflow] Always check bead status with br show before claiming — another agent may have completed or claimed it
- [workflow] When bash hook blocks a command due to no active bead, create or claim a bead first rather than bypassing the hook
- [architecture] Always verify file paths referenced in bead descriptions against the actual codebase before starting implementation
- [debugging] When a shell script uses SCRIPT_DIR to derive PROJECT_ROOT, verify the relative path depth matches the actual directory structure
- [architecture] Never hardcode agent names in scripts — use dynamic resolution via agent-mail-helper.sh whoami or environment variables
- [architecture] When writing hooks that create files atomically, use tmp-file-plus-mv pattern to prevent partial writes on crash
- [integration] When implementing timeout wrappers for external CLI tools, use perl alarm or timeout command with graceful fallback on failure
- [integration] Before running cm onboard or cm reflect, verify cm doctor shows healthy status and required directories exist
- [collaboration] When sending agent mail, use normal priority unless the message requires immediate action — overuse of urgent degrades signal
- [workflow] When a pre-edit hook blocks editing .beads/issues.jsonl, use br commands instead — never bypass the hook or edit the file directly
- [git] Always prefix git commit messages with the bead ID in brackets like [bd-xxx] when working on a bead to maintain traceability
- [integration] Before calling cm onboard sample, verify session files still exist on disk — cass index may contain stale entries for deleted files
- [debugging] When deriving PROJECT_ROOT from SCRIPT_DIR in shell scripts, count the actual directory depth rather than assuming a fixed relative path
- [collaboration] When agents report idle status, check if blocked beads have been unblocked before creating new work — dependencies may have resolved
- [collaboration] Avoid sending stale bead reminder notifications repeatedly — check if a reminder was already sent before dispatching another
- [architecture] When writing coordination scripts that create files, always export PROJECT_ROOT so child processes and Python subshells can access it
- [workflow] Use cm context with --json flag at bead claim time to inject relevant playbook rules into the agent work brief
- [collaboration] When checking agent mail inbox, check once and act on results — do not poll the inbox repeatedly in a loop waiting for new messages
- [integration] When an external AI bridge returns a truncated response, split the request into smaller chunks rather than re-sending the same large prompt
- [workflow] AVOID: Creating temporary scripts via cat heredoc in Bash — use the Write tool instead to ensure proper file visibility and tracking

## Output

Write extracted rules as JSON array to a file, then re-run with:
```bash
CM_CANDIDATE_RULES=/path/to/rules.json run-retro.sh --window 10
```

Format: `[{"content": "Always ...", "category": "debugging"}, ...]`
Max 10 rules per run. Rules must be imperative, 20-200 chars, valid category.
