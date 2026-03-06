# Dev Workflow Proposal: Hybrid Build Strategy for Consumer Repos

**Date:** 2026-03-04
**Status:** Draft — for cross-team review
**Context:** Fireproof-ERP Docker friction, platform crate distribution, health recovery gaps

---

## Problems

Three issues keep surfacing, each with a different owner:

1. **Slow dev iteration (Fireproof-ERP):** Every upstream platform change or migration issue triggers a full Rust compile inside Docker. No incremental builds. Minutes per cycle.
2. **No dependency isolation (shared concern):** Platform crate changes, Compose config changes, and migration conflicts all cascade into the consumer container. No version boundary.
3. **Alert without recovery (AgentCore/Platform):** The Prometheus + Alertmanager stack detects failures (service-down within 1–5 min, outbox backlog, payment failures) and sends webhooks, but nothing acts on them automatically. Recovery waits for a human.

These are three separate workstreams with different owners. This proposal breaks them apart.

---

## Workstream 1: Hybrid Build (Fireproof-ERP Owns)

### The Idea

Stop compiling Rust inside Docker for dev. Build locally with `cargo build` (fast, incremental — seconds). Run the resulting binary inside Docker Compose via a volume mount. Keep all Docker visibility: `docker compose ps`, centralized logs, health checks, Prometheus scraping, restart policies.

### What Changes in Fireproof-ERP's docker-compose

The current multi-stage Rust build service gets replaced with a thin runtime container:

```yaml
fireproof-erp:
  image: debian:bookworm-slim
  volumes:
    - ./target/release/fireproof-erp:/app/fireproof-erp:ro
  command: /app/fireproof-erp
  environment:
    - DATABASE_URL=${DATABASE_URL}
    - NATS_URL=nats://7d-nats:4222
    - NATS_AUTH_TOKEN=${NATS_AUTH_TOKEN}
    - RUST_LOG=${RUST_LOG:-info}
  networks:
    - 7d-platform
  healthcheck:
    test: ["CMD", "curl", "-f", "http://localhost:8080/healthz"]
    interval: 10s
    timeout: 5s
    retries: 5
  # All existing cap_drop, read_only, tmpfs settings carry over
```

### Dev Workflow Becomes

```bash
# 1. Start infrastructure (unchanged — platform services, NATS, Postgres all stay in Docker)
docker compose -f docker-compose.data.yml up -d
docker compose -f docker-compose.services.yml up -d

# 2. Build locally (incremental — seconds after first build)
cargo build --release -p fireproof-erp

# 3. Start the app container (mounts the local binary)
docker compose up -d fireproof-erp

# 4. After code changes, rebuild and restart
cargo build --release -p fireproof-erp
docker compose restart fireproof-erp
```

### What This Preserves

- `docker compose ps` — still shows fireproof-erp as a managed service
- `docker compose logs fireproof-erp` — still works
- Prometheus scrapes `/metrics` — unchanged
- Alertmanager fires on service-down — unchanged
- Restart policies — unchanged
- Health checks — unchanged
- Agent visibility — identical to today

### What This Fixes

- No more full Rust compiles inside Docker (seconds vs. minutes)
- Platform crate source changes don't trigger container rebuilds
- Migration errors surface immediately as local compile/runtime errors
- Docker build still exists for CI/staging/production — only dev changes

### Who Does This

**Fireproof-ERP agents only.** This is entirely within their docker-compose and Dockerfile. No changes needed to the platform repo. No changes to AgentCore. Platform services continue running in Docker exactly as they do now.

### Docker Build for CI/Production

The existing multi-stage Dockerfile stays for CI/staging/production. Only the dev workflow changes. Consider a `docker-compose.dev.yml` override that swaps the build service for the volume-mount approach, so `docker compose -f docker-compose.yml -f docker-compose.dev.yml up` gives the hybrid behavior.

---

## Workstream 2: Private Registry (AgentCore Owns)

### The Idea

Publish shared platform crates to a private Cargo registry so consumers declare normal versioned dependencies instead of filesystem paths.

```toml
# Consumer Cargo.toml — clean, no path hacking
[dependencies]
event-bus = { version = "0.1", registry = "7d-platform" }
```

### What Changes in the Platform Repo (AgentCore)

1. **`.cargo/config.toml`** — add registry block:

```toml
[registries.7d-platform]
index = "https://your-registry-url/index"  # Cloudsmith, Artifactory, or git-based
```

2. **Each shared platform crate's `Cargo.toml`** — add publish restriction:

```toml
[package]
publish = ["7d-platform"]  # Prevents accidental publish to crates.io
```

