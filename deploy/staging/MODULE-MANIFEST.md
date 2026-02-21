# Staging Module Manifest

> **Purpose:** Tracks the pinned image versions currently deployed (or targeted for deployment) to the staging environment.
> **Updated by:** Agents/CI after each `push_images.sh` run.
> **Image tag format:** `{semver}-{git-sha7}` — immutable, never 'latest'.

## Pinned Versions

| Service | Canonical Name | Version | Git SHA | Full Image Tag | Notes |
|---------|---------------|---------|---------|----------------|-------|
| Platform: Control Plane | `control-plane` | 0.1.0 | — | `7dsolutions/control-plane:0.1.0-{sha}` | Pending first staging push |
| Platform: Identity Auth | `identity-auth` | 1.1.0 | — | `7dsolutions/identity-auth:1.1.0-{sha}` | Pending first staging push |
| Module: TTP | `ttp` | 0.1.0 | — | `7dsolutions/ttp:0.1.0-{sha}` | Pending first staging push |
| Module: AR | `ar` | 0.1.0 | — | `7dsolutions/ar:0.1.0-{sha}` | Pending first staging push |
| Module: Payments | `payments` | 0.1.0 | — | `7dsolutions/payments:0.1.0-{sha}` | Pending first staging push |
| App: TCP UI | `tenant-control-plane-ui` | 0.1.0 | — | `7dsolutions/tenant-control-plane-ui:0.1.0-{sha}` | Pending first staging push |

## How to Update This File

After a successful push:
1. Run `bash scripts/staging/list_versions.sh` to get resolved tags.
2. Update the table above with the actual git SHA and full image tag.
3. Commit with `[bd-xxx] Update staging manifest to {sha}`.

## Registry

Default registry prefix: `7dsolutions`
Override: `export IMAGE_REGISTRY=ghcr.io/your-org` before running build/push scripts.

## Rollback

To roll back a service, push the previous pinned tag and update this manifest:
```bash
export IMAGE_REGISTRY=7dsolutions
docker pull 7dsolutions/ar:0.1.0-abc1234   # previous good tag
# update docker-compose.staging.yml to reference the old tag
```

Tags are immutable — old tags are always available for rollback.
