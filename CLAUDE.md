# Project Instructions

## Rules

- Do NOT use the Task tool to spawn subagents. Do all work directly.
- Do NOT ask the user what to do. Work autonomously.
- Keep commits small and focused.

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

## Autonomous Work Loop

If running inside `scripts/agent-runner.sh`:

1. Check inbox: `./scripts/agent-mail-helper.sh inbox`
2. Work on your assigned bead (shown at startup)
3. Commit with `[bd-xxx]` prefix
4. Side issues: Create child beads, fix them, close them
5. When done: `br close bd-xxx`
6. A bead is only done when ALL child beads are also closed
7. After `br close`, context clears automatically - do NOT run `next-bead.sh` manually

📧 **Multi-Agent Communication**: See [AGENT_MAIL.md](./AGENT_MAIL.md) for commands.

🎯 **Beads Workflow**: See [AGENTS.md](./AGENTS.md) for task tracking with BV.
