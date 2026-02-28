# Production Module Manifest

> **Purpose:** Tracks the pinned image versions currently deployed (or targeted for deployment) to the production environment.
> **Updated by:** Agents/CI after each successful promotion through the staging gate.
> **Image tag format:** `{semver}-{git-sha7}` — immutable, never 'latest'.
> **Invariant:** Production is only ever deployed from this manifest. No ad-hoc tag overrides.

## Pinned Versions

| Service | Canonical Name | Version | Git SHA | Full Image Tag | Notes |
|---------|---------------|---------|---------|----------------|-------|
| Platform: Control Plane | `control-plane` | 1.0.0 | — | `7dsolutions/control-plane:1.0.0-{sha}` | Promoted P44 (bd-qvbg). Pending image build/push via promote.yml |
| Platform: Tenant Registry | `tenant-registry` | 1.0.2 | — | `7dsolutions/tenant-registry:1.0.2-{sha}` | Promoted P44 (bd-tzsh), patched P46 (bd-2t65 seed-password fix). Pending image build/push |
| Platform: Identity Auth | `identity-auth` | 1.2.1 | — | `7dsolutions/identity-auth:1.2.1-{sha}` | Promoted P48 (bd-26ro key rotation + proof gate). Pending image build/push |
| Module: TTP | `ttp` | 1.0.0 | — | `7dsolutions/ttp:1.0.0-{sha}` | Promoted P44 (bd-2dq8). Pending image build/push via promote.yml |
| Module: AR | `ar` | 1.0.0 | — | `7dsolutions/ar:1.0.0-{sha}` | Promoted P44 (bd-rqbr). Pending image build/push via promote.yml |
| Module: Payments | `payments` | 1.0.0 | — | `7dsolutions/payments:1.0.0-{sha}` | Promoted P44 (bd-1b1x). Pending image build/push via promote.yml |

## How to Update This File

After a successful staging proof gate and promotion approval:
1. Run `bash scripts/production/manifest_validate.sh deploy/production/MODULE-MANIFEST.md` to confirm images exist in the registry.
2. Update the table above with the actual git SHA and full image tag.
3. Commit with `[bd-xxx] Update production manifest to {sha}`.
4. The production deploy consumes this file as the only source of image tags.

## Registry

Default registry prefix: `7dsolutions`
Override: `export IMAGE_REGISTRY=ghcr.io/your-org` before running build/push scripts.

## Rollback

To roll back production, update this manifest to the prior pinned tag and re-run the deploy:
```bash
# 1. Identify the prior good tag from deployment history:
bash scripts/production/rollback_stack.sh --history

# 2. Roll back to that tag (also updates this manifest):
bash scripts/production/rollback_stack.sh --tag v1.0.0-abc1234
```

Tags are immutable — prior tags are always available in the registry for rollback.

## Manifest Discipline

- **Never deploy 'latest'** — all entries must be `{semver}-{git-sha7}` format.
- **Pending entries** (SHA = "—" or tag contains `{sha}`) are skipped by validate/diff; they are not yet deployed.
- **Drift detection:** Run `bash scripts/production/manifest_diff.sh deploy/production/MODULE-MANIFEST.md` to verify what is actually running matches this manifest.
