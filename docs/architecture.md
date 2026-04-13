# AgentCore Architecture

> System architecture for autonomous multi-agent coordination

## Overview

AgentCore implements the **Agent Flywheel** pattern: autonomous agents that continuously pick up tasks (beads), work on them, commit changes, and move to the next task without human intervention. The system enables multiple agents to coordinate on shared work while avoiding conflicts.

## Core Pattern: The Agent Flywheel

```
┌─────────────────── Agent Flywheel ───────────────────┐
│                                                       │
│  ┌──────────┐    ┌──────────┐    ┌──────────┐      │
│  │  Claim   │───>│   Work   │───>│  Commit  │      │
│  │   Bead   │    │  on Task │    │ Changes  │      │
│  └──────────┘    └──────────┘    └──────────┘      │
│       ▲                                   │          │
│       │                                   ▼          │
│  ┌──────────┐                      ┌──────────┐    │
│  │   Next   │<─────────────────────│  Close   │    │
│  │   Bead   │                      │   Bead   │    │
│  └──────────┘                      └──────────┘    │
│                                                       │
└───────────────────────────────────────────────────────┘
```

**Key Principle**: Agents operate autonomously in a continuous loop, requiring no human intervention between tasks.

## Component Architecture

### Layer 1: Task Management Backend

#### beads_rust (`br`)
- **Role**: Git-backed task tracking system
- **Data**: Tasks (beads) stored as git commits in `.beads/` directory
- **Operations**: create, update, close, list, show
- **Key Feature**: Tasks persist across sessions via git

#### beads_viewer (`bv`)
- **Role**: Task visualization and prioritization
- **Data**: Reads from beads_rust git backend
- **Operations**: list, filter, --robot-next (recommendation engine)
- **Key Feature**: TUI for task triage and dependency graph visualization

**Data Flow**:
```
br create → .beads/issues.jsonl → br sync → git commit
                                            ↓
bv read ← git log ← .beads/ ← git checkout
```

### Layer 2: Communication Infrastructure

#### mcp_agent_mail
- **Role**: Asynchronous inter-agent messaging
- **Backend**: Docker PostgreSQL + FastAPI HTTP server
- **Operations**: send, inbox, whoami, register
- **Key Feature**: Agents can communicate without blocking

**Message Flow**:
```
Agent A: send → mcp_agent_mail (HTTP API) → Docker PostgreSQL
                                                    ↓
Agent B: inbox ← mcp_agent_mail ← fetch_inbox API call
```

### Layer 3: Workflow Automation

#### flywheel_tools
- **Role**: Shell script infrastructure for agent lifecycle
- **Components**:
  - **Core**: agent-runner.sh, wake-agents.sh, next-bead.sh
  - **Hooks**: pre-edit, post-bash, session lifecycle
  - **Beads**: br-create.sh, bv-claim.sh, monitoring
  - **Fleet**: Multi-agent coordination scripts
  - **Terminal**: tmux integration and command injection
  - **Monitoring**: Performance tracking and metrics

**Execution Flow**:
```
agent-runner.sh → agent-mail (register) → bv --robot-next
                                            ↓
                  claim bead → update tracking → launch claude
                                                   ↓
                  work loop → pre-edit hook → edit files → git commit
                              ↓
                  post-bash hook → log activity → close bead → next-bead.sh
```

### Layer 4: Code Quality & Search

#### ultimate_bug_scanner (UBS)
- **Role**: Static analysis and vulnerability detection
- **Integration**: Can run as post-commit hook or manual scan
- **Output**: JSONL reports of findings

#### coding_agent_session_search (CASS)
- **Role**: Semantic search over agent session transcripts
- **Technology**: Tantivy + SQLite hybrid index. p50 ~35ms; p95 ~120ms at 792k-message corpus (2026-04-11). Matched queries return in ≤35ms; zero-result queries require exhaustive index scan (~150–200ms) and grow with corpus size.
- **Use Case**: Agents can search past work to avoid duplicating solutions

## System Integration

### Full Workflow Example

1. **Agent Startup** (agent-runner.sh)
   ```
   1. Register with agent-mail (get unique identity)
   2. Check for wake trigger (/tmp/wake-agents.trigger)
   3. Query bv --robot-next for recommended bead
   4. Claim bead via br update --assignee $AGENT_NAME
   5. Launch claude with bead context
   ```

2. **Work Phase** (claude + hooks)
   ```
   1. session-start-hook: Verify registration, start monitors
   2. pre-edit-check-hook: Validate bead claimed before edits
   3. Edit files, write code
   4. post-bash-bead-track-hook: Log bash commands
   5. git commit -m "[BEAD-ID] message"
   ```

3. **Completion Phase** (br close)
   ```
   1. br close BEAD-ID
   2. post-bead-close-hook: Cleanup, suggestions
   3. next-bead.sh: Claim next bead, /clear, loop
   ```

### Multi-Agent Coordination

**Conflict Avoidance**:
- File reservations (advisory locks)
- Bead ownership (one agent per bead)
- Git-based CRDT for agent mail

**Coordination Patterns**:
- **Broadcast**: wake-agents.sh notifies all idle agents
- **Direct Message**: agent-mail send $RECIPIENT
- **Task Queue**: bv --robot-next prioritizes work
- **Swarm**: fleet-status.sh monitors agent health

### Data Persistence

