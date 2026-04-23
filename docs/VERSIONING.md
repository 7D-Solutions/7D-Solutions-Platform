# Module Versioning & Release Gating Standard

> **Who reads this:** All Claude Code agents — platform agents modifying modules, product agents adopting module versions.
> **What it covers:** How modules are versioned, how changes are gated, and how products control which module versions they run.
> **Parent:** This is a standalone platform standard. Referenced from CLAUDE.md in every project that uses platform modules.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.6 | 2026-02-21 | CopperRiver (bd-17u3) | Promotion runner added: scripts/versioning/promote_module.sh. Single-command end-to-end pipeline respecting Gates 1/2/3. Documented in Implementation Status. |
| 1.5 | 2026-02-21 | MaroonHarbor (bd-3gct) | Gate 2 built: detect_version_intent.sh + ci.yml gate2-detect-intent/gate2-image-build jobs. Documented tag scheme. Updated Implementation Status table. |
| 1.4 | 2026-02-20 | Platform Orchestrator | Gate 1 built: pre-commit hook installed. Doc audit: archived superseded docs, identity-auth REVISIONS.md backfilled. |
| 1.3 | 2026-02-20 | Platform Orchestrator | ChatGPT round 3: canonical module naming, which-file-to-bump rules, docs-only exception, upgrade ordering for breaking changes, deployment config source of truth, proof completeness principle. |
| 1.2 | 2026-02-20 | Platform Orchestrator | ChatGPT round 2: 1.0.0 reserved rule, no unproven in production, explicit bump mechanics, deploy-time manifest diff check, mandatory compat lines, proof command requirement, platform compat window definition. |
| 1.1 | 2026-02-20 | Platform Orchestrator | ChatGPT round 1: added module identification section, tag immutability rule, rollback procedure, cross-module change guidance, event schema vs module versioning clarification, platform component adoption model. |
| 1.0 | 2026-02-20 | Platform Orchestrator | Created. Replaces docs/architecture/VERSIONING-STANDARD.md, docs/architecture/CONTRACT-VERSIONING-POLICY.md, and docs/governance/RELEASE-POLICY.md — those documents described an aspirational system that was never implemented. This document describes the system that is implemented and enforced. |

---

## The Rule

No change to a proven module reaches any product automatically. Every module change is versioned. Every product explicitly adopts the versions it runs. There are three gates between an agent's code change and a production deployment, and all three must pass.

---

## Identifying Modules

A **module** is any directory in the workspace that produces a deployable service (Docker container). Each module has:

- A **package file** containing its version: `Cargo.toml` (Rust) or `package.json` (Node)
- A **source directory** (`src/`, `db/`, `migrations/`) whose changes trigger version checks
- A **REVISIONS.md** file in the module root (only after the module is proven)

**Where modules live in this platform:**
```
modules/{name}/Cargo.toml          → product-facing modules (AR, GL, AP, etc.)
platform/{name}/Cargo.toml         → shared platform services (identity-auth, tenant-registry, etc.)
```

**Canonical module name:** The folder name under `modules/` or `platform/` (e.g., `ar`, `identity-auth`). Use this exact identifier in manifests, revision entries, image names, and git tags. Do not use alternate names (e.g., "accounts-receivable" for `ar`).

**Which file to bump:** Rust modules bump version in `Cargo.toml`. Node/TS modules (e.g., UI apps under `apps/`) bump version in `package.json`. Never bump both. Never bump the workspace root `Cargo.toml` version.

**How to tell if a module is proven:** Read the `version` field in the module's package file. If the version is `>= 1.0.0`, the module is proven and all versioning rules apply. If the version is `0.x.x`, the module is unproven — change freely.

**1.0.0 is reserved for proof.** Agents must not set a module to `1.0.0` unless proof is complete (all E2E tests pass and the `REVISIONS.md` baseline entry is created). If a module accidentally crosses `1.0.0`, treat it as proven immediately and backfill the `REVISIONS.md` baseline entry.

---

## Module Lifecycle

