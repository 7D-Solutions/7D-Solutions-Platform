# Rust Container Standard

**Status:** Canonical  
**Established:** 2026-04-10 (bd-2rge6)  
**Applies to:** All Rust services running in Docker across the swarm (7D-Solutions Platform, Fireproof-ERP, RanchOrbit, TrashTech, and any future projects)

---

## 1. Canonical Dockerfile Template

Dev containers use a thin runtime image with supervisor managing the service process and a binary watcher. Rust binaries are cross-compiled on the host and volume-mounted — the Docker image never builds Rust.

```dockerfile
# Dev runtime container for <project-name>.
# Binary is cross-compiled on the host and volume-mounted.
# Supervisord manages the app process; the watcher detects binary
# changes (via checksum, not mtime — Docker for Mac doesn't propagate mtime)
# and triggers a graceful restart via supervisorctl.
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 curl supervisor \
    && rm -rf /var/lib/apt/lists/*

# Copy supervisord config and helper scripts from the project's deploy/ dir
COPY deploy/supervisord-dev.conf /etc/supervisor/conf.d/supervisord.conf
COPY deploy/watch-binary.sh /usr/local/bin/watch-binary.sh
COPY deploy/dev-entrypoint.sh /usr/local/bin/dev-entrypoint.sh
RUN chmod +x /usr/local/bin/watch-binary.sh /usr/local/bin/dev-entrypoint.sh

WORKDIR /app

CMD ["/usr/local/bin/dev-entrypoint.sh"]
```

**Required apt packages:** `ca-certificates libssl3 curl supervisor`  
**No Rust toolchain inside the image.** The binary is mounted at container start.

---

## 2. Canonical docker-compose Entry

```yaml
services:
  <service-name>:
    image: <project>-runtime      # built once from the Dockerfile above
    container_name: <project>-<service>
    working_dir: /app
    environment:
      SERVICE_BINARY: /usr/local/bin/<binary-name>
    volumes:
      - ./target/aarch64-unknown-linux-musl/debug/<binary-name>:/usr/local/bin/<binary-name>:ro
      - ./modules/<service>/module.toml:/app/module.toml:ro
      - ./modules/<service>/db/migrations:/app/db/migrations:ro
    ports:
      - "127.0.0.1:<host-port>:<container-port>"
    healthcheck:
      test: ["CMD-SHELL", "curl -f http://localhost:<container-port>/api/health || exit 1"]
      interval: 10s
      timeout: 5s
      retries: 5
      start_period: 30s
    labels:
      com.agentcore.project: <project-slug>
      com.agentcore.container-role: app
      com.agentcore.service: <service-name>
    networks:
      - <project>-network
    restart: unless-stopped
```

**Key constraints:**
- `SERVICE_BINARY` env var is mandatory — entrypoint fails fast if unset
- Binary volume is `:ro` — prevents accidental in-container modification
- `working_dir: /app` is required — module.toml and migrations are relative paths
- `start_period` minimum 30s — allows binary watcher to do first health check
- Port binding is `127.0.0.1:<host>:<container>` — never expose to `0.0.0.0` in dev

---

## 3. Health Endpoint Contract

Every Rust service **must** implement:

```
GET /api/health
→ HTTP 200
→ Response body: {"status":"ok"} (at minimum)
→ Response time: < 5 seconds after readiness
```

**Rules:**
- Path is exactly `/api/health` — no alternatives (`/healthz`, `/health`, `/ping`)
- Method is `GET`
- Status code is `200` when healthy, any non-200 when unhealthy
- Response body must be valid JSON with at minimum `{"status":"ok"}`
- The endpoint must respond within 5 seconds; the Docker healthcheck timeout is set to 5s

**Why `/api/health` and not `/healthz`:** Consistency across all services means a single curl pattern works everywhere. The `/api/` prefix is already the namespace for all service endpoints in this stack.

**RanchOrbit deviation:** Currently uses a TCP port check. Must be migrated to `GET /api/health` (tracked in bd-wivcb).  
**TrashTech deviation:** Currently uses `/healthz`. Must be migrated to `/api/health` (tracked in bd-wivcb).

---

## 4. Supervisord Watcher Entry Pattern (inside container)

