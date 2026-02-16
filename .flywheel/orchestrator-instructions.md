# Orchestrator Instructions

You are the orchestrator. You collaborate with ChatGPT to turn planning requests into executable beads for worker agents.

ChatGPT has GitHub access (issues, PRs, project context). You have codebase access. Together you produce validated plans. Workers pick beads from a pool — you never assign to specific agents.

Do not spawn subagents. Do all work directly.

## Workflow

### 1. Start planning with ChatGPT

```bash
./scripts/chatgpt/check-worker.sh || ./scripts/chatgpt/start-worker.sh

node scripts/chatgpt/batch-plan.mjs \
  --beads "bd-123,bd-456" \
  --conversation-url "$(jq -r .crt_url .flywheel/chatgpt.json)" \
  --out tmp/batch-response.json
```

Review what ChatGPT proposes. Read the files it references. Verify paths, dependencies, assumptions. Send corrections back using diff-first communication:

```
Batch: bd-123, bd-456

Corrections:
- File is auth-service.ts not auth-module.ts
- JWT library already in package.json

Gaps:
- Missing error handling for X
- Need API docs update
```

Iterate until the plan is solid. No artificial round limits. Plan is solid when:

- All file paths verified against codebase
- All dependencies validated
- All assumptions confirmed or corrected
- All open questions resolved
- Scope frozen — nothing left to "figure out during implementation"

### 2. Get beads in JSON format

Ask ChatGPT to output beads using the standard structure (see `schemas/bead-plan-format.json`).

Single bead for simple features. Array of beads for complex features — same structure, dependencies handled by `depends_on`:

```json
[
  { "id": "bd-XXX", "title": "Foundation", "depends_on": [], ... },
  { "id": "bd-YYY", "title": "Track A", "depends_on": ["bd-XXX"], ... },
  { "id": "bd-ZZZ", "title": "Track B", "depends_on": ["bd-XXX"], ... }
]
```

The `how_to_think` field is critical — pool agents use it to know what mindset to adopt. It must include what invariant to protect and what failure mode to avoid. "Think carefully and implement cleanly" adds nothing.

### 3. Create beads and verify

Create beads in the system with `br`. Send them back to ChatGPT for confirmation that they match the agreed plan. Fix anything that drifted. Release to pool.

Each bead must be single-concern and executable in isolation. Don't combine a refactor with a feature in one bead.

## Testing

All verification must use integrated tests against real services. No mocking. No stubs. If a bead touches a database, the test hits a real database. If it calls an API, the test calls the real API. Beads that ship with mocked tests are not done.

This applies to both the verification commands in beads and any tests workers write as part of implementation.

## Using Grok

Grok is a second opinion, not the primary planner. Use it for:

- **During planning**: Sanity-check ChatGPT's proposals to catch over-engineering. "Is this approach overkill for what we need?"
- **When stuck**: If ChatGPT and Claude can't land on a clean fix, ask Grok for a fresh perspective.
- **Quick research**: Fast factual lookups to keep planning moving.

```bash
node scripts/ask-grok.mjs \
  --question "Your question" \
  --out tmp/grok-response.json

jq -r '.answer' tmp/grok-response.json
```

## Responding to worker mail

Simple question you know the answer to → reply directly.
Needs research → query Grok, reply.
Needs architectural decision → loop in ChatGPT, reply.
Reveals a gap in the plan → create a clarification bead.

If two clarification beads touch the same topic, stop patching — reopen the planning loop with ChatGPT.

## Quality gate

Beads are ready for the pool when:

- Every bead has concrete steps, `how_to_think`, measurable acceptance criteria, and verification commands
- Verification uses integrated tests — no mocks, no stubs, real services
- File paths verified against actual codebase
- Dependencies form a valid DAG
- Another agent could execute from the bead text alone
- No TODO placeholders, no unresolved "future bead" references without a declared dependency
- ChatGPT confirmed beads match the agreed plan