A module exists in one of three states.

### Unproven (v0.x.x)

The module is being built for the first time. Agents change it freely. No version bumps required. No revision entries needed. The version in `Cargo.toml` (or `package.json`) stays at `0.1.0` until the module passes all E2E tests and is declared proven.

**Rule:** Do not create a `REVISIONS.md` file for unproven modules. It adds noise during active construction.

**Rule:** Products must not deploy unproven modules to production. If a product must temporarily depend on an unproven module, the product manifest must pin to a specific git commit SHA in the notes column and document the rollback point.

### Proven (v1.0.0)

The module has passed all E2E tests and is stable. At this point:

1. Bump the version to `1.0.0` in the module's `Cargo.toml` (or `package.json`). Do not bump the workspace root `Cargo.toml` version — only the module's own package file.
2. Create `REVISIONS.md` in the module root — e.g., `modules/ar/REVISIONS.md` or `platform/identity-auth/REVISIONS.md` (use the template at `docs/templates/MODULE-REVISIONS.md`)
3. Commit and push
4. Build and push the first versioned image to the container registry (CI does this when automated; agent or orchestrator does it manually until then)
5. Tag the commit: `git tag {module-name}-v1.0.0` (the committing agent creates the tag)

From this moment forward, every change to this module requires a version bump and revision entry. No exceptions.

### Revised (v1.0.1+)

Any change to a proven module creates a new revision. The agent must:

1. Bump the version in the package file (see Version Numbering below)
2. Add a row to `REVISIONS.md` describing what changed, why, and whether it is breaking
3. Commit with the bead ID and version bump: `[bd-xxx] {module} v1.0.0 → v1.0.1: description`
4. Tag the commit: `git tag {module-name}-v1.0.1` (the committing agent creates the tag)
5. Gate 2 CI builds and pushes the new versioned image to the registry automatically when the commit lands on main

The new version is now available in the registry. No product uses it until the product team explicitly adopts it.

---

## Version Numbering