3. **CI pipeline (GitHub Actions)** — add a publish step after successful builds. This slots into the existing Gate 2 (CI image pipeline) that already detects version bumps via `detect_version_intent.sh`:

```yaml
# After Docker image build succeeds
- name: Publish to registry
  if: steps.version_intent.outputs.bumped == 'true'
  run: cargo publish -p ${{ steps.version_intent.outputs.crate }} --registry 7d-platform
```

4. **Immutable versions for unproven crates** — event-bus is v0.1.0 (unproven), which currently has relaxed versioning rules. For registry publishes, even unproven crates should be immutable once published. Bump PATCH for any change that gets published. This prevents consumers getting different code for the same version string.

### What Changes in Consumer Repos (Fireproof-ERP, Future Consumers)

1. **`.cargo/config.toml`** — add the same registry reference
2. **`Cargo.toml`** — replace path dependencies with versioned registry dependencies
3. **Dockerfile** — remove `additional_contexts`, remove `sed`-patching of Cargo.toml paths
4. **Docker Compose** — remove platform repo filesystem mounts

### Registry Options

| Option | Pros | Cons |
|--------|------|------|
| **GitHub Packages** | Already on GitHub Actions, auth is native, zero new infra | Cargo support is still limited |
| **Cloudsmith** | Full Cargo support, hosted, good CI integration | Adds a service dependency + cost |
| **Artifactory** | Enterprise-grade, supports many package types | Heavy, expensive |
| **Kellnr (self-hosted)** | Lightweight, purpose-built for Rust | You maintain it |
| **Git-based index** | Simplest, no external service | Limited features, manual management |

### Who Does This

- **AgentCore**: Registry setup, `.cargo/config.toml`, publish restrictions on platform crates, CI publish step
- **Fireproof-ERP agents**: Swap path deps → versioned deps, clean up Docker hacks
- **Future consumers**: Just add the registry to their config and declare dependencies normally

### Timing

Not urgent. The hybrid build approach (Workstream 1) eliminates the immediate pain. The registry becomes important when a second consumer repo needs platform crates, or when more platform crates beyond event-bus get shared.

---

## Workstream 3: Health Recovery (AgentCore Owns)

### The Idea

The monitoring stack is solid — Prometheus scrapes 18 services every 15s, Alertmanager routes by severity (billing critical at 1min, platform at 2min, modules at 5min), Grafana dashboards cover service health, outbox backlog, payment failures, and projection lag. The gap: alerts fire to `host.docker.internal:9095/alertmanager` via webhook, but nothing acts on them.

### Options

**Option A: Recovery sidecar (simple)**

A lightweight service that receives Alertmanager webhooks and takes basic recovery actions:

```yaml
recovery-agent:
  image: curlimages/curl:latest  # or a small custom image
  volumes:
    - /var/run/docker.sock:/var/run/docker.sock:ro
  # Receives webhooks from Alertmanager, restarts failed containers
```

Actions by alert type:
- `service_down` → `docker restart <container>`
- `outbox_backlog_critical` → restart the service's outbox relay
- `nats_connection_lost` → restart affected service (NATS will reconnect)

Limitations: restart-only, no root cause analysis, risk of restart loops.

**Option B: Escalation tiers**

- Tier 1 (automatic): Restart on first alert, with a cooldown (don't restart the same service more than 2x in 10 minutes)
- Tier 2 (notify): If restart doesn't resolve within the cooldown, escalate — page/Slack/email with diagnostic context
- Tier 3 (manual): Human intervention for persistent failures

This prevents restart loops while still handling transient failures automatically.

### Who Does This

**AgentCore.** The monitoring stack (`docker-compose.monitoring.yml`), alert rules (`infra/monitoring/alerts/`), and Alertmanager config all live in the platform repo. A recovery service would be added to the monitoring compose file. Consumer repos don't need to change — they already expose `/healthz` and `/api/ready`.

---

## Summary: Who Owns What

| Workstream | Owner | Platform Repo Changes | Consumer Repo Changes | Priority |
|---|---|---|---|---|
| **Hybrid Build** | Fireproof-ERP | None | docker-compose, Dockerfile | Now |
| **Private Registry** | AgentCore + consumers | .cargo/config, Cargo.toml publish fields, CI publish step | .cargo/config, Cargo.toml deps, Dockerfile cleanup | When ready |
| **Health Recovery** | AgentCore | docker-compose.monitoring.yml, new recovery service | None | When ready |

### Dependency Between Workstreams

None. These are fully independent. Fireproof-ERP can implement hybrid builds today without waiting for the registry or recovery work. AgentCore can set up the registry without affecting any consumer's current workflow. Recovery can be added to the monitoring stack without touching any service code.
