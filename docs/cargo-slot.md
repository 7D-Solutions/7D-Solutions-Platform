# cargo-slot.sh

Concurrent cargo build slot system. Agents use `scripts/cargo-slot.sh` instead of raw `cargo` to avoid build lock contention across parallel agents.

```bash
./scripts/cargo-slot.sh build -p inventory-rs
./scripts/cargo-slot.sh test -p inventory-rs
./scripts/cargo-slot.sh build --release
./scripts/cargo-slot.sh --status   # Show slot status
./scripts/cargo-slot.sh --warm     # Pre-warm all slots
./scripts/cargo-slot.sh --clean    # Remove all locks and slot dirs
```

## How It Works

The script maintains N independent build slots, each with its own `CARGO_TARGET_DIR` (`target-slot-1/`, `target-slot-2/`, etc.). When an agent runs a cargo command, it acquires the first free slot, runs cargo against that slot's target directory, then nukes the slot directory on exit to prevent disk bloat.

Cross-compiled binaries are promoted to the stable `target/` directory (mounted by Docker containers) after a successful build.

## Configuration

| Env Var | Default | Description |
|---------|---------|-------------|
| `CARGO_SLOT_COUNT` | 3 | Number of concurrent slots |
| `CARGO_SLOT_WARM_CRATE` | (unset) | Crate to build when warming slots; skips warm if unset |
| `CARGO_BUILD_JOBS` | auto | Compiler parallelism per slot; auto-divides cores across slots |

## Nested Workspaces (.cargo-slot config)

By default, cargo-slot.sh runs cargo from the project root — the directory the script resolves to after following symlinks. This works when `Cargo.toml` (the workspace manifest) lives at the project root.

Some projects keep their Rust workspace in a subdirectory. For example, a vertical like TrashTech might organize its repo as:

```
trashtech/
  .cargo-slot          # config file
  docker-compose.yml
  docs/
  modules/
    Cargo.toml         # workspace manifest lives here
    inventory-rs/
    shipping-rs/
```

In this layout, the project root is `trashtech/` but the Cargo workspace root is `trashtech/modules/`. Running `cargo build` from `trashtech/` would fail because there is no `Cargo.toml` at that level.

To handle this, create a `.cargo-slot` file at the project root:

```ini
# .cargo-slot
workspace_dir=modules
```

When cargo-slot.sh finds this file, it `cd`s into the specified subdirectory before invoking cargo. The path is relative to the project root.

### Rules

- The `workspace_dir` value must be a relative path (no leading `/`).
- The directory must contain a valid `Cargo.toml` workspace manifest.
- Slot target directories (`target-slot-N/`) and the stable `target/` directory still live at the project root, not inside the nested workspace.
- If `.cargo-slot` does not exist, cargo runs from the project root as usual. No config file needed for standard layouts.
