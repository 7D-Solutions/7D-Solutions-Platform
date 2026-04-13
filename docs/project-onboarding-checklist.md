# New Project Onboarding Checklist

When adding a new project to the AgentCore platform, complete every item below. A project is not onboarded until all items are checked.

## 0. Docker Desktop Requirements

Before onboarding any project that uses Docker containers (Rust services, dev loop):

- [ ] **Docker compose >= v5.0.2** — our minimum supported version; older versions have known P0 issues in multi-service stacks and are not tested.
  - Verify: `docker compose version`  — must show v5.0.2 or later
  - Update via Docker Desktop: Settings → Software Updates
- [ ] **Docker Desktop VM memory ≥ 8 GB** (12 GB recommended when running multiple project stacks simultaneously)
  - Configure: Docker Desktop → Settings → Resources → Memory
  - Running four Rust-project stacks side by side (30+ containers total) can push RAM usage past the 8 GB floor — the 12 GB recommendation is for the multi-project dev workflow.
- [ ] Run the pre-flight gate: `bash scripts/check-compose-version.sh`
  - Exits 0 if compose version gate passes (hard requirement)
  - Warns (non-blocking) if VM memory is below 8 GB — fix before bringing up multiple stacks

> **Why the hard gate on compose version?** compose < v5.0.2 has P0 bugs affecting multi-service stacks. All four Rust projects (and any new ones onboarded under the Rust Service Container Spec) depend on v5.0.2+ being the baseline on every dev machine.

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
- [ ] Hand-edit `config/supervisord.conf` in AgentCore to add a `[program:agentmail-<project>]` entry for this project's mail monitor. The config is hand-maintained (the old `generate-supervisord-conf.sh` is no longer used and is being deleted — do not add new entries by regenerating).
- [ ] Reload supervisord: `supervisorctl -c config/supervisord.conf reread && supervisorctl -c config/supervisord.conf update`
- [ ] Verify mail monitors appear: `supervisorctl status | grep <project-name>`
- [ ] Verify monitors are RUNNING, not FATAL

## 5. Supervisord — Bead Stale Monitor
- [ ] Verify beadmonitor entry exists: `supervisorctl status | grep beadmonitor-<project>`
- [ ] Check logs show actual checks (not "command not found"): `tail -5 logs/beadmonitor.log`

## 6. Supervisord — Cross-Compile Watcher (for projects with Rust services)
- [ ] Hand-edit `config/supervisord.conf` to add a `[program:cross-watcher-<project-slug>]` entry. The canonical form is:
  ```
  [program:cross-watcher-<project-slug>]
  command=/Users/james/Projects/AgentCore/scripts/dev-cross-supervised.sh <absolute-project-root> [--workspace | --bin <name> --container <name>]
  directory=<absolute-project-root>
  autostart=true
  autorestart=true
  startretries=10
  startsecs=10
  environment=PROJECT_ROOT="<absolute-project-root>"
  ```
  Use `--workspace` for multi-crate Rust projects that build every crate in the workspace; use `--bin <name> --container <name>` for single-binary projects.
- [ ] Reload supervisord so the new program gets picked up: `supervisorctl -c config/supervisord.conf reread && supervisorctl -c config/supervisord.conf update`
- [ ] Verify watcher is RUNNING: `supervisorctl status | grep cross-watcher-<project>`
- [ ] **Nested workspace?** If `Cargo.toml` is not at the project root (e.g., `modules/myapp/`), create `.cargo-slot` at the project root:
  ```
  workspace_dir=modules/myapp
  ```
  Without this, `cargo-slot.sh` won't find the workspace and builds will fail.
- [ ] Reference: see `docs/rust-service-container-spec.md` Section 6 (Host wiring) for the full host-wiring contract.

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

## 13. Rust Service Container Spec Conformance (for projects with Rust services)

This section applies only when the project has one or more Rust services that run in containers. For Node-only projects (Next.js, frontends, etc.), skip to the end.

- [ ] Read `docs/rust-service-container-spec.md` in AgentCore before configuring any Rust-service compose entry. The spec is the canonical standard; this checklist is the operational companion.
- [ ] Compose file for each Rust service references `image: flywheel/rust-dev-runtime:<current-tag>` — no `build:`, no `develop.watch`, no `command:`, no `entrypoint:`.
- [ ] Binary volume mount uses `./target/aarch64-unknown-linux-musl/debug/<binary-name>:/app/service:ro`. The referenced cargo package or `[[bin]]` exists in the project's `Cargo.toml` tree.
- [ ] HTTP services (those with a `ports:` block) have `healthcheck:` using `curl -f http://localhost:<port>/api/health`. Non-HTTP services (workers, daemons) have neither a `ports:` block nor a `healthcheck:` block.
- [ ] Cross-watcher entry for this project is added to AgentCore's `config/supervisord.conf` per Section 6 above. The runtime image (`flywheel/rust-dev-runtime:<current-tag>`) already exists in the local Docker image store — built once in AgentCore, shared across all projects.
- [ ] Run the conformance linter: `python3 /Users/james/Projects/AgentCore/scripts/lint-rust-container-spec.py <project-root>` — fix every violation it flags before the first `docker compose up`.
- [ ] First boot: bring up the stack with `docker compose up -d` under `.claude-hooks-bypass`, then run the Section 9 behavioral checks from the Rust spec (container image matches canonical tag, `/app/service` exists and is executable, supervisord is PID 1, watcher program is RUNNING, healthcheck returns 200 for HTTP services, host and container binary SHA-256 match). Every check must pass.
- [ ] Add a one-line reference to `docs/dev-loop.md` (the runbook) in the project's `CLAUDE.md` so humans browsing the repo know where to find the operational documentation. Agents already see the runbook via the global rules folder at `~/.claude/rules/dev-loop.md`.

Full new-project onboarding flow for Rust services is documented in `docs/rust-service-container-spec.md` Section 11. This checklist cross-references it; the spec is the source of truth.

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

# Cargo slot symlink (for projects with Rust services)
ln -s /Users/james/Projects/AgentCore/flywheel_tools/scripts/core/cargo-slot.sh scripts/cargo-slot.sh
```

After this bootstrap, hand-edit AgentCore's `config/supervisord.conf` to add the project's mail monitor entry (Section 4) and — if the project has Rust services — the cross-watcher entry (Section 6). Then reload with `supervisorctl -c config/supervisord.conf reread && supervisorctl -c config/supervisord.conf update`.
