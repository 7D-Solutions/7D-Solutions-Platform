# Deployment Target & Staging Strategy

> **Who reads this:** All agents building or deploying services. Platform Orchestrator for infrastructure decisions.
> **Status:** Staging skeleton — scripts exist, registry configured, no live staging environment yet.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | SageDesert | Initial: deployment target, registry, staging push/deploy scripts. |

---

## Deployment Target

**Staging environment:** Single VPS running Docker Compose.

### Rationale

1. **Builds on existing infrastructure.** The platform already runs via `docker-compose.yml` with a three-stack model (data / backend / frontend). Staging reuses this pattern with pinned image versions instead of local builds.
2. **Minimal ops burden.** A single VM with Docker and Docker Compose installed requires no orchestration layer (Kubernetes, ECS). For staging validation, this is sufficient.
3. **Reproducible.** The same `docker compose` commands that work locally work on the staging VM. No translation layer between dev and staging.
4. **Cost-efficient.** A single VM (2-4 vCPU, 8GB RAM) runs the full stack for staging validation.

### Staging VM Requirements

- Ubuntu 22.04+ or Debian 12+
- Docker Engine 24+
- Docker Compose v2
- SSH access for deploy scripts
- Accessible on a private network or behind a firewall (staging is not public)

### Production Target (future)

Production deployment strategy is deferred. Options under consideration: managed container service (ECS, Cloud Run, Fly.io) or self-hosted Kubernetes. This decision will be made when the platform reaches production readiness. The staging scripts are designed so the image build/tag/push step is reusable regardless of the production orchestrator.

---

## Container Registry

**Registry:** GitHub Container Registry (ghcr.io)

### Rationale

1. **Co-located with source.** The repository is on GitHub (`7D-Solutions/7D-Solutions-Platform`). GHCR is natively integrated — no separate account or credential management.
2. **Free for the repo's plan.** GHCR storage and bandwidth are included in GitHub plans.
3. **Supports immutable tags.** Once an image is pushed with a version tag, the tag should never be overwritten (enforced by convention; see `docs/VERSIONING.md`).
4. **CI integration.** GitHub Actions can push to GHCR with `GITHUB_TOKEN` — no external secrets needed.

### Image Naming Convention

Aligned with `docs/VERSIONING.md` Section "Container Registry":

```
ghcr.io/7d-solutions/7d-{service}:{version}
```

**Examples:**

| Service | Image | Tag Example |
|---------|-------|-------------|
| identity-auth | `ghcr.io/7d-solutions/7d-auth` | `1.1.0` |
| ar | `ghcr.io/7d-solutions/7d-ar` | `0.1.0` |
| gl | `ghcr.io/7d-solutions/7d-gl` | `0.1.0` |
| tcp-ui | `ghcr.io/7d-solutions/7d-tcp-ui` | `0.1.0` |

### Naming Rules

- **Prefix:** All images use `7d-` prefix for namespace clarity.
- **Service name:** Matches the canonical module name from `docs/VERSIONING.md` (e.g., `ar`, `gl`, `auth`). Exception: `identity-auth` uses `auth` (the binary name).
- **Tag:** Exact SemVer version from the service's `Cargo.toml` or `package.json`. No `latest` in staging or production.
- **Immutability:** A pushed version tag is never overwritten. If a tag is wrong, bump to the next version.

### Authentication

To push images locally (before CI automation):

```bash
echo "$GITHUB_TOKEN" | docker login ghcr.io -u USERNAME --password-stdin
```

The `GITHUB_TOKEN` needs `write:packages` scope. In GitHub Actions, `GITHUB_TOKEN` has this by default.

---

## Staging Scripts

### scripts/staging_push.sh

Builds Docker images for a service subset, tags them with exact versions from package files, and pushes to GHCR.

**Usage:**
```bash
# Dry run — show what would be built and pushed
./scripts/staging_push.sh --dry-run

# Push to registry
./scripts/staging_push.sh
```

**Initial service subset:** `identity-auth`, `ar`, `tcp-ui`

### scripts/staging_deploy.sh

Deploys the built images to the staging VM by generating a staging-specific Docker Compose override and running it via SSH.

**Usage:**
```bash
# Dry run — show the compose file and deploy command
./scripts/staging_deploy.sh --dry-run

# Deploy to staging
./scripts/staging_deploy.sh
```

**Requires:** `STAGING_HOST` environment variable (SSH target, e.g., `deploy@staging.example.com`).

---

## Staging Compose Model

Staging uses the same `docker-compose.data.yml` for infrastructure (Postgres, NATS) and a generated `docker-compose.staging.yml` for services. The staging compose file references GHCR images at pinned versions instead of building from source.

This means:
1. Infrastructure (databases, NATS) runs directly on the staging VM.
2. Application services pull pre-built images from GHCR.
3. Version pinning is explicit — no `:latest` tags.

---

## Relationship to Versioning Standard

This document implements the deployment decisions referenced in `docs/VERSIONING.md`:

- **Gate 2 (CI image pipeline):** `staging_push.sh` is the manual precursor to automated CI image builds. When CI automation is built, it will call the same build/tag/push logic.
- **Registry setup:** GHCR is now the selected registry, filling the "Not yet selected" gap in VERSIONING.md.
- **Image naming:** The convention above matches the VERSIONING.md pattern `{registry}/{module-name}:{version}`.

---

> This document is the source of truth for deployment targets and staging infrastructure.
