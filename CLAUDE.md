# 7D Solutions Platform

## Rules for Agents

### 1. Beads (MANDATORY)
All work must be tracked with a bead. Edits are blocked until you have one.
```bash
./scripts/bv-claim.sh                          # Claim recommended bead
./scripts/br-start-work.sh "Your task title"   # Or create new
```
Never bypass or disable hooks. If blocked, create a bead first.

### 2. Git — mail the orchestrator (see global rules)
```bash
./scripts/agent-mail-helper.sh send <orchestrator> "bd-xxx done" "Files changed: X, Y, Z. What I did: ..."
br close bd-xxx
./scripts/bv-claim.sh
```

### 3. Cargo (MANDATORY)
Never call `cargo` directly. Use the slot system:
```bash
./scripts/cargo-slot.sh build -p inventory-rs
./scripts/cargo-slot.sh test -p inventory-rs
./scripts/cargo-slot.sh test --workspace
```

### 4. Docker
Do NOT run docker commands (hook blocks it). The cross-watcher handles compilation and container restarts automatically on commits.

### 5. Mail
```bash
./scripts/agent-mail-helper.sh whoami    # Check identity
./scripts/agent-mail-helper.sh inbox     # Check messages
```

## Standards

**Versioning:** Proven modules (>= 1.0.0) require version bumps. PATCH for fixes, MINOR for features, MAJOR for breaking. Add REVISIONS.md entry. See [docs/VERSIONING.md](./docs/VERSIONING.md).

**Frontend:** This is a backend-only repo. Verticals build frontends separately.
