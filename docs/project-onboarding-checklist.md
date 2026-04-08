# New Project Onboarding Checklist

When adding a new project to the AgentCore platform, complete every item below. A project is not onboarded until all items are checked.

## 1. Flywheel Tools
- [ ] Symlink `flywheel_tools` to AgentCore: `ln -s /Users/james/Projects/AgentCore/flywheel_tools flywheel_tools`
- [ ] Symlink `scripts/` shared hooks (any from AgentCore/scripts/ that apply): `ln -s /Users/james/Projects/AgentCore/scripts/<name>.sh scripts/<name>.sh`
- [ ] Verify: `ls -la flywheel_tools` shows symlink to AgentCore

## 2. Beads
- [ ] Initialize beads: `br init`
- [ ] Verify `.beads/` directory exists with `issues.jsonl`
- [ ] Run `br sync` to confirm DB is healthy

## 3. Orchestrator Instructions
- [ ] Symlink `.flywheel/orchestrator-instructions.md` to canonical: `ln -s ../flywheel_tools/config/orchestrator-instructions.md .flywheel/orchestrator-instructions.md`
- [ ] Verify: `wc -l .flywheel/orchestrator-instructions.md` matches AgentCore version

## 4. Supervisord — Mail Monitors
- [ ] Ensure tmux session is running for the project
- [ ] Regenerate supervisord config: `scripts/generate-supervisord-conf.sh --reload`
- [ ] Verify mail monitors appear: `supervisorctl status | grep <project-name>`
- [ ] Verify monitors are RUNNING, not FATAL

## 5. Supervisord — Bead Stale Monitor
- [ ] Verify beadmonitor entry exists: `supervisorctl status | grep beadmonitor-<project>`
- [ ] Check logs show actual checks (not "command not found"): `tail -5 logs/beadmonitor.log`

## 6. Supervisord — Cross-Compile Watcher (if applicable)
- [ ] Add cross-watcher entry to supervisord config (for Rust projects with Docker containers)
- [ ] Verify watcher is RUNNING: `supervisorctl status | grep cross-watcher-<project>`
- [ ] **Nested workspace?** If `Cargo.toml` is not at project root (e.g., `modules/myapp/`), create `.cargo-slot` at project root:
  ```
  workspace_dir=modules/myapp
  ```
  Without this, `cargo-slot.sh` won't find the workspace and builds will fail.

## 7. Git Hooks
- [ ] Symlink pre-commit hook if needed: `ln -sf ../../scripts/pre-commit-version-check.sh .git/hooks/pre-commit`
- [ ] All hook logic runs through HTTP hook server (localhost:9876) — no standalone shell/python hooks

## 8. Agent Identity
- [ ] Orchestrator name file exists: `.flywheel/orchestrator-name`
- [ ] Agents can register: `flywheel_tools/scripts/core/agent-mail-helper.sh register "role"`
- [ ] Mail delivery works: send a test message and confirm receipt

## 9. Linting
- [ ] Symlink SoC lint: `ln -s /Users/james/Projects/AgentCore/scripts/lint-soc.sh scripts/lint-soc.sh`
- [ ] Run it: `scripts/lint-soc.sh` — verify it finds source files

## 10. Global Rules
- [ ] Verify `~/.claude/CLAUDE.md` is loaded (applies to all projects automatically)
- [ ] Verify `~/.claude/rules/*.md` are loaded (beads, mail, search, etc.)
- [ ] Project-specific CLAUDE.md should NOT duplicate global rules — only add project-specific details
- [ ] If project has an orchestrator, set `.flywheel/orchestrator-name` to the agent name

## 11. Hook Bypass Utility
- [ ] Symlink hook-bypass.sh: `ln -s /Users/james/Projects/AgentCore/flywheel_tools/scripts/dev/hook-bypass.sh scripts/hook-bypass.sh`
- [ ] Verify: `scripts/hook-bypass.sh status` shows INACTIVE

## 12. Verification
- [ ] Spawn an agent in the project: `ntm spawn <ProjectName> --cc=1`
- [ ] Agent claims a bead successfully
- [ ] Agent receives mail notifications
- [ ] Idle agent gets notified when new beads are published (within 60s)
- [ ] Agent can close bead and transition to next one

## Quick Setup Command
```bash
PROJECT="/Users/james/Projects/<NewProject>"
cd "$PROJECT"

# Flywheel tools
ln -s /Users/james/Projects/AgentCore/flywheel_tools flywheel_tools

# Beads
br init

# Orchestrator instructions
mkdir -p .flywheel
ln -s ../flywheel_tools/config/orchestrator-instructions.md .flywheel/orchestrator-instructions.md

# Lint + hook bypass
mkdir -p scripts
ln -s /Users/james/Projects/AgentCore/scripts/lint-soc.sh scripts/lint-soc.sh
ln -s /Users/james/Projects/AgentCore/flywheel_tools/scripts/dev/hook-bypass.sh scripts/hook-bypass.sh

# Orchestrator name (replace with actual agent name at session start)
echo "OrchestratorName" > .flywheel/orchestrator-name

# Regenerate supervisord (must have tmux session running first)
/Users/james/Projects/AgentCore/scripts/generate-supervisord-conf.sh --reload
```
