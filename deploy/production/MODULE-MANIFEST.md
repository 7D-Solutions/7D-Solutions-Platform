# Production Module Manifest

> **Purpose:** Tracks the pinned image versions currently deployed (or targeted for deployment) to the production environment.
> **Updated by:** Agents/CI after each successful promotion through the staging gate.
> **Image tag format:** `{release}-{git-sha}` — immutable, never 'latest'.
> **Invariant:** Production is only ever deployed from this manifest. No ad-hoc tag overrides.

## Pinned Versions

| Service | Canonical Name | Version | Git SHA | Full Image Tag | Notes |
|---------|---------------|---------|---------|----------------|-------|
| Platform: Identity Auth | `identity-auth` | 1.2.1 | d3b0e8932887 | `7dsolutions/identity-auth:phase66-d3b0e8932887` | Promoted P48 (bd-26ro key rotation + proof gate) |
| Platform: Control Plane | `control-plane` | 1.0.0 | d3b0e8932887 | `7dsolutions/control-plane:phase66-d3b0e8932887` | Promoted P44 (bd-qvbg) |
| Platform: Tenant Registry | `tenant-registry` | 1.0.2 | d3b0e8932887 | `7dsolutions/tenant-registry:phase66-d3b0e8932887` | Promoted P44 (bd-tzsh), patched P46 (bd-2t65 seed-password fix) |
| Module: TTP | `ttp` | 1.0.0 | d3b0e8932887 | `7dsolutions/ttp:phase66-d3b0e8932887` | Promoted P44 (bd-2dq8) |
| Module: AR | `ar` | 1.0.0 | d3b0e8932887 | `7dsolutions/ar:phase66-d3b0e8932887` | Promoted P44 (bd-rqbr) |
| Module: Payments | `payments` | 1.0.0 | d3b0e8932887 | `7dsolutions/payments:phase66-d3b0e8932887` | Promoted P44 (bd-1b1x) |
| Module: Subscriptions | `subscriptions` | 0.1.0 | d3b0e8932887 | `7dsolutions/subscriptions:phase66-d3b0e8932887` | Phase 66 release |
| Module: Notifications | `notifications` | 0.1.0 | d3b0e8932887 | `7dsolutions/notifications:phase66-d3b0e8932887` | Phase 66 release |
| Module: GL | `gl` | 0.1.0 | d3b0e8932887 | `7dsolutions/gl:phase66-d3b0e8932887` | Phase 66 release |
| Module: Inventory | `inventory` | 0.1.0 | d3b0e8932887 | `7dsolutions/inventory:phase66-d3b0e8932887` | Phase 66 release |
| Module: AP | `ap` | 0.1.0 | d3b0e8932887 | `7dsolutions/ap:phase66-d3b0e8932887` | Phase 66 release |
| Module: Treasury | `treasury` | 0.1.0 | d3b0e8932887 | `7dsolutions/treasury:phase66-d3b0e8932887` | Phase 66 release |
| Module: Fixed Assets | `fixed-assets` | 0.1.0 | d3b0e8932887 | `7dsolutions/fixed-assets:phase66-d3b0e8932887` | Phase 66 release |
| Module: Consolidation | `consolidation` | 0.1.0 | d3b0e8932887 | `7dsolutions/consolidation:phase66-d3b0e8932887` | Phase 66 release |
| Module: Timekeeping | `timekeeping` | 0.1.0 | d3b0e8932887 | `7dsolutions/timekeeping:phase66-d3b0e8932887` | Phase 66 release |
| Module: Party | `party` | 0.1.0 | d3b0e8932887 | `7dsolutions/party:phase66-d3b0e8932887` | Phase 66 release |
| Module: Integrations | `integrations` | 0.1.0 | d3b0e8932887 | `7dsolutions/integrations:phase66-d3b0e8932887` | Phase 66 release |
| Module: PDF Editor | `pdf-editor` | 0.1.0 | d3b0e8932887 | `7dsolutions/pdf-editor:phase66-d3b0e8932887` | Phase 66 release |
| Module: Maintenance | `maintenance` | 0.1.0 | d3b0e8932887 | `7dsolutions/maintenance:phase66-d3b0e8932887` | Phase 66 release |
| Module: Shipping & Receiving | `shipping-receiving` | 0.1.0 | d3b0e8932887 | `7dsolutions/shipping-receiving:phase66-d3b0e8932887` | Phase 66 release |
| Module: Quality Inspection | `quality-inspection` | 0.1.0 | d3b0e8932887 | `7dsolutions/quality-inspection:phase66-d3b0e8932887` | Phase 66 release |
| Module: BOM | `bom` | 0.1.0 | d3b0e8932887 | `7dsolutions/bom:phase66-d3b0e8932887` | Phase 66 release |
| Module: Production | `production` | 0.1.0 | d3b0e8932887 | `7dsolutions/production:phase66-d3b0e8932887` | Phase 66 release |
| Module: Workflow | `workflow` | 0.1.0 | d3b0e8932887 | `7dsolutions/workflow:phase66-d3b0e8932887` | Phase 66 release |
| Module: Numbering | `numbering` | 0.1.0 | d3b0e8932887 | `7dsolutions/numbering:phase66-d3b0e8932887` | Phase 66 release |
| Module: Workforce Competence | `workforce-competence` | 0.1.0 | d3b0e8932887 | `7dsolutions/workforce-competence:phase66-d3b0e8932887` | Phase 66 release |
| Module: Customer Portal | `customer-portal` | 0.1.0 | d3b0e8932887 | `7dsolutions/customer-portal:phase66-d3b0e8932887` | Phase 66 release |
| Module: Reporting | `reporting` | 0.1.0 | d3b0e8932887 | `7dsolutions/reporting:phase66-d3b0e8932887` | Phase 66 release |
| Infra: Auth LB | `auth-lb` | — | upstream-nginx | `nginx:alpine` | Upstream nginx, not a 7D image |
| Infra: Gateway | `gateway` | — | upstream-nginx | `nginx:alpine` | Upstream nginx, not a 7D image |

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

- **Never deploy 'latest'** — all entries must use immutable image tags.
- **Pending entries** (SHA = "—" or tag contains `{sha}`) are skipped by validate/diff; they are not yet deployed.
- **Drift detection:** Run `bash scripts/production/manifest_diff.sh deploy/production/MODULE-MANIFEST.md` to verify what is actually running matches this manifest.
