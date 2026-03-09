# 7D Solutions Platform

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

## Module Versioning (MANDATORY)

**Full standard:** See [docs/VERSIONING.md](./docs/VERSIONING.md) for the complete system.

**Quick rules:**
1. If the module's version is >= `1.0.0`, it is a **proven module**. Extra rules apply.
2. Proven module changes require a version bump — PATCH for fixes, MINOR for features, MAJOR for breaking.
3. Add a revision entry in the module's `REVISIONS.md` for every version bump.
4. Version bump, revision entry, and code change go in the same commit.
5. Breaking changes (MAJOR): note migration path, mail the orchestrator.
6. Unproven modules (v0.x.x): no version bumps or revision entries required.

## Native Development (MANDATORY)

All Rust compilation happens **natively on the host**, not inside Docker containers. Agents never interact with Docker.

**Build and test:**
```bash
./scripts/cargo-slot.sh build -p inventory-rs   # Build a service
./scripts/cargo-slot.sh test -p inventory-rs    # Test a service
./scripts/cargo-slot.sh test --workspace         # Test everything
```

**What Docker does:** Infrastructure (Postgres, NATS) and idle services run in containers. Agents don't manage them.

**What agents must NOT do:**
- Run `docker` commands (the hook will block you)
- Modify Dockerfiles or docker-compose files
- Start, stop, or restart containers

**For the developer (not agents):**
```bash
scripts/dev-watch.sh                # Start stack with compose watch (auto-rebuild)
scripts/dev-native.sh inventory     # Stop container, run natively with cargo-watch
scripts/dev-native.sh --list        # Show all services
```

## Frontend

This is a backend-only platform repo. Verticals build their own frontends in separate repos.

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
