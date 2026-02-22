# Platform Completion Gate

**Gate script:** `scripts/proof_platform_completion.sh`
**Defined in:** Phase 47, bead bd-32ef (P47-900 Capstone)

---

## What "Done" Means

The 7D Solutions Platform is considered **complete** when a single run of the completion gate returns exit 0 against a release candidate environment. The gate proves three independent invariants in sequence:

| Gate | What It Proves |
|------|---------------|
| 1 — Contract Validation | All OpenAPI specs in `contracts/` are syntactically valid YAML/JSON. |
| 2 — Breaking-Change Gate | No API breaking changes landed without an acknowledged version bump in `info.version`. |
| 3 — Perf Smoke | The billing spine endpoints meet response-time and error-rate thresholds under a single-VU smoke load. |
| 4 — Onboarding E2E | The TCP UI onboarding wizard creates tenants correctly, end-to-end against real services. |

All four gates must pass. Skipping a gate is allowed during development but **not** for a production release candidate.

---

## How to Run

### Local (development check)

```bash
# All gates — requires k6, Node/npx, and a running TCP UI on :3000
./scripts/proof_platform_completion.sh

# Skip perf if k6 is not installed
./scripts/proof_platform_completion.sh --skip-perf

# Skip Playwright if TCP UI is not running
./scripts/proof_platform_completion.sh --skip-e2e
```

### Against Staging

```bash
export STAGING_HOST=staging.7dsolutions.app
export PERF_AUTH_EMAIL=perf@7d.staging
export PERF_AUTH_PASSWORD=<secret>
export BASE_URL=https://tcp.7dsolutions.app

./scripts/proof_platform_completion.sh --staging "$STAGING_HOST"
```

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `STAGING_HOST` | _(empty)_ | VPS hostname/IP for k6 perf scenarios |
| `PERF_AUTH_EMAIL` | _(empty)_ | Login email for k6 auth |
| `PERF_AUTH_PASSWORD` | _(empty)_ | Login password for k6 auth |
| `PERF_AUTH_TOKEN` | _(empty)_ | Pre-minted JWT (skips k6 login step) |
| `BASE_URL` | `http://localhost:3000` | Playwright base URL for TCP UI |
| `BASE_REF` | `HEAD~1` | Git ref used as baseline for breaking-change diff |

---

## Gate Details

### Gate 1 + 2 — Contracts

Contract specs live in `contracts/` and follow the policy in `docs/architecture/CONTRACT-VERSIONING-POLICY.md`.

- Gate 1 validates every `*.yaml` and `*.json` file parses without errors.
- Gate 2 runs `scripts/ci/check-openapi-breaking-changes.sh` to detect removed paths or required fields that were not acknowledged with a version bump.

Both gates run on every CI push and PR via `.github/workflows/ci.yml`.

### Gate 3 — Perf Smoke

Runs `tools/perf/smoke.js` (k6) against the target environment.

The smoke scenario exercises 5 critical endpoints with 1 VU × 1 iteration — a fast readiness sanity check. Threshold breach exits non-zero and fails the gate.

For sustained load testing use `tools/perf/baseline_billing_spine.js`. This is run separately via `.github/workflows/perf.yml` (manual dispatch).

See `tools/perf/README.md` for full k6 usage and threshold definitions.

### Gate 4 — Onboarding E2E

Runs `apps/tenant-control-plane-ui/tests/onboarding-wizard.spec.ts` via Playwright.

Covers:
- Wizard renders and navigates through all 3 steps.
- Required-field and step-sequence guardrails are enforced.
- BFF routes are called (no direct upstream calls from the browser).
- Success and error states are handled correctly.
- Server-side guardrail blocks user creation for non-existent tenants.

Requires the TCP UI (`apps/tenant-control-plane-ui/`) and all upstream services to be running.

---

## Recovery Guidance

If a gate fails:

| Gate | Next Action |
|------|-------------|
| Contract Validation | Run `python3 -c "import yaml; yaml.safe_load(open('contracts/...').read())"` on the failing spec to see the parse error. |
| Breaking-Change Gate | Either revert the breaking change or bump `info.version` in the affected spec and document the migration in a REVISIONS entry. |
| Perf Smoke | Check if the target service is up. Review threshold settings in `tools/perf/config/`. |
| Onboarding E2E | Run `cd apps/tenant-control-plane-ui && npx playwright test tests/onboarding-wizard.spec.ts --headed` for interactive debugging. Check BFF logs for upstream errors. |

---

## CI Integration

The contract gates run automatically on every PR via `ci.yml`.

The perf gate is launched manually via `perf.yml` (workflow_dispatch) to avoid noise and cost on every PR.

The onboarding E2E is part of the full Playwright suite run in CI.

To trigger the full completion gate in CI, all three workflows must have passed on the same git SHA.

---

## Definition of "Release Candidate"

A release candidate is a git SHA where:

1. `scripts/proof_platform_completion.sh` exits 0 (no `--skip-*` flags) against the staging environment.
2. `MODULE-MANIFEST.md` reflects all module versions actually deployed.
3. No open P0/P1 beads exist in the bead pool.

When all three conditions are met, the platform is complete and ready for production promotion.
