# Cross-Compile Dev Setup

How to set up the local cross-compilation workflow so you can iterate on platform
services without a full rebuild cycle inside Docker.

---

## Overview

Each service runs as a pre-compiled Linux binary inside a thin Docker container.
You cross-compile on macOS, mount the binary into the container, and the
in-container binary watcher detects the new file and restarts the service
automatically — no `docker restart` needed.

```
macOS (cargo build) → target/aarch64-unknown-linux-musl/debug/<binary>
                           ↓ bind-mount (read-only)
                   Docker container (7d-runtime)
                           ↓ watches checksum
                   supervisord restarts service
```

---

## Prerequisites

- Docker Desktop (any recent version)
- Rust with the `aarch64-unknown-linux-musl` target installed:

  ```bash
  rustup target add aarch64-unknown-linux-musl
  ```

- `cross` or `cargo-zigbuild` (or the project's `cargo-slot.sh` wrapper):

  ```bash
  cargo install cross
  ```

---

## Step 1 — Build the 7d-runtime image (once per workstation)

This image contains supervisord, the binary watcher, and the entrypoint script.
It has no Rust toolchain and is only ~120 MB.

```bash
docker build -t 7d-runtime -f infra/Dockerfile.runtime .
```

Run this from the project root. The build context is the whole repo root so
that `COPY infra/...` paths resolve correctly.

**Rebuild the image only when** `infra/supervisord.conf`, `infra/dev-entrypoint.sh`,
or `infra/watch-binary.sh` change. Ordinary service code changes never require
an image rebuild.

---

## Step 2 — Cross-compile a service binary

Use the project's cargo-slot wrapper (required — do not call `cargo` directly):

```bash
./scripts/cargo-slot.sh build -p ar-rs --target aarch64-unknown-linux-musl
```

The compiled binary lands at:

```
target/aarch64-unknown-linux-musl/debug/ar-rs
```

---

## Step 3 — Start services in cross-compile mode

```bash
docker compose -f docker-compose.services.yml -f docker-compose.cross.yml up -d
```

The cross overlay (`docker-compose.cross.yml`) overrides each service's image
to `7d-runtime` and bind-mounts the cross-compiled binary. The `SERVICE_BINARY`
environment variable tells the entrypoint which binary to run.

To bring up a single service:

```bash
docker compose -f docker-compose.services.yml -f docker-compose.cross.yml up -d ar
```

---

## Step 4 — Iterate

Recompile the binary:

```bash
./scripts/cargo-slot.sh build -p ar-rs --target aarch64-unknown-linux-musl
```

The in-container watcher polls the binary checksum every 3 seconds. Once the
new binary is stable (two consecutive identical checksums), it validates the ELF
header and restarts the service via supervisord. Restart happens within ~5 seconds
of a stable write.

You do not need to run `docker restart` or `docker compose restart`.

---

## How the watcher works

`infra/watch-binary.sh` runs as a supervisord-managed process inside every
`7d-runtime` container. It:

1. Polls `$SERVICE_BINARY` checksum every 3 seconds.
2. Waits for two consecutive identical checksums (write-stability gate).
3. Validates the ELF magic bytes before loading.
4. Issues `supervisorctl stop service && supervisorctl start service`.
5. Confirms the service reached `RUNNING` state.

If the service crashes, the watcher's health-check loop (every 30 seconds)
detects `FATAL`/`STOPPED` state and attempts recovery.

---

## Troubleshooting

**Container exits immediately**

`SERVICE_BINARY` is not set or the path is wrong. Check the environment block
in `docker-compose.cross.yml` for the service.

**Binary change not detected**

The bind-mount may have a 1–3 second virtiofs lag on Docker-for-Mac. The
stability check handles this — wait up to 10 seconds after a compile finishes.

**Service keeps crashing after restart**

The binary compiled but the service is misconfigured or missing env vars.
Check logs: `docker compose logs -f <service>`.

**Need to rebuild the image**

```bash
docker image rm 7d-runtime
docker build -t 7d-runtime -f infra/Dockerfile.runtime .
```
