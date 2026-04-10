# AgentCore Feature Reference

Comprehensive reference for all AgentCore systems. For everyday agent operations, see [AGENTS.md](../AGENTS.md). For installation, see [installation.md](installation.md).

---

## MCP Tools Reference

34 tools on the running MCP mail server (`http://localhost:8765/mcp`).

### Messaging

| Tool | Description |
|------|-------------|
| `send_message` | Send a message to another agent |
| `reply_message` | Reply to an existing message |
| `fetch_inbox` | Fetch inbox messages (unread or all) |
| `search_messages` | Search messages by keyword |
| `mark_message_read` | Mark a specific message as read on server |
| `acknowledge_message` | Acknowledge receipt of a message |
| `summarize_recent` | AI summary of recent messages |
| `summarize_thread` | AI summary of a message thread |

### Identity & Contacts

| Tool | Description |
|------|-------------|
| `register_agent` | Register an agent with the mail server |
| `create_agent_identity` | Create a new agent identity |
| `list_contacts` | List known contacts for an agent |
| `whois` | Look up agent identity information |
| `request_contact` | Send a contact request to another agent |
| `respond_contact` | Accept or reject a contact request |
| `set_contact_policy` | Set auto-accept/reject policy for contacts |
| `macro_contact_handshake` | One-call contact exchange between two agents |

### Sessions & Windows

| Tool | Description |
|------|-------------|
| `macro_start_session` | Initialize a full agent session (register + project + window) |
| `macro_prepare_thread` | Set up a thread for conversation |
| `list_window_identities` | List all window/pane identities in a project |
| `rename_window` | Rename a window identity |
| `expire_window` | Mark a window as expired |
| `fetch_summary` | Fetch session summary |
| `fetch_topic` | Fetch current topic for a window |
| `ensure_project` | Create project if it doesn't exist |

### File Reservations

| Tool | Description |
|------|-------------|
| `macro_file_reservation_cycle` | Reserve files, do work, release — full cycle |
| `file_reservation_paths` | Check which files are currently reserved |
| `renew_file_reservations` | Extend reservation expiry |
| `release_file_reservations` | Release file reservations |
| `force_release_file_reservation` | Force-release another agent's reservation |
| `install_precommit_guard` | Install git pre-commit hook that checks reservations |
| `uninstall_precommit_guard` | Remove the pre-commit reservation guard |

### System

| Tool | Description |
|------|-------------|
| `health_check` | Check server health |

### Upstream-Only Tools (not yet deployed)

The local MCP server is ~10 commits behind upstream. These exist upstream but are **not available** on the running server:

| Tool | Description |
|------|-------------|
| `hard_delete_agent` | Permanently delete an agent and all its data |
| `hard_delete_project` | Permanently delete a project and all its data |
| `archive_project` | Archive a project (soft removal) |
| `retire_agent` | Retire an agent identity |
| `deregister_agent` | Remove agent registration |
| `purge_old_messages` | Bulk delete old messages |

To deploy: pull upstream changes to `~/mcp_agent_mail/` and restart the server.

---

## Hooks System

All hook logic runs in a single HTTP server: `~/.claude/hooks/hook-server.mjs` on `localhost:9876`. There are no individual hook scripts — every handler was consolidated into this file. Claude Code POSTs each event to the server rather than spawning subprocess scripts.

`~/.claude/settings.json` wires the four endpoints:

| Event | Matcher | Endpoint |
|-------|---------|----------|
| `PreToolUse` | `ExitPlanMode\|Task\|Bash\|Edit\|Write\|MultiEdit\|Read\|Glob\|Grep\|NotebookEdit\|Skill` | `POST /hooks/pre-tool-use` |
| `PostToolUse` | `Bash` | `POST /hooks/post-tool-use` |
| `SessionStart` | — | `POST /hooks/session-start` |
| `Stop` | — | `POST /hooks/stop` |