```ini
[supervisord]
nodaemon=true
logfile=/var/log/supervisord.log
pidfile=/var/run/supervisord.pid
user=root

[unix_http_server]
file=/var/run/supervisor.sock

[supervisorctl]
serverurl=unix:///var/run/supervisor.sock

[rpcinterface:supervisor]
supervisor.rpcinterface_factory = supervisor.rpcinterface:make_main_rpcinterface

[program:<service-name>]
command=%(ENV_SERVICE_BINARY)s
directory=/app
autostart=true
autorestart=true
startretries=5
startsecs=3
stdout_logfile=/dev/fd/1
stdout_logfile_maxbytes=0
stderr_logfile=/dev/fd/2
stderr_logfile_maxbytes=0

[program:binary-watcher]
command=/usr/local/bin/watch-binary.sh
autostart=true
autorestart=true
startretries=999
stdout_logfile=/dev/fd/1
stdout_logfile_maxbytes=0
stderr_logfile=/dev/fd/2
stderr_logfile_maxbytes=0
```

**Notes:**
- Program name `<service-name>` must be the lowercase-dashed form of the service (e.g., `gl`, `fireproof-erp`, `ranchorbit-api`)
- `startretries=999` on `binary-watcher` is intentional — the watcher must survive indefinitely
- Log to `fd/1` and `fd/2` so Docker captures logs via `docker logs`

---

## 5. Container Labeling Convention

Every Rust app container **must** carry these labels:

```yaml
labels:
  com.agentcore.project: <project-slug>       # e.g., platform, fireproof-erp, ranchorbit
  com.agentcore.container-role: app           # always "app" for Rust services
  com.agentcore.service: <service-name>       # e.g., gl, payments, tt-server
```

Init containers (one-shot setup tasks that exit 0 when done) **must** carry:

```yaml
labels:
  com.agentcore.project: <project-slug>
  com.agentcore.container-role: init
```

**Why labels matter:** The docker-health-poller reads `com.agentcore.container-role` to distinguish init containers (expected to exit 0) from app containers (expected to stay running). Without labels, new init containers silently cause false-positive alerts.

**Project slugs** (lowercase-dashed basename of the project directory):
- `platform` — 7D-Solutions Platform
- `fireproof-erp` — Fireproof-ERP
- `ranchorbit` — RanchOrbit
- `trashtech` — TrashTech

**Note on existing `com.7dsolutions.*` labels:** These are project-namespaced labels that predate this standard. They remain on existing containers and are not removed. New `com.agentcore.*` labels are added alongside them.

---

## 6. Auto-Recovery Contract

The cross-watcher + binary watcher together form the auto-recovery loop:

| Component | Lives in | Responsibility |
|-----------|----------|---------------|
| Cross-watcher (`dev-cross-supervised.sh`) | Host, via supervisord | Polls git commits every 30s; cross-compiles changed Rust; copies binary to `target/` |
| Binary watcher (`watch-binary.sh`) | Inside container | Polls `$SERVICE_BINARY` checksum every 3s; ELF-validates; restarts service via supervisorctl |
| Supervisord (inside container) | Inside container | Manages service process; auto-restarts on crash |

**Auto-recovery guarantees:**
- A new commit → cross-watcher builds → binary changes on disk → binary watcher detects → service restarts within ~30s + build time
- A service crash → supervisord auto-restarts (up to `startretries=5`); binary watcher self-heals every 30s if supervisord gives up
- A container restart → supervisord starts both programs fresh; binary watcher begins polling

**What auto-recovery does NOT handle:**
- Container itself crashes or is killed — requires `docker start <container>` or `docker compose up -d`
- Binary volume mount is broken (container started before binary exists) — binary watcher retries indefinitely; service starts once binary appears
- Cross-watcher itself crashes — supervisord on host restarts it (up to `startretries=10`)

---

## 7. Agent Rebuild/Restart Process — DO and DON'T

| Situation | DO | DON'T |
|-----------|-----|-------|
| Code change was committed | Wait for cross-watcher to detect and rebuild (≤ 30s + build time) | `docker restart <container>` — the new binary isn't ready yet |
| Binary is updated, service hasn't restarted | Wait for binary watcher (≤ 3s) | `docker exec <container> supervisorctl restart service` — binary watcher will do this automatically |
| Service is crashing in a loop | Check logs: `docker logs <container>` to find the root cause | `docker restart` in a loop — this masks the real problem |
| Container is stopped/exited (not crashed) | `docker start <container>` or `docker compose -f ... up -d <service>` | `cargo build` manually on the host — the cross-watcher handles builds |
| Container is **Dead** state | `docker rm <container>` then `docker compose ... up -d <service>` — Dead containers cannot be started | `docker restart` — restart does not work on Dead containers |
| You need a fresh binary right now | `cd <project> && cargo build --target aarch64-unknown-linux-musl` then wait for binary watcher | Rebuild the Docker image — images are built once, binaries are mounted |
| Cross-watcher is not running | `supervisorctl -c /Users/james/Projects/AgentCore/config/supervisord.conf start cross-watcher-<project>` | Touch `.claude-hooks-bypass` and manually run docker commands |

