# Rust Service Container Spec
**Version 2026-04 · AgentCore**

Canonical specification for how a Rust service runs in a Docker container during development. The four projects in scope — 7D-Solutions Platform, Fireproof-ERP, TrashTech, RanchOrbit — all use this template. AgentCore hosts the canonical template and the runtime image but runs no Rust services of its own. A service is either spec-conformant or it is wrong. Non-Rust services (Node, Next.js frontends) and third-party containers (postgres, nats, redis, minio, nginx) are out of scope and run their own way.

This spec does not redesign the dev loop. The existing pattern — cross-compile on the host via `cargo-slot.sh`, volume-mount the resulting binary into a supervisord-managed container, in-container watcher polls for changes and restarts the service — already works. The spec's job is to make every Rust project's copy of that pattern identical, lock it down so agents cannot drift it, and document it so agents know how to work with it.

## 1. The runtime image

Exactly one Dockerfile, at `infra/rust-dev-runtime/Dockerfile` in AgentCore. No dev-loop Rust-service Dockerfile exists in any other project. Built once per spec version and tagged `flywheel/rust-dev-runtime:2026-04`. The tag is namespaced under `flywheel/` to avoid collision with unrelated images on the host.

```dockerfile
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 curl supervisor \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd -r appuser -g 1001 && \
    useradd -r -u 1001 -g appuser -d /app -s /sbin/nologin appuser && \
    mkdir -p /app && chown appuser:appuser /app

COPY supervisord-dev.conf /etc/supervisor/conf.d/supervisord.conf
COPY watch-binary.sh /usr/local/bin/watch-binary.sh
COPY dev-entrypoint.sh /usr/local/bin/dev-entrypoint.sh
RUN chmod +x /usr/local/bin/watch-binary.sh /usr/local/bin/dev-entrypoint.sh

WORKDIR /app
CMD ["/usr/local/bin/dev-entrypoint.sh"]
```

Decisions:

- **`debian:bookworm-slim`** — small, stable, has `apt`. Distroless would be smaller but cannot run supervisord.
- **Fixed UID 1001** — prevents permission drift across machines and container recreations. Does not by itself guarantee tar-extract success; that is verified at runtime by the behavioral conformance checks (Section 9).
- **supervisor + curl** — supervisor runs the process and the watcher, curl is used by the in-container healthcheck. No in-container log rotation: supervisord and both programs log to `/dev/fd/1` and `/dev/fd/2`, which Docker's log driver captures and rotates externally. Nothing writes to disk under `/var/log/` inside the container.
- **No `ENTRYPOINT` directive — only `CMD`.** The Dockerfile forbids compose files from setting either `command:` or `entrypoint:` (see Section 5).
- **Scripts are baked into the image at build time.** They live only in `infra/rust-dev-runtime/` in AgentCore. No other project has a copy of any of them.

**Dev-loop Dockerfiles deleted during conformance.** The following files exist today and are removed as part of per-project conformance work. They are named explicitly so there is no ambiguity about what is in scope for deletion.

- `7D-Solutions Platform/infra/Dockerfile.runtime`
- `Fireproof-ERP/deploy/Dockerfile.backend-dev`
- `TrashTech/modules/trashtech/Dockerfile.backend-dev`
- `RanchOrbit/deploy/Dockerfile.backend-dev`
- All per-module `Dockerfile.workspace` files in `7D-Solutions Platform/modules/*/deploy/` (approximately thirty files).

**Production Dockerfiles are explicitly not touched.** Any Dockerfile that is not on the deletion list above stays. This includes `RanchOrbit/deploy/Dockerfile.auth`, `RanchOrbit/deploy/Dockerfile.control-plane`, `RanchOrbit/deploy/Dockerfile.backend-runtime`, `RanchOrbit/deploy/Dockerfile.frontend`, and any Dockerfile used by production compose stacks in any project.

## 2. The binary