### PreToolUse handlers

Handlers run in order. The first deny short-circuits the chain. Advisory responses accumulate and are shown to the agent without blocking.

| Handler | What it guards |
|---------|---------------|
| `handlePreTaskBlock` | Blocks Task/subagent spawning when configured |
| `handlePreEditCheck` | Rejects edits to files not in the active bead's declared file list |
| `handlePreBashCheck` | Pre-execution bash safety checks |
| `handleNoDelete` | Prevents file deletion |
| `handleCrossWatcherGuard` | Blocks cross-project watcher interference |
| `handleDockerGuard` | Guards Docker operations |
| `handleDirectoryRestriction` | Blocks reads/writes outside the project directory |
| `handlePackageLockProtection` | Protects lock files from modification |
| `handleFsfsNudge` | Advisory: suggests `fsfs` before raw grep (non-blocking) |
| `handlePlanPreview` | Shows plan preview before ExitPlanMode |
| `handleSlbGuard` | Guards SLB (Simultaneous Launch Button) operations |

### PostToolUse handlers

Fires after every `Bash` call. None block.

| Handler | What it does |
|---------|-------------|
| `handlePostBashCommandLog` | Logs the command for verify-gate matching |
| `handlePostBeadClose` | Detects `br close`, increments retro counter, calls `next-bead.sh` |
| `handlePostBashTrack` | Tracks bash command history for bead activity logging |

### SessionStart / Stop handlers

| Endpoint | What it does |
|----------|-------------|
| `/hooks/session-start` | Registers agent identity, sets up mail monitor, creates tracking files |
| `/hooks/stop` | Cleans up tracking files and mail monitors |

### Bypass

```bash
touch .claude-hooks-bypass   # disable all guards for this project
rm .claude-hooks-bypass      # re-enable
```

Only create this file when the user explicitly asks. Never create it on your own initiative.

### Adding a new rule

1. Add a handler function in `hook-server.mjs`:
   ```js
   function handleMyRule(toolName, toolInput, cwd) {
     if (toolName !== 'Bash') return null;
     if (toolInput.command?.includes('dangerous-pattern')) {
       return denyResponse('MyRule: reason the command is blocked');
     }
     return null; // allow
   }
   ```
2. Add it to the `handlers` array in the `'/hooks/pre-tool-use'` case (order matters — earlier handlers run first).
3. Restart the server: `pkill -f hook-server.mjs && node ~/.claude/hooks/hook-server.mjs &`

### Debugging a blocked tool call

The deny reason appears in Claude's UI inline. To trace further:

```bash
# Confirm the server is up
curl -s http://localhost:9876/health | jq .
# → {"status":"ok"}

# Replay the blocked call directly
curl -s -X POST http://localhost:9876/hooks/pre-tool-use \
  -H 'Content-Type: application/json' \
  -d '{"tool_name":"Edit","tool_input":{"file_path":"/tmp/test.txt"},"cwd":"/Users/james/Projects/AgentCore"}' \
  | jq .

# If the server is not running, start it
node ~/.claude/hooks/hook-server.mjs
```

**Example:** Agent tries to edit `AGENTS.md` and gets blocked with "file not in bead scope." That's `handlePreEditCheck`. The active bead doesn't declare `AGENTS.md` in its file list. Fix: add the file to the bead with `br update bd-xxx --files AGENTS.md`, or switch to a bead that already declares it.

---

## Fleet Management

Scripts in `flywheel_tools/scripts/fleet/`.

