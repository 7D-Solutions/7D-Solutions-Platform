# Patch Cadence SLA

**Bead:** bd-1t8gw  
**Audience:** Security auditors, Intuit integration review  
**Owner:** Platform Engineering  
**Last reviewed:** 2026-04-25

---

## 1. Scope

This policy covers all software layers in the 7D Solutions Platform production environment:

| Layer | Examples |
|-------|---------|
| Host OS | Debian/Ubuntu packages on the production server |
| Docker base images | `rust:slim`, `debian:bookworm-slim` used in service containers |
| Rust dependencies | `Cargo.lock` workspace crates (sourced from crates.io) |
| Node.js dependencies | `package-lock.json` for tooling and scripts |
| Secrets / certificates | TLS certs, API keys (covered separately in `secret-rotation.md`) |

Third-party SaaS integrations (QuickBooks, Stripe, carrier APIs) are patched on the vendor's schedule. We track their CVE advisories but do not control their patch timing.

---

## 2. Severity SLA

Severity is assigned by CVSS base score or the vendor's published rating, whichever is higher.

| Severity | CVSS Range | Patch Applied Within |
|----------|-----------|---------------------|
| Critical | 9.0–10.0 | 7 calendar days of vendor release |
| High | 7.0–8.9 | 7 calendar days of vendor release |
| Medium | 4.0–6.9 | 30 calendar days of vendor release |
| Low | 0.1–3.9 | 90 calendar days of vendor release |

"Applied within N days" means: the patched artifact is deployed to production within N days of the earliest public disclosure date (NVD publish date or vendor advisory, whichever is earlier).

---

## 3. Scheduled Review Cadence

### Weekly (every Monday)
- Review `cargo audit` output from CI — triage new advisories against the SLA table above.
- Review Dependabot alerts for Node dependencies.
- Check Docker Hub / GitHub Container Registry for updated base image tags.
- Any Critical or High findings trigger an out-of-band patch window that week.

### Monthly (first Monday of each month)
- Full dependency bump pass: `cargo update` across the workspace, then run full integration test suite.
- Pull latest Docker base image tag, rebuild, and deploy if tests pass.
- Review host OS `unattended-upgrades` logs; confirm no packages were held back.
- Update this document if SLAs or scope change.

---

## 4. Automation

| Tool | Layer | What it does |
|------|-------|-------------|
| `cargo audit` (CI) | Rust deps | Runs on every pull request; fails the build on Critical/High advisories |
| Dependabot | Node deps + Rust | Opens PRs for dependency updates; merges blocked until CI passes |
| `unattended-upgrades` (host) | Host OS | Automatically applies security-only packages nightly; kernel upgrades require a manual reboot window |
| Weekly CI cron | Docker base images | Pulls latest digest, rebuilds images, runs integration tests; creates a PR if the digest changed |

No automated tool gates production deploys without a passing integration test suite. Automation opens PRs; engineers merge after review.

---

## 5. Exception Process

A patch may be deferred beyond its SLA window only when applying it would cause demonstrable service disruption and no mitigating control is available.

**Steps to defer a patch:**

1. **Document the risk.** Create a bead (or GitHub issue) titled `[DEFERRED-PATCH] CVE-XXXX-XXXXX` with:
   - CVE identifier and CVSS score
   - Why the patch cannot be applied within SLA (e.g., upstream bug, breaking API change)
   - What mitigating control is in place (network isolation, WAF rule, feature flag)
   - New target date (must be ≤ 2× the standard SLA window)

2. **Approve the deferral.** The Platform Engineering lead signs off in writing (bead comment or email).

3. **Track weekly.** The deferred patch is reviewed every Monday until resolved.

4. **No indefinite deferrals.** If the patch cannot be applied within 2× the SLA window, escalate to leadership and document the business decision.

All active deferrals are listed in `docs/operations/patch-deferrals.md` (created when the first deferral occurs).

---

## 6. Evidence for Auditors

| Artifact | Location |
|----------|---------|
| CI `cargo audit` logs | GitHub Actions run history for each PR |
| Dependabot activity | GitHub → Security → Dependabot alerts |
| Host OS patch log | `/var/log/unattended-upgrades/unattended-upgrades.log` on the production host |
| Docker base image rebuild PRs | GitHub PR history, label `base-image-update` |
| Active patch deferrals | `docs/operations/patch-deferrals.md` |

---

*This document is reviewed monthly and updated whenever scope or SLAs change. For questions, contact the Platform Engineering team.*