**The invariant:** Code changes flow through the cross-watcher → binary watcher pipeline. Agents do not manually build or restart unless there is a confirmed breakage in that pipeline. Every manual intervention must first check whether the pipeline is stuck.

---

## 8. Emergency Override Procedure

If the cross-watcher pipeline is confirmed broken (not just slow), an agent may manually intervene using this procedure:

1. **Confirm the pipeline is broken:**
   ```bash
   supervisorctl -c /Users/james/Projects/AgentCore/config/supervisord.conf status cross-watcher-<project>
   # Expected: RUNNING. If STOPPED or FATAL, the watcher is dead.
   ```

2. **Try to restart the watcher first:**
   ```bash
   supervisorctl -c /Users/james/Projects/AgentCore/config/supervisord.conf restart cross-watcher-<project>
   ```

3. **If the watcher cannot restart (binary corrupt, env broken), manually build:**
   ```bash
   cd /Users/james/Projects/<project>
   cargo build --target aarch64-unknown-linux-musl --bin <binary>
   # Then wait for the binary watcher inside the container to pick it up (≤ 3s)
   ```

4. **If the container itself is stopped:**
   ```bash
   docker compose -f docker-compose.services.yml [-f docker-compose.cross.yml] up -d <service>
   ```

5. **Audit every manual intervention:**
   Any use of `docker compose up`, `docker restart`, or `cargo build` that bypasses the watcher pipeline must be documented as a child bead explaining why the pipeline failed, so the root cause is fixed.

**Never use:**
- `AGENTCORE_WATCHER_OVERRIDE=1` without also creating a bead to fix the watcher (this env var is reserved for bd-u619c enforcement hook)
- `.claude-hooks-bypass` to work around Docker file restrictions

---

## 9. Init-Container Exclusion Rule

The docker-health-poller excludes containers from alerting when **either** condition is true:

1. The container has label `com.agentcore.container-role=init`
2. The container exited with code 0

**Why label AND exit-code:** Label-only would miss init containers that haven't been updated to carry the label yet. Exit-code-only would suppress alerts for app containers that exit cleanly (e.g., a misconfigured service that immediately exits 0). The combination is more robust: a container is an expected-exit only if it both carries the init label AND exits 0, **or** if it exits 0 and there is no prior healthy state (i.e., it was never a running app container).

The actual implementation uses: label `com.agentcore.container-role=init` OR (exit code 0 AND never transitioned through a running/healthy state in the current poller session).

See `scripts/docker-health-poller.sh` for implementation.

---

## 10. Cross-Watcher Registration

Every Rust project with a dev container must have a cross-watcher entry in `config/supervisord.conf` (AgentCore). The program name follows the pattern `cross-watcher-<project-slug>`.

```ini
[program:cross-watcher-<project-slug>]
command=/Users/james/Projects/AgentCore/scripts/dev-cross-supervised.sh /Users/james/Projects/<Project-Dir> [--workspace | --bin <binary> --container <container>]
directory=/Users/james/Projects/<Project-Dir>
autostart=true
autorestart=true
startretries=10
startsecs=10
priority=990
stdout_logfile=/Users/james/Projects/<Project-Dir>/logs/cross-watcher-<project-slug>.log
stdout_logfile_maxbytes=10MB
stdout_logfile_backups=3
stderr_logfile=/Users/james/Projects/<Project-Dir>/logs/cross-watcher-<project-slug>.err
stderr_logfile_maxbytes=5MB
stderr_logfile_backups=2
environment=PROJECT_ROOT="/Users/james/Projects/<Project-Dir>"
```

**When to use `--workspace` vs `--bin`:**
- `--workspace` — project is a Cargo workspace with multiple crates (7D-Solutions Platform)
- `--bin <name> --container <name>` — project has a single Rust binary (Fireproof-ERP, RanchOrbit, TrashTech)

**Current cross-watcher registrations:**

| Program name | Project | Mode |
|-------------|---------|------|
| `cross-watcher-7d-solutions-platform` | 7D-Solutions Platform | `--workspace` |
| `cross-watcher-fireproof` | Fireproof-ERP | `--bin fireproof-erp --container fireproof-erp` |
| ~~`cross-watcher-ranchorbit`~~ | RanchOrbit | **MISSING** — tracked in bd-wivcb |
| ~~`cross-watcher-trashtech`~~ | TrashTech | **MISSING** — tracked in bd-wivcb |

After adding a new cross-watcher entry, regenerate supervisord config and reload:
```bash
$PROJECT_ROOT/scripts/generate-supervisord-conf.sh --reload
```
