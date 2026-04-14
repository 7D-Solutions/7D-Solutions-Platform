# Contributing

This repository uses `br` for work tracking and `./scripts/cargo-slot.sh` for every Rust build or test. Use the normal bead flow:

1. Find a bead with `br ready --json` or `br list --status in_progress`.
2. Claim it with `br update <id> --status in_progress`.
3. Make one focused change.
4. Verify it with `./scripts/cargo-slot.sh test -p <package>` or the smallest applicable `cargo-slot` command.
5. Commit with the bead prefix, for example `git commit -m "[bd-12345] fix the parser"`.
6. Close the bead with `br close <id> --reason "Implemented"`.
7. Run `br sync --flush-only` before ending the session.

## First PR Flow

On a machine that already has the prerequisites installed, this is the verified 15-minute path from clone to first pull request:

1. Clone the repo and change into it.
2. Run `./scripts/dev/up.sh`.
3. If it reports missing prerequisites, install exactly what it names and rerun the script.
4. Claim a small bead and make a narrow change.
5. Validate the change with `./scripts/cargo-slot.sh`.
6. Commit with the required `[bd-xxxxx]` prefix.
7. Close and sync the bead.
8. Push the branch and open a PR.

## Prerequisites

Install these before starting:

- Docker Desktop with Docker Compose v2
- `rustup`
- `aarch64-linux-musl-gcc` from `musl-cross`
- `cargo-watch`
- `python3`
- `curl`
- `br`

The doctor script reports missing items in the same order the setup flow needs them.

## Notes

- Use `./scripts/dev/wait-for-ready.sh` when you need to wait on a specific service or a subset of services.
- Use `./scripts/verify_health_endpoints.sh` after bring-up when you want the full HTTP health sweep.
- Do not use raw `cargo` for Rust builds or tests in this repo.
