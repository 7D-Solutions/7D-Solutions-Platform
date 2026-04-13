# AgentCore Installation Guide

**Target Audience**: Operators setting up AgentCore for the first time
**Project Location**: `~/Projects/AgentCore`

---

## Prerequisites

```bash
rustc --version   # 1.70+  (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh)
python3 --version # 3.8+
go version        # 1.18+  (brew install go / https://go.dev/dl/)
tmux -V           # 2.0+   (brew install tmux / sudo apt install tmux)
git --version     # 2.0+
```

---

## Installation Sequence

Execute in order. Each must succeed before proceeding.

### 1. NTM (Named Tmux Manager)

Tmux session orchestration for multi-agent coordination. Go binary.

```bash
cd ~/Projects/AgentCore/ntm
./install.sh --easy-mode
ntm --version          # verify
ntm deps -v            # check dependencies
```

### 2. beads_rust (br)

Git-based task management. Rust.

```bash
cd ~/Projects/AgentCore/beads_rust
cargo build --release
cargo install --path .
br --version           # verify
```

Troubleshooting: `source ~/.cargo/env` if cargo not found. `cargo clean && cargo build --release` if build fails.

### 3. Beads Viewer (bv)

Terminal UI for task visualization. Go.

```bash
cd ~/Projects/AgentCore/beads_viewer
./install.sh
bv --version           # verify
```

### 4. MCP Agent Mail

Multi-agent communication via MCP. Python FastAPI.

```bash
cd ~/Projects/AgentCore/mcp_agent_mail
./install.sh
source venv/bin/activate
python -m mcp_agent_mail.server --help   # verify
deactivate
```

### 5. CASS (Coding Agent Session Search)

Semantic search for coding sessions. p50 ~35ms; p95 ~120ms at 792k-message corpus (zero-result queries exhaust the index). Python.

```bash
cd ~/Projects/AgentCore/coding_agent_session_search
./install.sh
```

### 6. fsfs (FrankenSearch)

Fast semantic + keyword search over the codebase. Rust.

```bash
curl -fsSL https://raw.githubusercontent.com/Dicklesworthstone/frankensearch/main/install.sh | bash -s -- --easy-mode
fsfs --version
fsfs doctor            # health check
```

### 7. UBS (Ultimate Bug Scanner)

Multi-language static analysis. Python.

```bash
cd ~/Projects/AgentCore/ultimate_bug_scanner
./install.sh
ubs --version || ubs --help   # verify
```

---

## Post-Installation Setup

### Start MCP Agent Mail Server

```bash
cd ~/Projects/AgentCore/mcp_agent_mail
source venv/bin/activate
python -m mcp_agent_mail.server
# Server runs on http://localhost:8765 by default
```

### Create Multi-Agent Session

```bash
ntm spawn test-project --cc=2 --cod=1
ntm send test-project --cc "Hello! List the AgentCore components."
ntm list
tmux attach -t test-project
```

### Initialize Beads in a Project

```bash
cd ~/Projects/my-project
br init
br new "First task"
bv
```

---

## Integration Test

```bash
#!/bin/bash
set -e

echo "=== AgentCore Integration Test ==="

cd ~/Projects/AgentCore/mcp_agent_mail
source venv/bin/activate
python -m mcp_agent_mail.server &
MCP_PID=$!
sleep 2
echo "✓ MCP Agent Mail server started"

TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"
br init
br new "Integration test task"
echo "✓ Beads initialized"

bv --version >/dev/null 2>&1 && echo "✓ Beads Viewer available"

ntm spawn integration-test --cc=1
echo "✓ NTM session created"

ntm send integration-test --cc "echo 'Integration test successful'"
echo "✓ Prompt sent"

cd ~/Projects/AgentCore
ubs scan --quick ultimate_bug_scanner/ >/dev/null 2>&1 && echo "✓ UBS scan completed"

kill $MCP_PID 2>/dev/null || true
tmux kill-session -t integration-test 2>/dev/null || true
rm -rf "$TEST_DIR"

echo "=== All components verified! ==="
```

---

## Common Issues

| Issue | Solution |
|-------|----------|
| `cargo: command not found` | `source ~/.cargo/env` |
| Python venv fails | `rm -rf venv && python3 -m venv venv && source venv/bin/activate && pip install -r requirements.txt` |
| Go build `module not found` | `go mod tidy && go get -u && go build` |
| NTM `tmux not found` | `brew install tmux` (macOS) / `sudo apt install tmux` (Linux) |
| MCP server port in use | `lsof -ti:8765 \| xargs kill -9` |

---

## Architecture Overview

```
AgentCore/
├── ntm/                           # Tmux session orchestration
├── beads_rust/                    # Task management (br)
├── beads_viewer/                  # Task visualization (bv)
├── mcp_agent_mail/                # Agent communication
├── coding_agent_session_search/   # CASS semantic search
├── ultimate_bug_scanner/          # UBS static analysis
├── flywheel_tools/                # Agent flywheel scripts
│   ├── scripts/core/              # Runner, mail, wake, broadcast
│   ├── scripts/beads/             # Bead lifecycle tools
│   ├── scripts/fleet/             # Fleet management
│   ├── scripts/monitoring/        # Metrics & observability
│   ├── scripts/retro/             # Retrospective & learning
│   ├── scripts/terminal/          # Tmux pane management
│   ├── scripts/adapters/          # Non-Claude agent wrappers
│   └── scripts/dev/               # Dev & debug utilities
├── config/                        # supervisord, hooks
└── docs/                          # Extended documentation
```

---

## Component Versions

| Component | Version | Language | Binary |
|-----------|---------|----------|--------|
| br (beads_rust) | 0.1.13 | Rust | `~/.cargo/bin/br` |
| bv (beads_viewer) | v0.14.4 | Go | `~/go/bin/bv` |
| ntm | v1.7.0 | Go | `~/.local/bin/ntm` |
| mcp_agent_mail | 0.3.0 | Python | FastAPI on port 8765 |
| cass | — | Python | `~/.local/bin/cass` |
| fsfs | — | Rust | `~/.local/bin/fsfs` |
| ubs | 0.0.0 | Python | `~/.local/bin/ubs` |
