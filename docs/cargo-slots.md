# Cargo Build Slots

Cargo-slot lets multiple agents run `cargo build` concurrently without lock contention. Each build gets its own temporary target directory (`target-slot-N/`), and Docker containers mount binaries from a stable `target/` directory that never moves.

## How it works

```
Agent A ─┐                          ┌─ target-slot-1/ (temporary, nuked after build)
         ├─ cargo-slot.sh build ──► │
Agent B ─┘                          └─ target-slot-2/ (temporary, nuked after build)
                                         │
                                         ▼ (cross-compile only: copy final binaries)
                                      target/  (stable, real directory — Docker mounts here)
```

1. Agent calls `scripts/cargo-slot.sh build ...` instead of raw `cargo build`.
2. The script acquires a free slot (1 through N) by creating a lock directory in `/tmp/cargo-build-slots/`.
3. `CARGO_TARGET_DIR` is set to `target-slot-N/` and cargo runs.
4. **Cross-compile builds** (`--target aarch64-unknown-linux-musl`): on success, final binaries are copied atomically to `target/aarch64-unknown-linux-musl/debug/`. Docker compose mounts from this stable path.
5. The entire `target-slot-N/` directory is deleted after every build. No artifacts accumulate.

## Key design decisions

**`target/` is a real directory, not a symlink.** Previous design used a symlink (`target → target-slot-N`) that got flipped by concurrent agents. This caused race conditions, broken containers, and 100GB+ disk bloat. The current design copies only final binaries (~7GB total) to a stable directory.

**Slots are fully ephemeral.** After each build, the entire slot directory is nuked. This trades incremental build speed for zero maintenance. A full workspace rebuild takes longer, but there's no cleanup to fail and no disk bloat to accumulate.

**Self-cleaning at acquire time.** Before acquiring a slot, the script nukes any unlocked stale slot directories. No cron job needed.

## Usage

```bash
# Build (agents use this instead of raw cargo)
scripts/cargo-slot.sh build --target aarch64-unknown-linux-musl -p party --bin party
scripts/cargo-slot.sh test -p inventory-rs
scripts/cargo-slot.sh check --workspace

# Status — shows slot locks, sizes, Docker target dir
scripts/cargo-slot.sh --status

# Manual cleanup — nukes all slot dirs (preserves target/)
scripts/cargo-slot.sh --clean

# Pre-warm slots (requires CARGO_SLOT_WARM_CRATE env var)
scripts/cargo-slot.sh --warm
```

## Configuration

| Env var | Default | Description |
|---------|---------|-------------|
| `CARGO_SLOT_COUNT` | 3 | Number of concurrent build slots |
| `CARGO_BUILD_JOBS` | `num_cpus / SLOT_COUNT` | Parallel compiler jobs per slot. Auto-calculated to prevent OOM under concurrent builds. Override to tune. |
| `CARGO_SLOT_WARM_CRATE` | (unset) | Crate to build when warming slots |

## File layout

```
project/
├── target/                          # Stable. Docker mounts from here. NEVER delete.
│   └── aarch64-unknown-linux-musl/
│       └── debug/
│           ├── party               # Final cross-compiled binaries (~170MB each)
│           ├── inventory-rs
│           └── ...
├── target-slot-1/                   # Temporary. Created during build, nuked after.
├── target-slot-2/                   # Temporary.
├── target-slot-3/                   # Temporary.
└── scripts/
    └── cargo-slot.sh → flywheel_tools/scripts/core/cargo-slot.sh
```

## Troubleshooting

### Containers fail with exit code 127 after a build
The binary wasn't copied to `target/`. Check:
1. Was the build a cross-compile? (`--target aarch64-unknown-linux-musl`)
2. Did the build succeed? (Binaries only promote on exit code 0)
3. Run `scripts/cargo-slot.sh --status` to see the Docker target dir

### Disk space filling up
Should not happen with the current design. If it does:
1. `scripts/cargo-slot.sh --status` — are slots accumulating?
2. `scripts/cargo-slot.sh --clean` — manual cleanup
3. Check for builds running outside the slot system (raw `cargo build`)

### `target/` is a symlink (old design)
The script auto-migrates on the next build. It removes the symlink, creates a real directory, and copies binaries from the old symlink target. No manual action needed.

### Stale slot locks
If a build process was killed (SIGKILL, crash), its lock may persist. The next `cargo-slot.sh` invocation detects dead PIDs and reclaims stale locks automatically.

## How Docker compose connects

`docker-compose.cross.yml` volume-mounts binaries from `./target/`:

```yaml
volumes:
  - ./target/aarch64-unknown-linux-musl/debug/party:/usr/local/bin/party:ro
```

This path is stable. No compose changes are needed when slots are created or destroyed.

## Canonical location

The script lives at `flywheel_tools/scripts/core/cargo-slot.sh` in AgentCore. Spoke projects symlink to it:

```
spoke/scripts/cargo-slot.sh → ../flywheel_tools/scripts/core/cargo-slot.sh
spoke/flywheel_tools → /path/to/AgentCore/flywheel_tools
```

Changes to the canonical copy propagate to all spokes immediately.