- **Path inside the container:** `/app/service`. Always. No per-service variation.
- **Path on the host:** `target/aarch64-unknown-linux-musl/debug/<binary-name>`. Cargo's default output for this target triple. Projects may not override `CARGO_TARGET_DIR` globally; if a project does so today, it is undone during conformance.
- **Volume mount:** `./target/aarch64-unknown-linux-musl/debug/<binary-name>:/app/service:ro`, read-only from inside the container.
- **Binary name resolution:** the binary name matches the cargo package name if no `[[bin]]` section exists in the crate's `Cargo.toml`. If a `[[bin]]` section defines an alternate name, the binary name is the `name` field from that section. Multi-binary crates have one compose service entry per binary.
- **Cross-compile target:** `aarch64-unknown-linux-musl` for every project. This matches the current working setup; the host does the cross-compile, the container runs the musl binary.
- **Existing cargo package names stay as they are.** Names with `-rs` suffixes in 7D-Solutions Platform are preserved. The spec does not require cargo renames.

## 3. Process supervision inside the container

`dev-entrypoint.sh` starts supervisord as PID 1. Supervisord manages two programs:

- **`service`** — runs `/app/service` as user `appuser`. Auto-restart on unexpected exit. stdout and stderr go to `/dev/fd/1` and `/dev/fd/2` so Docker's log driver captures them.
- **`watcher`** — runs `watch-binary.sh`. Auto-restart on unexpected exit. stdout and stderr go to `/dev/fd/1` and `/dev/fd/2`, also captured by Docker's log driver.

The supervisord program name for the Rust process is always `service`, regardless of what the binary or compose service is called. This lets the watcher script reference a fixed program name.

**Watcher algorithm, exact:**

1. Compute MD5 of `/app/service` and store as `last`.
2. Sleep 3 seconds.
3. Compute MD5 as `current`. If `current == last`, loop back to step 2 (nothing changed).
4. If `current != last`, store `current` as a candidate.
5. Sleep 3 seconds and recompute. If the hash equals the candidate, proceed. If it differs, go back to step 4 with the newest hash (the binary is still being written).
6. Validate ELF magic — read the first 4 bytes and confirm they are `7f 45 4c 46`. If invalid, log a WARNING and return to step 1. The old binary keeps running.
7. Call `supervisorctl -s unix:///tmp/supervisor.sock restart service`.
8. Sleep 4 seconds.
9. Check `supervisorctl status service`. If not RUNNING, log a WARNING. Continue polling.
10. Store `current` as `last` and loop.

**Why MD5 and polling rather than inotify.** Docker for Mac bind mounts do not reliably propagate `mtime` or inode events across the virtiofs boundary. Polling the file content hash is the portable approach that actually works on both macOS and Linux hosts. MD5 is chosen because it is fast and cryptographic strength is not relevant here — we only need to detect change.

**ELF magic check is necessary but not sufficient.** It catches the mid-write case and corrupt-download cases. It does not prove the binary is the right architecture or that it will actually exec. The post-restart behavioral check (Section 9) is where real verification happens.

**Watcher failure modes and their signatures:**

- **Watcher process crashes.** Supervisord auto-restarts it. No action needed.
- **Watcher wedges silently (not polling, but alive).** Not automatically detected. Symptom: host binary hash differs from in-container hash for more than ten seconds under normal conditions. The runbook's decision tree catches this and tells the agent to mail the orchestrator.
- **Binary is corrupt and repeatedly fails ELF check.** Watcher logs WARNING and keeps polling. Service continues on the old binary. Symptom: repeated WARNING lines in the watcher's stderr, visible via `docker compose logs <service>`.
- **Binary change occurs between polls.** Not possible to miss: the polling interval is 3 seconds and the hash is computed directly from file content, not from metadata.

## 4. Health

**For HTTP services** — any Rust service that exposes a port in its compose entry:

- One endpoint: `GET /api/health` on the service's declared HTTP port.
- Returns 200 once the service is fully started, connected to its hard dependencies, and ready to handle requests. Returns non-200 during startup or when a hard dependency is unreachable.
- Degraded mode — where optional dependencies are down but the service can still answer meaningful requests — returns 200. Where the service cannot serve any of its core functions, it returns non-200.
- Docker healthcheck uses `curl -f http://localhost:<port>/api/health`.
- Services currently using `/healthz`, `/health`, or other paths get a one-line route rename as part of conformance.