All modules follow [Semantic Versioning 2.0.0](https://semver.org/).

```
MAJOR.MINOR.PATCH
```

### When to bump PATCH (1.0.0 → 1.0.1)

Bug fixes that do not change the API surface or event schemas. Internal refactoring. Performance improvements. The module behaves the same from the outside; the fix corrects something that was wrong.

### When to bump MINOR (1.0.0 → 1.1.0)

New behavior that does not break existing consumers. Adding an optional field to a response. Adding a new endpoint. Adding a new event type. Existing API calls and event consumers continue to work unchanged.

### When to bump MAJOR (1.0.0 → 2.0.0)

Breaking changes. Removing or renaming a field. Changing a field type. Removing an endpoint. Changing event payload structure. Any change that would cause an existing consumer to fail.

**A breaking change requires extra work:**
- The revision entry must explain the migration path
- All products using this module must be notified (via agent mail or manifest CI warnings)
- Products cannot adopt the new major version without updating their code

**Upgrade ordering for breaking changes:** For additive (MINOR) changes, producers and consumers can upgrade in either order. For breaking (MAJOR) changes: deploy the producer with dual-write / backward compatibility first, then upgrade consumers, then remove old behavior after the compatibility window expires.

---

## The Three Gates

```
GATE 1                    GATE 2                  GATE 3
Pre-commit hook           CI pipeline             Product adoption
───────────────           ──────────────          ─────────────────

Agent changes a     ───>  Tests pass?       ───>  Product team
proven module             Contract tests?         decides to adopt.
                          E2E tests?
Version bumped?                                   Updates manifest.
Revision entry?           Image built             Runs product tests
                          and pushed to           against new version.
                          registry.

If NO → commit            If NO → image           If NO → product
is rejected.              never published.        stays on previous
Agent cannot              Change does not         version. Zero
proceed.                  reach registry.         impact on product.
```

### Gate 1: Pre-commit hook

When an agent commits changes to files inside a proven module's directory, the pre-commit hook checks:

1. Has the version in the package file been bumped?
2. Has a new row been added to `REVISIONS.md`?

If either check fails, the commit is rejected. The agent must bump the version and add the revision entry before committing.

**How the hook identifies a proven module:** If the module's package file has a version >= `1.0.0`, the module is proven. Modules at `0.x.x` skip this check.

**Files that trigger the check:** Any file under the module's source directory (`src/`, `db/`, `migrations/`). Changes to test files, documentation, or CI configuration do not trigger the version check. This means docs-only changes to a proven module do not require a version bump.

### Gate 2: CI pipeline (or manual build until CI is automated)

After a commit passes Gate 1 and is pushed:

1. Run the module's unit tests
2. Run contract tests (if the module has event schemas)
3. Run E2E tests that exercise this module
4. If all pass: build a Docker image tagged with the new version and push it to the container registry
5. The committing agent creates the git tag: `git tag {module-name}-v{version}`

If any test fails, no image is published. The change exists in the code but never reaches the registry.

**Today:** These steps are run by the agent or orchestrator manually. When CI automation is built, these become pipeline steps triggered on push.

### Gate 3: Product adoption

This is the gate that prevents changes from automatically reaching products.

Every product has a `MODULE-MANIFEST.md` file (see template at `docs/templates/MODULE-MANIFEST.md`). This file lists every platform module the product depends on and the specific version it has been validated against.

**The manifest is enforced at deployment time.** The product's deployment configuration (Docker Compose, Kubernetes, or any other orchestration) pulls module images at the versions specified in the manifest. Not `latest`. Not `main`. The exact version.

**To adopt a new module version, the product team must:**

1. Read the module's `REVISIONS.md` to understand what changed
2. Update the version in `MODULE-MANIFEST.md`
3. Run the product's E2E tests against the new module version
4. If tests pass, commit the manifest change with a note explaining the adoption
5. Deploy with the updated manifest

**Before every deployment:** Diff `MODULE-MANIFEST.md` against the deployment configuration and confirm every module version matches exactly. If there is a mismatch, block the deployment until reconciled. Each product must document which file(s) constitute its deployment config (e.g., `docker-compose.yml`, Helm values, ECS task definitions) so agents know what to diff against.

**If the product team does not update their manifest, they stay on the old version.** The new module version exists in the registry but has no effect on the product.

---

## Compatibility Matrix

Every module version bump requires a corresponding row in [`docs/COMPAT-MATRIX.md`](COMPAT-MATRIX.md) before the PR merges. The row maps the new version to the minimum frontend app version that supports it. The `compat-matrix-gate` CI job (`.github/workflows/compat-matrix-gate.yml`) enforces this automatically: it extracts the version string from any changed `modules/**/Cargo.toml` or `platform/**/Cargo.toml` and fails the PR if that exact version string is absent from `COMPAT-MATRIX.md`.

---

## Container Registry

All proven module images are stored in a container registry accessible to both development and production environments.

### Image naming

```
{registry}/{module-name}:{version}
```

**Examples:**
```
registry.example.com/7d-ar:1.0.0
registry.example.com/7d-ar:1.0.1
registry.example.com/7d-gl:1.0.0
registry.example.com/7d-auth:1.0.0
```

### Rules

- **Published version tags are immutable.** Once an image is pushed as `7d-ar:1.0.1`, that tag must never be overwritten with different bytes. If the tag is wrong, bump to the next version and push a new tag.
- Every version ever published stays in the registry. Old versions are not deleted.
- There is no `latest` tag in production. Products always reference explicit version numbers.
- Development environments may use `latest` for convenience, but production deployments must pin versions.
- The `latest` tag, if used, points to the most recent version. It is never used in product manifests.

### Registry setup

The specific registry (GitHub Container Registry, AWS ECR, Docker Hub, self-hosted) is a deployment decision. This standard does not mandate a specific provider. The requirement is:

1. Images are accessible from both dev and production environments
2. Images are tagged with the exact version number
3. Old versions remain available

---

## Product Manifests

Every product that uses platform modules maintains a `MODULE-MANIFEST.md` file in its root or `docs/` directory. This file is the source of truth for which module versions the product runs.

See `docs/templates/MODULE-MANIFEST.md` for the template.

### What the manifest contains

- Module name
- Pinned version (the exact version this product runs)
- Date the version was last validated
- Notes (why this version was adopted, any known issues)

### Manifest rules

- The manifest is committed to the product's repository
- Changes to the manifest require a commit explaining what was adopted and why
- CI may warn when a module in the registry is newer than the manifest's pinned version — this is informational, not blocking
- The product's deployment configuration must match the manifest. If the manifest says AR 1.0.0, the deployment pulls AR 1.0.0.

### When a module has a breaking change (major version bump)

The product cannot simply bump the version in the manifest. A breaking change means the product's code must be updated to handle the new API or event schema. The workflow is:

1. Read the module's `REVISIONS.md` for the migration path
2. Update product code to handle the new version
3. Update the manifest version
4. Run E2E tests
5. Commit code changes and manifest update together

---

## Platform Components vs Modules

Platform components (`identity-auth`, `event-bus`, `tenant-registry`, etc.) follow the same versioning rules as modules — proven, versioned, gated. The difference is in **who adopts them.**

- **Modules** (AR, GL, AP, etc.) are adopted per-product. Each product's `MODULE-MANIFEST.md` pins the module version it uses.
- **Platform components** (identity-auth, event-bus, tenant-registry) are shared infrastructure. They are adopted at the **environment level**, not per-product. When a platform component is upgraded, all products in that environment use the new version.

Platform component upgrades require backward compatibility for at least one prior version. Concretely: all HTTP endpoints and event contracts used by any currently deployed product must remain valid for at least one prior minor/major version. Deprecated endpoints and event schemas must be documented in `REVISIONS.md` with a removal timeline. If a platform component makes a breaking change, it must support both the old and new behavior until all products have migrated. This is enforced via the same REVISIONS.md process — the breaking change entry must document the compatibility window and which products are affected.

Products still list platform component versions in their manifests for documentation and traceability, but the platform team controls when the shared service is actually upgraded.

---

## What Agents Must Do

These rules are enforced via CLAUDE.md in every project. They are repeated here for completeness.

### When modifying a proven module (version >= 1.0.0)

1. **Before writing code:** Check the module's current version in its package file.
2. **Decide the bump type:** Is this a fix (PATCH), new feature (MINOR), or breaking change (MAJOR)?
3. **Bump the version** in the package file before committing.
4. **Add a revision entry** to the module's `REVISIONS.md` with: date, bead ID, what changed, why, and whether it is breaking.
5. **Commit** with the version bump and revision entry in the same commit as the code change.
6. **If the change is breaking (MAJOR):** Note in the revision entry what consumers must change. Send agent mail to the orchestrator flagging the breaking change.

### When adopting a new module version in a product

1. **Read the module's `REVISIONS.md`** to understand every change between your current pinned version and the version you are adopting.
2. **If any revision is breaking:** Update product code first.
3. **Update `MODULE-MANIFEST.md`** with the new version and validation date.
4. **Run the product's E2E tests** against the new version.
5. **Commit** the manifest change (and any code changes) with a note explaining the adoption.

### When a change spans multiple proven modules

If a single bead requires changes to more than one proven module, each module gets its own version bump and revision entry. The revision entries should cross-reference each other:

- Bump and add a revision row in each module's `REVISIONS.md`
- In each revision entry, include a **Compatibility** line: "Requires {other-module} >= {version}". This is mandatory whenever a change creates a cross-module dependency.
- Commit all module changes together so they are atomically deployed

### When rolling back a module version in a product

If a product needs to revert to an earlier module version:

1. Update `MODULE-MANIFEST.md` to the previous version
2. Add an adoption log entry with reason: "Rollback: {why}"
3. Run the product's E2E tests against the previous version
4. Commit the manifest change
5. Deploy with the reverted manifest

Rollbacks are version changes like any other — they go through the manifest and are recorded.

### When proving a module for the first time

1. Ensure all E2E tests pass.
2. Bump the version to `1.0.0` in the module's `Cargo.toml` (not the workspace root).
3. Create `REVISIONS.md` from the template.
4. Ensure the module has a repo-owned proof script at `scripts/proof_{module}.sh` (preferred) or a `README.md` "Proof" section. This must exist before `1.0.0` is committed. The proof command must be comprehensive — it is the single acceptance gate for `1.0.0`. A partial test suite does not qualify.

   **Proof script convention (`scripts/proof_{module}.sh`):**
   - Uses `./scripts/cargo-slot.sh test -p {package}` for all Rust test execution (never raw `cargo`)
   - Exits non-zero immediately on any failure (`set -euo pipefail`)
   - Prints clear pass/fail diagnostics for each gate
   - Optionally accepts `--staging <host>` to curl `/healthz` and `/api/ready` on a live instance
   - Current proof scripts: `proof_ar.sh`, `proof_control_plane.sh`, `proof_payments.sh`, `proof_tenant_registry.sh`, `proof_ttp.sh`
5. Commit with: `[bd-xxx] {module} v1.0.0: initial proof`
6. Tag the commit: `git tag {module-name}-v1.0.0`
7. Build and push the versioned image — Gate 2 CI does this automatically when the version bump is pushed to main (see Implementation Status).

---

## Implementation Status

This section states what is operational today and what is coming.

| Gate | Status | Enforcement today |
|------|--------|-------------------|
| Gate 1: Pre-commit hook | **Built and installed.** | `scripts/pre-commit-version-check.sh` — rejects commits to proven modules without version bump + REVISIONS.md entry. Installed via symlink at `.git/hooks/pre-commit`. |
| Gate 2: CI image pipeline | **Built and live.** | `scripts/versioning/detect_version_intent.sh` detects version bumps on push to main. `.github/workflows/ci.yml` jobs `gate2-detect-intent` + `gate2-image-build` then build and push immutable images. Tag format: `{version}-{git-sha7}`. No `latest`, no overwrite. |
| Gate 3: Product manifests | **Convention.** | Products maintain `MODULE-MANIFEST.md` and reference pinned versions in deployment config. No automated manifest-vs-deployment validation yet. |
| Container registry | **Not yet selected.** | Registry provider and credentials are a deployment decision. `IMAGE_REGISTRY` repository variable controls the prefix (default: `7dsolutions`). `DOCKER_USERNAME` / `DOCKER_PASSWORD` secrets are required for Gate 2 pushes. |

### Gate 2: Tag scheme

Images pushed by Gate 2 use the following tag format:

```
{registry}/{module-name}:{version}-{git-sha7}
```

**Fields:**
- `registry` — controlled by the `IMAGE_REGISTRY` repository variable (default: `7dsolutions`)
- `module-name` — canonical module name (folder name under `modules/`, `platform/`, or `apps/`)
- `version` — version from the module's package file (`Cargo.toml` or `package.json`) at the time of the push
- `git-sha7` — first 7 characters of the commit SHA

**Examples:**
```
7dsolutions/ar:1.0.1-a1b2c3d
7dsolutions/identity-auth:1.1.0-f4e5d6c
7dsolutions/tenant-control-plane-ui:0.3.0-b7c8d9e
```

**Invariants enforced by `scripts/staging/push_images.sh`:**
1. Refuses to push any image tagged `latest`
2. Refuses to overwrite an existing tag in the registry
3. Requires `--confirm` flag (prevents accidental pushes)

**Trigger condition:** Gate 2 fires on any push to `main` where `scripts/versioning/detect_version_intent.sh` detects one of:
- A version field bump in a `modules/*/Cargo.toml` or `platform/*/Cargo.toml`
- A version field bump in an `apps/*/package.json`
- Any change to a `MODULE-MANIFEST.md` file

### Promotion runner (single-command)

`scripts/versioning/promote_module.sh` executes the full promotion pipeline locally in one command.

**Usage:**

```bash
bash scripts/versioning/promote_module.sh \
  --module modules/ar \
  --version 1.0.0 \
  --bead bd-qvbg

# Or compute next version automatically:
bash scripts/versioning/promote_module.sh \
  --module modules/ar \
  --bump-type minor \
  --bead bd-qvbg \
  --push-tag
```

**What it does (in order):**

1. Validates prerequisites: proof script exists, version is not `latest`, tag does not already exist, working tree is clean.
2. Checks `REVISIONS.md` for the target version — if missing, generates a stub and exits with instructions to fill in the TODOs.
3. Runs the module's proof script (`scripts/proof_{module}.sh`) — must pass.
4. Runs REVISIONS lint to confirm all fields are filled.
5. Bumps the version in `Cargo.toml` or `package.json`.
6. Updates `deploy/staging/MODULE-MANIFEST.md` with the new version and image tag.
7. Commits both changes with a `[bead-id] module v{old} → v{new}: promote` message.
8. Creates git tag `{module}-v{version}`.
9. (If `--push-tag`) pushes the tag to origin — triggers Gate 2 CI.
10. (If `--staging-host`) runs the staging proof gate after deploy.

**Guards enforced:**

| Guard | Trigger |
|-------|---------|
| No `latest` | Refuses if target version is `latest` or matches `*:latest` |
| No overwrite | Refuses if git tag `{module}-v{version}` already exists |
| Proof required | Refuses if `scripts/proof_{module}.sh` does not exist |
| Clean worktree | Refuses if there are uncommitted changes |
| REVISIONS complete | Refuses if REVISIONS.md row has unfilled TODO placeholders |

**Dry-run mode** — use `--dry-run` to preview all steps without making any changes.

---

## Creating a Product's Initial Manifest

When a product is being built for the first time:

1. Identify every platform module and component the product calls (HTTP or NATS).
2. Copy the template from `docs/templates/MODULE-MANIFEST.md` into the product's repo.
3. For each dependency, list the current proven version (check the module's `Cargo.toml` or `package.json`).
4. Set "Last Validated" to the date the product's E2E tests pass against those versions.
5. Commit the manifest as part of the product's initial proof.

The manifest starts as a snapshot of what the product was built against. From that point forward, it is maintained as described in the Product Manifests section above.

---

## Superseded Documents

This document replaces three earlier documents that described an aspirational system:

- `docs/architecture/VERSIONING-STANDARD.md` — described npm-based versioning, API URL prefixes, pre-release versions. Not implemented.
- `docs/architecture/CONTRACT-VERSIONING-POLICY.md` — described event schema versioning (partially implemented) and dual-publish/dual-consume (not implemented).
- `docs/governance/RELEASE-POLICY.md` — described Kubernetes deployments, biweekly release cadence, approval matrices. Not implemented.

The event schema versioning concepts from `CONTRACT-VERSIONING-POLICY.md` (v1.json file naming, source_version in EventEnvelope, contract tests) are valid and remain in effect. The rest of those documents is superseded by this one.

---

## Event Schema Versioning vs Module Versioning

These are two independent version axes. Do not confuse them.

- **Module version** (SemVer in `Cargo.toml`): Tracks the module's code. Bumped on every change to a proven module. Recorded in `REVISIONS.md`.
- **Event schema version** (`source_version` in EventEnvelope, e.g., `"v1"`): Tracks the shape of event payloads. Changes only when the event contract changes.

**Rules:**

- A module PATCH or MINOR bump does not change the event schema version. Internal fixes and additive features keep `source_version` at its current value.
- A module MAJOR bump that changes event payload structure requires a new schema version: create `contracts/events/{event}.v2.json`, update `source_version` to `"v2"`.
- When a new schema version is introduced, the module must continue producing/consuming the previous schema version until all known consumers have migrated. The `REVISIONS.md` entry must document this compatibility window.
- Contract tests must pass for all supported schema versions.

---

> This is the single source of truth for module versioning and release gating across the platform.
