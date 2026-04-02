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

All Rust compilation happens **natively on the host**. Agents never run Docker commands (the hook will block you).

**Build and test:**
```bash
./scripts/cargo-slot.sh build -p inventory-rs   # Build a service
./scripts/cargo-slot.sh test -p inventory-rs    # Test a service
./scripts/cargo-slot.sh test --workspace         # Test everything
```

## How Docker Works (READ THIS)

**The pipeline:** Agent writes code → git commit → supervisord cross-watcher detects (polls every 30s) → cross-compiles for Linux ARM64 via cargo-slot.sh → docker restart → container runs new binary.

**Binaries are volume-mounted, not baked into images.** `docker-compose.cross.yml` mounts each binary from `target/aarch64-unknown-linux-musl/debug/<name>` into the container. The `target` symlink points to whichever `target-slot-N` cargo-slot.sh last used.

**SDK-converted modules** (Party, Production, and future conversions) also need `module.toml` volume-mounted. These mounts are in `docker-compose.cross.yml`.

**What agents must do:**
- Write code and commit with `[bd-xxx]` prefix. The watcher handles the rest.
- If a new module needs env vars (e.g., `BUS_TYPE`, `NATS_URL`), edit `docker-compose.services.yml` and commit. GentleCliff handles container recreation.

**What agents must NOT do:**
- Run `docker` commands (hook blocks it)
- Modify Dockerfiles
- Start, stop, or restart containers

**Who manages Docker:**
- **Supervisord cross-watcher:** Detects commits, cross-compiles, restarts containers
- **GentleCliff:** Handles container recreation when volume mounts or compose config changes
- **Developer:** `scripts/dev-watch.sh` (compose watch), `scripts/dev-native.sh` (run natively)

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

## Git Commit Protocol (MANDATORY)

**Agents do NOT commit.** The orchestrator (BrightHill) handles all git operations.

**What agents must NOT do:**
- `git add`, `git commit`, `git stash`, `git reset`, or any git write operation
- Stage files, create commits, or push

**What agents DO:**
1. Write code for your bead
2. When done, mail BrightHill with: bead ID, list of files changed, what you did
3. BrightHill reviews, stages, and commits in clean batches
4. You can still run `git diff` and `git status` to check your work

**Why:** Multiple agents sharing one working tree causes cross-contamination when staging and committing simultaneously. Pre-commit hooks scan all staged files, not just yours. Stashes collide. Centralized commits prevent all of this.

**End of work:** Do NOT close your bead — mail BrightHill and wait for confirmation.
```bash
./scripts/agent-mail-helper.sh send BrightHill "bd-xxx done" "Files changed: X, Y, Z. What I did: ..."
```