**For non-HTTP services** — any Rust service that does not expose a port (workers, NATS consumers, background daemons, scheduled tasks):

- No `/api/health` endpoint required.
- No compose-level `healthcheck:` block.
- Liveness is determined by `supervisorctl status service` reporting RUNNING.
- A non-HTTP service may expose its own readiness signal (NATS message, file, etc.) if it wants, but the spec does not require it.

## 5. Compose entry shape

Every Rust service's compose block follows this shape, differing only in the per-service fields:

```yaml
<service-name>:
  image: flywheel/rust-dev-runtime:2026-04
  container_name: <project-prefix>-<service-name>   # SHOULD, not MUST — see note below
  environment:
    DATABASE_URL: ...
    PORT: <port>
    RUST_LOG: ${RUST_LOG:-info}
  ports:
    - "127.0.0.1:<port>:<port>"
  volumes:
    - ./target/aarch64-unknown-linux-musl/debug/<binary-name>:/app/service:ro
  healthcheck:
    test: ["CMD-SHELL", "curl -f http://localhost:<port>/api/health || exit 1"]
    interval: 10s
    timeout: 5s
    retries: 5
    start_period: 30s
  restart: unless-stopped
```

**Forbidden fields in any dev compose file, no exceptions:**

- **`build:`** — triggers image rebuild in the hot path and violates the OOM invariant from March 30.
- **`develop.watch`** — the host cross-watcher and in-container binary watcher handle reloads. Compose watch is not used.
- **`command:`** — the image's `CMD` is the supervisord entrypoint and must not be overridden.
- **`entrypoint:`** — same reason as `command:`. Both are forbidden to close the override hole.
- **Source code volume mounts** — source lives on the host, compiles to a binary, the binary is what is mounted. Do not mount source directories.

**Optional fields used as needed:** `networks`, `depends_on`, `labels`, `cap_drop`, `read_only`, `tmpfs`, additional read-only `volumes:` entries for per-service config files (for example `module.toml`).

**When `read_only: true` is set**, the following tmpfs mounts are required so supervisord can write its socket and pid file:

```yaml
tmpfs:
  - /tmp
  - /var/run
```

Without these, supervisord cannot write its unix socket (`/var/run/supervisor.sock`) or pid file (`/var/run/supervisord.pid`), and the container will fail to start. No `/var/log` tmpfs is needed because nothing inside the container writes to disk — all logs go to stdout/stderr via `/dev/fd/1` and `/dev/fd/2`.

**`container_name` is SHOULD, not MUST.** If set, it follows `<project-prefix>-<service-name>` — for example `7d-ar`, `fireproof-erp`, `tt-server`, `ro-cattle-tracker`. If left unset, Docker auto-generates a unique name. Leaving it unset avoids container-name conflicts when a second worktree of the same project brings up the stack in parallel.

**Nested-workspace binary mount paths.** The `./target/aarch64-unknown-linux-musl/debug/<binary-name>` template in the compose block above is relative to the compose file's directory, and assumes the compose file sits alongside the cargo `target/` directory. For projects where the compose file is nested inside a module folder (for example `modules/trashtech/docker-compose.yml`) while the cargo target lives at the project root, the relative path uses the appropriate number of `../` segments: `../../target/aarch64-unknown-linux-musl/debug/<binary-name>:/app/service:ro`. The conformance linter accepts any sequence of `./` and `../` segments in front of `target/` as long as the rest of the shape matches.

**`depends_on` is required** for services with hard startup dependencies on other services. A service that requires a database must declare `depends_on: [postgres]` with `condition: service_healthy`. A service that fails gracefully without a dependency does not need to declare it.

**For non-HTTP services**, omit the `ports:` and `healthcheck:` blocks entirely. The compose entry is shorter and the conformance linter expects the absence.

## 6. Host wiring

Every project with Rust services has exactly these four things, configured identically in shape:

