# AGENTS.md - AgentCore Setup Instructions

**Target Audience**: AI coding agents (Claude Code, Codex, Gemini)
**Purpose**: Complete installation and setup of AgentCore multi-agent system
**Project Location**: `~/Projects/AgentCore`

---

## Prerequisites Check

Before starting installation, verify these are installed:

```bash
# Check Rust
rustc --version  # Should be 1.70+
# If missing: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Check Python
python3 --version  # Should be 3.8+

# Check Go
go version  # Should be 1.18+
# If missing (macOS): brew install go
# If missing (Linux): Download from https://go.dev/dl/

# Check tmux
tmux -V  # Should be 2.0+
# If missing (macOS): brew install tmux
# If missing (Linux): sudo apt install tmux

# Check Git
git --version  # Should be 2.0+
```

---

## Installation Sequence

Execute these commands in order. Each component must complete successfully before proceeding to the next.

### 1. NTM (Named Tmux Manager)

**Purpose**: Tmux session orchestration for multi-agent coordination
**Language**: Go (pre-built binary available)
**Install time**: ~30 seconds

```bash
cd ~/Projects/AgentCore/ntm
./install.sh --easy-mode
```

**Verify**:
```bash
ntm --version
ntm deps -v  # Check all dependencies
```

**Expected output**: Version number displayed (e.g., `ntm v1.4.1`)

---

### 2. beads_rust (br)

**Purpose**: Git-based task management system
**Language**: Rust
**Install time**: 5-10 minutes (compilation)

```bash
cd ~/Projects/AgentCore/beads_rust
cargo build --release
cargo install --path .
```

**Verify**:
```bash
br --version
which br  # Should show path in ~/.cargo/bin/
```

**Expected output**: Version number and binary location

**Troubleshooting**:
- If cargo not found: `source ~/.cargo/env`
- If build fails: `cargo clean && cargo build --release`

---

### 3. Beads Viewer (bv)

**Purpose**: Terminal UI for task visualization and graph analysis
**Language**: Go
**Install time**: 2-5 minutes

```bash
cd ~/Projects/AgentCore/beads_viewer
./install.sh
```

**Verify**:
```bash
bv --version
bv --help
```

**Expected output**: Help text with available commands

**Troubleshooting**:
- If Go modules error: `go mod tidy && go build`
- Check GOPATH: `echo $GOPATH`

---

### 4. MCP Agent Mail

**Purpose**: Multi-agent communication via Model Context Protocol
**Language**: Python (FastAPI server)
**Install time**: 1-2 minutes

```bash
cd ~/Projects/AgentCore/mcp_agent_mail
./install.sh
```

**Verify**:
```bash
cd ~/Projects/AgentCore/mcp_agent_mail
source venv/bin/activate
python -m mcp_agent_mail.server --help
deactivate
```

**Expected output**: Server help text with available options

**Configuration**:
```bash
cd ~/Projects/AgentCore/mcp_agent_mail
# Copy example config if it exists
if [ -f config.example.json ]; then
    cp config.example.json config.json
fi
```

---

### 5. CASS (Coding Agent Session Search)

**Purpose**: Semantic search for coding sessions (<60ms performance)
**Language**: Python (RAG-based)
**Install time**: 2-3 minutes

```bash
cd ~/Projects/AgentCore/coding_agent_session_search
./install.sh
```

**Verify**:
```bash
cd ~/Projects/AgentCore/coding_agent_session_search
source venv/bin/activate
python -m cass --help 2>/dev/null || echo "Install successful, awaiting first run"
deactivate
```

**Note**: CASS may need first-run initialization for embedding models

---

### 6. UBS (Ultimate Bug Scanner)

**Purpose**: Multi-language static analysis for automated bug detection
**Language**: Python
**Install time**: 1-2 minutes

```bash
cd ~/Projects/AgentCore/ultimate_bug_scanner
./install.sh
```

**Verify**:
```bash
ubs --version || ubs --help
```

**Expected output**: Version or help text

**Usage Example**:
```bash
# Scan current directory
ubs scan .

# Scan with specific languages
ubs scan --lang python,rust,go .
```

---

## Post-Installation Setup

### Initialize a Test Project

```bash
# Create test directory
mkdir -p ~/Projects/test-agent-project
cd ~/Projects/test-agent-project

# Initialize beads
br init
br new "Setup AgentCore integration"

# View with Beads Viewer
bv
```

### Start MCP Agent Mail Server

```bash
# Terminal 1: Start server
cd ~/Projects/AgentCore/mcp_agent_mail
source venv/bin/activate
python -m mcp_agent_mail.server
# Server runs on http://localhost:8000 by default
```

### Create Multi-Agent Session with NTM

```bash
# Terminal 2: Create session
ntm spawn test-project --cc=2 --cod=1

# This creates:
# - 2 Claude Code agent panes
# - 1 Codex agent pane
# - All in a single tmux session named "test-project"

# Send prompt to all Claude agents
ntm send test-project --cc "Hello! List the AgentCore components."

# View all sessions
ntm list

# Attach to session
tmux attach -t test-project
```

---

## Integration Test

Run this complete integration test to verify all components work together:

