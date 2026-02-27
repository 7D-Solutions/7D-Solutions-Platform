# Project Instructions

## Rules

- Do NOT use the Task tool to spawn subagents. Do all work directly.
- Do NOT ask the user what to do. Work autonomously.
- Keep commits small and focused.
- **Pre-existing issues:** If you encounter a bug, broken test, dead code, incorrect logic, or any other pre-existing issue in files you are working in — do NOT silently skip it. Either fix it (as a child bead if out of scope) or mail the orchestrator (BrightHill) describing the issue, its location, and severity. Never leave known problems unreported.

## Cargo Build Slots (MANDATORY)

**Never call `cargo` directly.** Use the slot system to avoid build lock contention:

```bash
./scripts/cargo-slot.sh test -p inventory-rs    # instead of: cargo test -p inventory-rs
./scripts/cargo-slot.sh build -p inventory-rs   # instead of: cargo build -p inventory-rs
./scripts/cargo-slot.sh test --workspace         # instead of: cargo test --workspace
```

This routes through 4 independent build slots so multiple agents can compile in parallel. If all slots are busy, the script waits automatically.

## File Size Limit

Keep source files under 500 LOC. If a file would exceed 500 LOC after your changes, split it into logical modules first. Files over 500 LOC without an entry in `.file-size-allowlist` will fail CI.

## Agent Mail

**First time:** Register in the mail system:
```bash
./scripts/agent-mail-helper.sh register "Your role"
```

**Every session:** Check identity and inbox:
```bash
./scripts/agent-mail-helper.sh whoami
./scripts/agent-mail-helper.sh inbox
```

Inbox messages are work assignments. **Act on them autonomously.**

## Beads Workflow (MANDATORY)

All work MUST be tracked with a bead. Edits are blocked until you have an active bead.

**IMPORTANT: Never bypass or disable hooks. If an edit is blocked, create a bead first.**

**Start of session:**
```bash
./scripts/br-start-work.sh "Your task title"  # Create new bead
# OR
./scripts/bv-claim.sh                          # Claim recommended bead
```

**Commits:** Always prefix with bead ID:
```bash
git commit -m "[bd-xxx] Your commit message"
```

**End of work:** Close your bead:
```bash
br close bd-xxx
```

## When No Beads Are Available

If `./scripts/bv-claim.sh` returns nothing claimable:

1. **Do NOT manually browse beads** with `br show` or `br list`. The pool is managed — if bv-claim found nothing, there is nothing for you.
2. **Check your inbox:** `./scripts/agent-mail-helper.sh inbox` — the orchestrator may have sent you direction.
3. **If no inbox messages:** send a single message to the orchestrator reporting idle status. Include which beads are blocking the pool (bv-claim output will show this).
4. **Wait for a response.** Do not loop or retry bv-claim repeatedly.
5. **Never claim a blocked bead** (one with unfinished dependencies) — `br` will reject it anyway.

## Autonomous Work Loop

If running inside `scripts/agent-runner.sh`:

1. Check inbox: `./scripts/agent-mail-helper.sh inbox`
2. Claim a bead: `./scripts/bv-claim.sh` — if nothing returned, follow **When No Beads Are Available** above.
3. Commit with `[bd-xxx]` prefix
4. Side issues: Create child beads, fix them, close them
5. When done: `br close bd-xxx`
6. A bead is only done when ALL child beads are also closed
7. After `br close`, context clears automatically - do NOT run `next-bead.sh` manually

## Module Versioning (MANDATORY)

**Full standard:** See [docs/VERSIONING.md](./docs/VERSIONING.md) for the complete system.

**Quick rules for agents:**

1. **Check before you edit:** If the module's package file (`Cargo.toml` or `package.json`) has version >= `1.0.0`, it is a **proven module**. Extra rules apply.
2. **Proven module changes require a version bump.** Bump the version in the package file before committing. PATCH for fixes, MINOR for new features, MAJOR for breaking changes.
3. **Add a revision entry.** Every version bump requires a new row in the module's `REVISIONS.md`. Describe what changed, why, and whether it is breaking.
4. **Same commit.** The version bump, revision entry, and code change go in the same commit.
5. **Breaking changes (MAJOR):** Note the migration path in the revision entry. Send agent mail to the orchestrator flagging the breaking change.
6. **Unproven modules (v0.x.x):** No version bumps or revision entries required. Change freely.

**For product agents adopting a new module version:**

1. Read the module's `REVISIONS.md` for all changes since your current pinned version.
2. Update `MODULE-MANIFEST.md` with the new version.
3. Run your product's E2E tests against the new version.
4. Commit the manifest change with a note explaining the adoption.

## Frontend Work

**Plugin:** `frontend-design@claude-code-plugins` is installed. Use it when working on any frontend bead (TCP UI or future apps).

**Frontend location:** `apps/tenant-control-plane-ui/` (Next.js App Router + TypeScript, shadcn/ui + Tailwind)

**Standards:** See `docs/frontend/PLATFORM-FRONTEND-STANDARDS.md` and `docs/frontend/TCP-UI-VISION.md`.

📧 **Multi-Agent Communication**: See [AGENT_MAIL.md](./AGENT_MAIL.md) for commands.

🎯 **Beads Workflow**: See [AGENTS.md](./AGENTS.md) for task tracking with BV.

🔍 **Codebase Search**: Use `fsfs search "query" --format json` for semantic codebase search. See [AGENTS.md](./AGENTS.md) for the full reasoning loop (cass → fsfs → grep).