| Script | Usage | Description |
|--------|-------|-------------|
| `fleet-status.sh` | `fleet-status.sh [--compact\|--watch\|--json]` | CLI dashboard: agents, tasks, reservations, mail, health |
| `swarm-status.sh` | `swarm-status.sh` | Swarm-level status overview |
| `fleet-metrics.sh` | `fleet-metrics.sh` | Agent fleet performance metrics |
| `swarm-metrics.sh` | `swarm-metrics.sh` | Swarm-level performance metrics |
| `assign-tasks.sh` | `assign-tasks.sh` | Distribute tasks across available agents |
| `fleet-tmux-status.sh` | `fleet-tmux-status.sh` | Tmux pane status for fleet agents |
| `start-orchestrator.sh` | `start-orchestrator.sh` | Launch the fleet orchestrator |
| `fleet-core.sh` | (library) | Shared functions for fleet scripts |

---

## Session Management

| Script | Location | Description |
|--------|----------|-------------|
| `spawn-swarm.sh` | `scripts/` | Spawn N agent panes in a tmux session |
| `teardown-swarm.sh` | `scripts/` | Clean teardown — kills panes, cleans tracking files |
| `visual-session-manager.sh` | `scripts/` | TUI for managing agent sessions interactively |
| `start-multi-agent-session.sh` | `scripts/` | Bootstrap a complete multi-agent session |

### Tracking File Cleanup

On teardown or bead close, these are automatically cleaned:
- `/tmp/agent-bead-{AGENT}.txt` — current bead assignment
- `/tmp/agent-identity-{PANE}.name` — agent identity
- `pids/{SAFE_PANE}.no-exit` — no-exit toggle
- `pids/{SAFE_PANE}.pid` — agent process PID

---

## Monitoring & Observability

| Script | Location | Description |
|--------|----------|-------------|
| `metrics-summary.sh` | `flywheel_tools/scripts/monitoring/` | Aggregate metrics report (`--weekly`, `--full`, `--thresholds`) |
| `reservation-metrics.sh` | `flywheel_tools/scripts/monitoring/` | File reservation usage metrics |
| `reservation-status.sh` | `flywheel_tools/scripts/monitoring/` | Current file reservation status |
| `search-metrics.sh` | `flywheel_tools/scripts/monitoring/` | Search tool (fsfs/cass) usage metrics |
| `expiry-notify-monitor.sh` | `flywheel_tools/scripts/monitoring/` | Monitor for expiring reservations/windows |
| `bead-stale-monitor-daemon.sh` | `scripts/` | Daemon that detects stale in_progress beads |
| `bead-stale-monitor.sh` | `flywheel_tools/scripts/beads/` | One-shot stale bead check |
| `performance-tracker.sh` | `scripts/` | Agent performance tracking |
| `disk-space-monitor.sh` | `scripts/` | Disk space monitoring |
| `ntm-dashboard.sh` | `scripts/` | NTM session dashboard (`--watch` for auto-refresh) |

---

## Terminal Management

Scripts in `flywheel_tools/scripts/terminal/`.

| Script | Description |
|--------|-------------|
| `terminal-inject.sh` | Queue terminal injections (commands, mail notifications) for delivery to agent panes |
| `arrange-panes.sh` | Arrange tmux panes into standard layouts |
| `cleanup-after-pane-removal.sh` | Clean up tracking files when a pane is removed |
| `renumber-panes.sh` | Renumber panes after removal to keep indices sequential |

---

## Agent Adapters

Scripts in `flywheel_tools/scripts/adapters/` for non-Claude agent integrations.

| Script | Description |
|--------|-------------|
| `deepseek-claude-wrapper.sh` | Wraps DeepSeek to work with the flywheel |
| `start-deepseek-proxy.sh` | Starts the DeepSeek compact proxy |
| `grok-claude-wrapper.sh` | Wraps Grok to work with the flywheel |
| `setup-codex-oauth.sh` | Sets up OAuth for OpenAI Codex |
| `start-mail-server.sh` | Start the MCP mail server |
| `stop-mail-server.sh` | Stop the MCP mail server |

Setup scripts in `scripts/`:

| Script | Description |
|--------|-------------|
| `setup-grok.sh` / `setup-grok-mcp.sh` | Grok agent setup and MCP config |
| `setup-deepseek.sh` | DeepSeek agent setup |
| `setup-chatgpt.sh` | ChatGPT agent setup (Playwright-based) |
| `setup-gemini.sh` | Gemini agent setup |
| `setup-openai-key.sh` | OpenAI API key configuration |

---

## Dev & Utility Tools

Scripts in `flywheel_tools/scripts/dev/`.

| Script | Description |
|--------|-------------|
| `doctor.sh` | Health check — verifies all dependencies and services |
| `self-review.sh` | Validate work before submission (up to 3 iterations) |
| `validate-agent-session.sh` | Validate agent session is properly configured |
| `summarize-session.sh` | Generate session summary |
| `task-analyzer.sh` | Analyze bead task patterns |
| `task-lifecycle-tracker.sh` | Track task state transitions |
| `generate-task-graph.sh` | Generate dependency graph visualization |
| `search-history.sh` | Search past command/search history |
| `hook-bypass.sh` | Temporarily bypass hooks (debugging) |
| `prepare-for-fresh-start.sh` | Clean slate reset |
| `launcher.sh` | Agent launcher utility |
| `file-picker.sh` | Interactive file selection |
| `macro-helpers.sh` | Shared MCP macro helpers |

---

## Advanced Bead Tools

Scripts in `flywheel_tools/scripts/beads/` beyond basic `br` commands.

### Draft Status & Publishing

```bash
br create --title "New feature"          # creates in draft status
br-publish.sh bd-XXXX                    # promote draft → open, wakes idle agents
br-publish.sh bd-XXXX bd-YYYY --notify   # publish multiple + broadcast
```

### Enhanced Bead Creation

`br-create.sh` wraps `br create` with automatic type inference and work brief enrichment from `.agent-profiles/types.yaml`.

```bash
br-create.sh "Fix login API endpoint"              # auto-infers type
br-create.sh "Add API route" --infer-type backend   # force type
# Types: general, backend, frontend, devops, docs, qa
```

### Other Bead Tools

| Script | Description |
|--------|-------------|
| `br-start-work.sh` | Claim + transition to in_progress in one step |
| `bead-quality-scorer.sh` | Score task quality (`score`, `report`, `stats`, `warn`) |
| `bead-stale-monitor.sh` | Check for stale in_progress beads |
| `log-bead-activity.sh` | Log bead activity events to JSONL |
| `bv-all-open.sh` | List all open beads across projects |
| `bv-all.sh` | List all beads regardless of status |
| `bv-open.sh` | List open beads in current project |
| `bv-sync.sh` | Sync beads state to git |

---

## Retro & Learning Tools

Scripts in `flywheel_tools/scripts/retro/`.

| Script | Description |
|--------|-------------|
| `run-retro.sh` | Execute a retrospective (data aggregation) |
| `cm-playbook-bootstrap.sh` | Seed CM playbook from historical sessions |
| `test-learning-flywheel.sh` | End-to-end smoke test for auto-retro and CM loop |

---

## Queue & Auto-Scaling

| Script | Location | Description |
|--------|----------|-------------|
| `queue-monitor.sh` | `scripts/` | Monitor bead queue depth and status |
| `auto-scaler.sh` | `scripts/` | Auto-scale agent count based on queue depth |
| `match-engine.sh` | `scripts/` | Match available agents to open beads by type/skill |

---

## Supervisord Integration

Config: `config/supervisord.conf`

```bash
./scripts/stop-supervisord.sh              # Stop all supervised services
./scripts/generate-supervisord-conf.sh     # Generate config
```

---

## File Reservations

Prevents multiple agents from editing the same file simultaneously.

```bash
$PROJECT_ROOT/scripts/reserve-files.sh list   # Check current reservations
# Automatic: macro_file_reservation_cycle MCP tool
# Pre-commit guard: install_precommit_guard / uninstall_precommit_guard MCP tools
```