1. A `cross-watcher-<project-slug>` entry in AgentCore's `config/supervisord.conf`, running `scripts/dev-cross-supervised.sh` with the project root and the appropriate flags (`--workspace` for multi-crate projects, `--bin <name> --container <name>` for single-binary projects). TrashTech and RanchOrbit currently lack this and gain one during conformance.
2. A `scripts/cargo-slot.sh` symlink pointing at `flywheel_tools/scripts/core/cargo-slot.sh` in AgentCore. The symlink is absolute. If it is broken, the conformance linter catches it.
3. A `Cargo.toml` at the project root defining a workspace or a single crate. The workspace layout is the project's own choice; the spec does not dictate it beyond requiring a root `Cargo.toml`.
4. Compose files that use `flywheel/rust-dev-runtime:2026-04` as the image for every Rust service and follow the entry shape in Section 5.

The script `scripts/generate-supervisord-conf.sh` does not exist. It was a persistent source of drift and was deleted during conformance. `config/supervisord.conf` in AgentCore is hand-maintained. It is approximately 130 lines long.

## 7. The three restart paths

There are three sanctioned ways to get a running Rust service to pick up a code or config change. They are ordered by how agents should reach for them.

**1. Commit-driven, the primary path.** The agent commits source code. The host cross-watcher polls `git rev-parse HEAD` every 30 seconds and detects the new commit. It runs `cargo-slot.sh build --workspace` (or `cargo-slot.sh build -p <name>` in single-bin mode), and on success it calls `docker restart <container>` for each container whose code changed. The container cold-starts with the new binary mounted via volume. End-to-end latency is 30 seconds of poll plus cargo build time, typically three to five minutes for a full workspace rebuild.

**2. `cargo-slot`-direct, the fast local loop.** The agent runs `./scripts/cargo-slot.sh build -p <package>` directly, without committing. The binary lands in the target directory. The in-container `watch-binary.sh` detects the checksum change within six seconds (two three-second polls for stability), validates ELF magic, and calls `supervisorctl restart service`. The process restarts inside the same container without container recreation. Latency is cargo build time only. Use this when committing every change would be wasteful — tight iteration on a single service.

**3. Override, config reload only.** When the agent has edited a bind-mounted config file that the service only reads at startup, and no code change is needed, they run `AGENTCORE_WATCHER_OVERRIDE=1 docker restart <container>`. This is the one case where an agent directly invokes `docker restart` on a container. The override is logged to `logs/watcher-override.log` in AgentCore for audit. Latency is about two seconds.

**Who runs what, and why each path is safe from the command lockdown:**

- **The host cross-watcher process runs under AgentCore's supervisord, not under an agent session.** Its `docker restart` calls do not pass through the hook server because the hook only intercepts agent tool invocations. Restart path 1 is therefore not blocked by the agent command lockdown — the lockdown is for agent sessions, not for background supervisord-managed processes.
- **The in-container watcher runs inside the container.** It uses `supervisorctl`, not `docker restart`. It does not touch the host Docker socket at all. Restart path 2 is also not affected by the agent command lockdown.
- **The override path is the only one an agent invokes directly.** It requires the `AGENTCORE_WATCHER_OVERRIDE=1` prefix on the specific command, and the hook server's `handleCrossWatcherGuard` function detects and logs it.

**What agents cannot do, enforced by the hook server:**

- `docker compose up`, `down`, `build`, `restart`, `stop` — blocked by `handleCrossWatcherGuard` inside cross-watcher-registered projects. Override is available via `AGENTCORE_WATCHER_OVERRIDE=1` for legitimate exceptions, with audit logging.
- `docker build` and `docker buildx build` — blocked by `handleDockerGuard`'s destructive list.
- `docker restart <container>` without the override prefix — blocked by `handleCrossWatcherGuard`.
- `docker kill`, `docker rm`, `docker rmi`, `docker container stop/kill/restart/rm/prune`, `docker image rm`, `docker volume rm`, `docker volume prune`, `docker network rm`, `docker network prune`, `docker system prune` — already blocked in the existing implementation.
- `docker run`, `docker create` — already blocked.
- Edits to the dev-loop file blocklist — the files listed in Section 9 cannot be modified by any agent session.

The hook server implementation lives at `~/.claude/hooks/hook-server.mjs`. `handleDockerGuard` is around line 932 and `handleCrossWatcherGuard` is around line 843. Conformance work adds `docker build` and `docker buildx build` to the destructive list and extends the file edit blocklist to cover the dev-loop file names.

