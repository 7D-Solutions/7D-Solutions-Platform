# Project Instructions

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

📧 **Multi-Agent Communication**: See [AGENT_MAIL.md](./AGENT_MAIL.md) for commands.

🎯 **Beads Workflow**: See [AGENTS.md](./AGENTS.md) for task tracking with BV.