```bash
#!/bin/bash
set -e

echo "=== AgentCore Integration Test ==="

# 1. Start MCP server in background
cd ~/Projects/AgentCore/mcp_agent_mail
source venv/bin/activate
python -m mcp_agent_mail.server &
MCP_PID=$!
sleep 2
echo "✓ MCP Agent Mail server started (PID: $MCP_PID)"

# 2. Create test project with beads
TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"
br init
br new "Integration test task"
echo "✓ Beads initialized in $TEST_DIR"

# 3. View with Beads Viewer (non-interactive check)
bv --version >/dev/null 2>&1 && echo "✓ Beads Viewer available"

# 4. Create NTM session
ntm spawn integration-test --cc=1
echo "✓ NTM session created"

# 5. Send test prompt
ntm send integration-test --cc "echo 'Integration test successful'"
echo "✓ Prompt sent to agents"

# 6. Run UBS scan
cd ~/Projects/AgentCore
ubs scan --quick ultimate_bug_scanner/ >/dev/null 2>&1 && echo "✓ UBS scan completed"

# Cleanup
kill $MCP_PID 2>/dev/null || true
tmux kill-session -t integration-test 2>/dev/null || true
rm -rf "$TEST_DIR"

echo "=== All components verified! ==="
```

Save this as `~/Projects/AgentCore/test-integration.sh` and run:
```bash
chmod +x ~/Projects/AgentCore/test-integration.sh
./test-integration.sh
```

---

## Common Issues and Solutions

### Issue: "cargo: command not found"
**Solution**:
```bash
source ~/.cargo/env
# Or add to shell rc: echo 'source ~/.cargo/env' >> ~/.zshrc
```

### Issue: Python venv activation fails
**Solution**:
```bash
cd <component-dir>
rm -rf venv
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt
```

### Issue: Go build fails with "module not found"
**Solution**:
```bash
go mod tidy
go get -u
go build
```

### Issue: NTM says "tmux not found"
**Solution**:
```bash
# macOS
brew install tmux

# Linux
sudo apt update && sudo apt install tmux
```

### Issue: MCP server port already in use
**Solution**:
```bash
# Find and kill process on port 8000
lsof -ti:8000 | xargs kill -9

# Or change port in config.json
```

---

## Architecture Overview

```
AgentCore/
├── ntm/                           # Tmux session orchestration
│   └── install.sh                 # Pre-built binary installer
├── beads_rust/                    # Task management (br)
│   └── Cargo.toml                 # Rust project
├── beads_viewer/                  # Task visualization (bv)
│   └── go.mod                     # Go project
├── mcp_agent_mail/                # Agent communication
│   ├── install.sh                 # Python venv setup
│   └── src/mcp_agent_mail/        # FastAPI server
├── coding_agent_session_search/   # CASS semantic search
│   └── venv/                      # Python environment
└── ultimate_bug_scanner/          # UBS static analysis
    └── install.sh                 # Python installer
```

---

## Next Steps After Installation

1. **Configure MCP Agent Mail**: Edit `mcp_agent_mail/config.json` for custom ports/settings
2. **Initialize Beads**: Run `br init` in your project directories
3. **Create NTM Sessions**: Use `ntm spawn <project>` to create multi-agent sessions
4. **Run UBS Scans**: Scan codebases with `ubs scan .` to identify issues
5. **Use CASS**: Search past sessions with semantic queries

---

## Agent Workflow Pattern

```bash
# 1. Start MCP server (terminal 1)
cd ~/Projects/AgentCore/mcp_agent_mail && source venv/bin/activate
python -m mcp_agent_mail.server

# 2. Create/attach to session (terminal 2)
ntm spawn myproject --cc=2

# 3. Initialize beads in project
cd ~/Projects/myproject
br init

# 4. Send coordinated tasks
ntm send myproject --cc "Review codebase and create improvement beads"

# 5. Monitor with Beads Viewer
bv

# 6. Scan for issues
ubs scan .
```

---

---

## Disabling /clear Between Beads

By default, `next-bead.sh` sends `/clear` to reset context between bead cycles. To keep context (e.g. for debugging or reviewing agent work), disable it with either:

```bash
# Option 1: File flag (easy to toggle from another pane)
touch .no-clear       # disable /clear
rm .no-clear          # re-enable /clear

# Option 2: Environment variable (set before launching agent-runner)
AGENT_NO_CLEAR=1 ./scripts/agent-runner.sh
```

When `/clear` is skipped, `next-bead.sh` still claims the next bead and prints the prompt — it just doesn't interrupt the running session.

---

## Success Criteria

After installation, you should be able to:
- [x] Run `ntm --version` successfully
- [x] Run `br --version` successfully
- [x] Run `bv --version` successfully
- [x] Start MCP Agent Mail server
- [x] Create and view beads
- [x] Spawn multi-agent NTM sessions
- [x] Run UBS scans

---

## Getting Help

- NTM: `ntm --help` or `ntm tutorial`
- Beads: `br --help` or read `beads_rust/README.md`
- MCP Agent Mail: See `mcp_agent_mail/README.md`
- UBS: `ubs --help` or read `ultimate_bug_scanner/README.md`

---

**Last Updated**: 2026-02-10
**Project Status**: Fresh installation ready
**Target Agents**: Claude Code, OpenAI Codex, Google Gemini