## 8. Out of scope

- **Node and TypeScript services** — Next.js frontends, Node backends. They use `develop.watch` with `action: rebuild`, which is the correct action for Node because rebuilding the image is how Node apps pick up source changes. This spec does not touch them.
- **Third-party containers** — postgres, nats, redis, minio, nginx. Off-the-shelf vendor images, not part of the Rust dev-loop pattern.
- **Production Dockerfiles and compose files.** This spec covers the dev loop only. Production deployments use separate Dockerfiles and compose files per project, which are not touched by conformance work. Any Dockerfile not on the deletion list in Section 1 stays.
- **Windows host support.** Spec assumes macOS or Linux host.
- **Multi-architecture dev builds.** Dev is `aarch64-unknown-linux-musl` only.
- **Non-Rust backends, if they exist.** If a project has a Go or Python backend, it is out of scope for this spec.

## 9. Conformance checks

A service is spec-conformant if it passes all of the static checks below. A project is spec-conformant if every Rust service in it is conformant AND the four host-wiring items from Section 6 are present. The linter at `scripts/lint-rust-container-spec.py` in AgentCore (replacing the old `lint-compose-watch.py`) runs the static checks; a small behavioral test harness runs the behavioral checks after `docker compose up -d`.

**Static checks, per compose file in each project:**

1. Image is `flywheel/rust-dev-runtime:<current-tag>`. No `build:`, no `develop.watch`, no `command:`, no `entrypoint:`.
2. Binary volume mount matches `./target/aarch64-unknown-linux-musl/debug/<name>:/app/service:ro`. The referenced cargo package or `[[bin]]` entry actually exists in the project's `Cargo.toml` tree.
3. For HTTP services (those with a `ports:` block): `healthcheck:` uses `curl -f http://localhost:<port>/api/health`.
4. For non-HTTP services: no `ports:` block and no `healthcheck:` block.
5. If `container_name` is set, it matches `<project-prefix>-<service-name>`.
6. No two compose services in the same stack share the same binary volume mount path.
7. If `read_only: true`, the required tmpfs mounts are declared.
8. No project-level environment sets `CARGO_TARGET_DIR`.

**Behavioral checks, per running container after `docker compose up -d`:**