**Git-Backed Storage**:
```
$PROJECT_ROOT/.beads/           # beads_rust data
  ├── issues.jsonl               # Current beads state
  ├── history/                   # Past states
  └── .git/                      # Version history

$PROJECT_ROOT/mcp_agent_mail/      # Agent mail server (Docker)
  ├── .env                        # Auth token, config
  ├── docker-compose.yml          # PostgreSQL + MCP server
  └── venv/                       # Python virtualenv
```

**Why Git?**:
- Atomic operations (git commit)
- Conflict-free merges (CRDT semantics)
- Audit trail (git log)
- Distributed (agents can work offline)

## Key Architectural Principles

### 1. Autonomy First
Agents operate independently without human intervention. All coordination happens via:
- Task queue (beads_rust)
- Async messaging (agent mail)
- Shared state (git)

### 2. Git as Truth
All persistent state lives in git:
- Tasks → .beads/ (beads_rust)
- Messages → PostgreSQL via mcp_agent_mail Docker service
- Code → project repo

Benefits: atomic operations, versioning, conflict resolution, audit trail.

### 3. Hook-Based Enforcement
Workflow rules enforced via hooks, not documentation:
- Can't edit without claimed bead
- Can't run bash without bead context
- Session lifecycle managed automatically

### 4. Observable System
Monitoring built-in at every layer:
- bead-stale-monitor: Detects abandoned work
- performance-tracker: Agent metrics
- reservation-metrics: Resource usage
- fleet-status: Multi-agent health

### 5. Graceful Degradation
System works even when components fail:
- Agent mail unavailable? Work solo
- bv down? Use br directly
- Hook bypass for emergencies

## Performance Characteristics

### Bottlenecks

1. **Git Operations**: Bottleneck for high-frequency operations
   - Mitigation: Batch commits, lazy sync

2. **File System**: Lock contention on shared files
   - Mitigation: Advisory file reservations

3. **Context Window**: LLM context fills over long sessions
   - Mitigation: /clear between beads, auto-compaction

### Scaling

**Horizontal Scaling**:
- Add more agents (tmux panes)
- Each agent independent
- Git handles concurrent access

**Limits**:
- ~10-20 agents per project (git merge overhead)
- ~1000 active beads (bv performance)
- ~100 messages/min (agent mail throughput)

## Security Model

### Trust Boundaries

1. **Agent Code Execution**: Agents run arbitrary code
   - Mitigation: Sandboxed environments, review hooks

2. **Git Backend**: Shared git repo is trusted
   - Mitigation: Pre-push hooks, code review

3. **Agent Identity**: Self-asserted identity
   - Mitigation: Log correlation, audit trail

### Safety Mechanisms

- **Pre-edit hook**: Prevents editing without bead
- **Pre-bash hook**: Validates commands before execution
- **Dry-run mode**: Preview before execution
- **Hook bypass**: Emergency override (logged)

## Extension Points

### Adding New Components

1. **New Hook**: Place in flywheel_tools/scripts/hooks/
2. **New Adapter**: Model adapters in flywheel_tools/scripts/adapters/
3. **New Monitor**: Monitoring scripts read from git backend

### Custom Workflows

Override configuration:
```bash
export AGENT_RUNNER_IDLE_SLEEP=30
export BEAD_STALE_THRESHOLD=900
export MAIL_PROJECT_KEY=/custom/path
```

### Integration with Other Systems

- **CI/CD**: Agents can trigger via bash hooks
- **Notifications**: Slack/email via agent-mail send
- **Analytics**: Export .beads/agent-activity.jsonl

## Failure Modes & Recovery

### Common Failures

1. **Agent Crash**
   - agent-runner.sh auto-restarts (max 5 times)
   - Tracking file preserved in /tmp/agent-bead-$NAME.txt

2. **Stale Bead**
   - bead-stale-monitor sends reminders after 15min
   - Agents can reassign via br update --assignee

3. **Git Conflict**
   - beads_rust uses CRDT-style merges
   - Worst case: Manual git conflict resolution

4. **Lost Agent Identity**
   - session-start-hook re-registers
   - Idempotent operations recover gracefully

### Recovery Procedures

**Stuck Agent**:
```bash
# 1. Check status
./scripts/fleet-status.sh

# 2. Force release bead
br update BEAD-ID --assignee ""

# 3. Restart agent
./scripts/agent-runner.sh
```

**Corrupted Beads DB**:
```bash
# 1. Restore from git
cd .beads && git checkout HEAD issues.jsonl

# 2. Re-sync
br sync --flush-only
```

## Future Architecture

### Planned Enhancements

1. **Distributed Beads**: Multi-repo task tracking
2. **Agent Swarm Optimizer**: Dynamic work allocation
3. **Cross-Project Search**: CASS across all projects
4. **Web Dashboard**: Real-time agent monitoring

### Research Areas

- LLM-based task prioritization
- Automatic bead decomposition
- Agent skill specialization
- Conflict-free merge strategies

## Related Documentation

- [Flywheel Tools README](../flywheel_tools/README.md) - Shell script infrastructure
- [beads_rust README](../beads_rust/README.md) - Task tracking backend
- [MCP Agent Mail README](../mcp_agent_mail/README.md) - Communication protocol
- [Feature Reference](feature-reference.md) - Comprehensive tool and script reference
- [Installation Guide](installation.md) - Setup and prerequisites
