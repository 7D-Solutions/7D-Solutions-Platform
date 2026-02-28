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

## Frontend

This is a backend-only platform repo. Verticals build their own frontends in separate repos.