1. `docker inspect <container> --format '{{.Config.Image}}'` returns `flywheel/rust-dev-runtime:<current-tag>`.
2. `docker exec <container> test -x /app/service` returns 0.
3. `docker exec <container> cat /proc/1/comm` returns `supervisord`.
4. `docker exec <container> supervisorctl status service` reports RUNNING.
5. `docker exec <container> supervisorctl status watcher` reports RUNNING.
6. For HTTP services: `docker exec <container> curl -fs http://localhost:<port>/api/health` returns 200.
7. `docker inspect <container> --format '{{range .Mounts}}{{if eq .Destination "/app/service"}}{{.RW}}{{end}}{{end}}'` returns `false` (the mount is read-only).
8. Host SHA-256 of `target/aarch64-unknown-linux-musl/debug/<name>` matches the SHA-256 returned by `docker exec <container> sha256sum /app/service`.
9. After triggering a binary change (either a commit on the project's branch or a direct `cargo-slot.sh build`), the in-container watcher restarts the service within 10 seconds and the new binary's SHA-256 is visible inside the container.
10. No compose service has a duplicate `container_name` with any other running container on the host.

**Dev-loop file blocklist.** The following file basenames cannot be edited by any agent session. The hook server enforces this via the existing file-path-based blocklist, extended to include these names:

- `Dockerfile.runtime`
- `Dockerfile.backend-dev`
- `Dockerfile.workspace`
- `watch-binary.sh`
- `dev-entrypoint.sh`
- `supervisord-dev.conf`
- `supervisord.conf`
- `generate-supervisord-conf.sh` (if still present)
- `dev-cross-supervised.sh`
- `cargo-slot.sh`
- `docker-health-poller.sh`
- Any file named `docker-compose*.yml` or `Dockerfile*` (already blocked by existing logic)

An agent can read any of these files. An agent cannot write to any of these files, in any project, ever, without the user explicitly flipping `.claude-hooks-bypass` for the session.

## 10. Versioning and rollback

**Versioning.** The spec has a version string at the top of this document (currently `2026-04`). The runtime image tag tracks it: `flywheel/rust-dev-runtime:2026-04`. When the spec changes:

1. The orchestrator updates this document and bumps the version.
2. The orchestrator updates the files in `infra/rust-dev-runtime/` to match the new spec.
3. The orchestrator, under `.claude-hooks-bypass` flipped by the user, runs `docker build -t flywheel/rust-dev-runtime:<new-version> infra/rust-dev-runtime/`.
4. The orchestrator updates every project's compose files to reference the new tag.
5. The orchestrator runs `docker compose up -d` in each project under bypass, project by project, to recreate containers on the new image.

Steps 3 through 5 are deliberate, orchestrator-driven operations executed with the user's explicit approval via the bypass mechanism. Agents never run these operations and the hook server blocks them in normal sessions.

**Rollback.** The previous image tag is retained in the local Docker image store for at least one spec version cycle. If a new spec version causes failures in production dev work:

1. The orchestrator reverts the tag references in each affected project's compose files to the previous version.
2. The orchestrator runs `docker compose up -d` under bypass to recreate containers against the previous image.
3. The current bad image tag is not deleted until a third spec version supersedes it, so the rollback can itself be rolled back if needed.

The local Docker image store keeps at least the last two spec versions at all times. Older tags may be pruned explicitly by the orchestrator when disk pressure is real.

**Nothing in this spec happens automatically.** Image builds, container recreations, tag bumps, and rollbacks are all deliberate operations performed by the orchestrator with user oversight. No agent session triggers any of them.

## 11. Adding a new Rust project

When a new Rust-backed project joins the workspace — a fifth spoke, a sixth, whatever — it conforms to this spec from day one. The checklist for onboarding is:

1. **Create the cargo workspace.** `Cargo.toml` at the project root, defining either a single crate or a workspace with named members. Standard layout, nothing spec-driven.
2. **Set up cross-compile.** Symlink `scripts/cargo-slot.sh` to `flywheel_tools/scripts/core/cargo-slot.sh` in AgentCore (absolute symlink). Confirm the binary cross-compiles to `target/aarch64-unknown-linux-musl/debug/<binary-name>` via `cargo-slot.sh build --workspace` or `cargo-slot.sh build -p <package>`.
3. **Write the compose file.** Use the Section 5 entry shape verbatim for each Rust service: `image: flywheel/rust-dev-runtime:<current-tag>`, no `build:`, no `develop.watch`, no `command:`, no `entrypoint:`. Volume-mount the binary to `/app/service`. Healthcheck on `/api/health` for HTTP services; omit `healthcheck:` entirely for non-HTTP services.
4. **Register a cross-watcher.** The orchestrator, under `.claude-hooks-bypass`, adds a `[program:cross-watcher-<project-slug>]` entry to AgentCore's `config/supervisord.conf` pointing at `scripts/dev-cross-supervised.sh` with the project root and appropriate flags. Reload supervisord for the new project to pick up.
5. **Run the conformance linter.** Execute `scripts/lint-rust-container-spec.py` against the new project's compose file and fix everything it flags before the first `docker compose up`. A spec-conformant project passes the linter with zero violations.
6. **First boot.** Bring up the stack with `docker compose up -d` under bypass. Run every behavioral check from Section 9 and confirm each one passes. If any check fails, the project is not conformant and the first-boot procedure does not end until every check is green.
7. **Link the runbook.** Add a one-line reference to `docs/dev-loop.md` in the new project's `CLAUDE.md` so humans browsing the repo know where the operational documentation lives. Agents already see the runbook via the global rules folder at `~/.claude/rules/dev-loop.md`, so no per-project wiring is needed for agent-side context.

A new project is onboarded when steps 1 through 7 are complete and the linter reports clean. If any step gets skipped, the project is not spec-conformant and the linter will flag it the next time it runs — there is no grandfather clause.

The same checklist applies when an existing project (like HuberPower, if and when it gains a Rust backend) adds its first Rust service. The onboarding is not tied to project age; it is tied to the first Rust service appearing.
