# Module Versioning & Release Gating Standard

> **Who reads this:** All Claude Code agents — platform agents modifying modules, product agents adopting module versions.
> **What it covers:** How modules are versioned, how changes are gated, and how products control which module versions they run.
> **Parent:** This is a standalone platform standard. Referenced from CLAUDE.md in every project that uses platform modules.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Created. Replaces docs/architecture/VERSIONING-STANDARD.md, docs/architecture/CONTRACT-VERSIONING-POLICY.md, and docs/governance/RELEASE-POLICY.md — those documents described an aspirational system that was never implemented. This document describes the system that is implemented and enforced. |

---

## The Rule

No change to a proven module reaches any product automatically. Every module change is versioned. Every product explicitly adopts the versions it runs. There are three gates between an agent's code change and a production deployment, and all three must pass.

---

## Module Lifecycle

A module exists in one of three states.

### Unproven (v0.x.x)

The module is being built for the first time. Agents change it freely. No version bumps required. No revision entries needed. The version in `Cargo.toml` (or `package.json`) stays at `0.1.0` until the module passes all E2E tests and is declared proven.

**Rule:** Do not create a `REVISIONS.md` file for unproven modules. It adds noise during active construction.

### Proven (v1.0.0)

The module has passed all E2E tests and is stable. At this point:

1. Bump the version to `1.0.0` in the package file
2. Create `REVISIONS.md` in the module root (use the template at `docs/templates/MODULE-REVISIONS.md`)
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
5. CI builds and pushes the new versioned image to the registry (manual until CI is automated — see Implementation Status)

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

**Files that trigger the check:** Any file under the module's source directory (`src/`, `db/`, `migrations/`). Changes to test files, documentation, or CI configuration do not trigger the version check.

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

**If the product team does not update their manifest, they stay on the old version.** The new module version exists in the registry but has no effect on the product.

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

Platform components (`identity-auth`, `event-bus`, `tenant-registry`, etc.) follow the same versioning rules as modules. They are proven, versioned, and gated in the same way. Products include platform component versions in their manifests.

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

### When proving a module for the first time

1. Ensure all E2E tests pass.
2. Bump the version to `1.0.0`.
3. Create `REVISIONS.md` from the template.
4. Commit with: `[bd-xxx] {module} v1.0.0: initial proof`
5. Tag the commit: `git tag {module-name}-v1.0.0`
6. Build and push the versioned image (CI does this when automated; manual until then — see Implementation Status).

---

## Implementation Status

Not all gates are mechanically enforced yet. This section states what is operational today and what is coming. Agents must follow the rules regardless — CLAUDE.md enforcement applies even before hooks exist.

| Gate | Status | Enforcement today |
|------|--------|-------------------|
| Gate 1: Pre-commit hook | **Not yet built.** | CLAUDE.md rules. Agents must self-enforce version bumps and revision entries. The hook will be added to mechanically prevent forgotten bumps. |
| Gate 2: CI image pipeline | **Not yet built.** | Manual image build and push. Agent or orchestrator runs `docker build` and `docker push` after tests pass. CI automation will be added. |
| Gate 3: Product manifests | **Convention.** | Products maintain `MODULE-MANIFEST.md` and reference pinned versions in deployment config. No automated manifest-vs-deployment validation yet. |
| Container registry | **Not yet selected.** | Registry provider and credentials are a deployment decision. Until selected, images are built locally with version tags. |

**Until Gate 1 is automated:** Agents are responsible for checking the module version before committing. If you change a file in a proven module and forget to bump, no hook will stop you — but you have violated the standard and created a deployment risk.

**Until Gate 2 is automated:** After committing a version bump to a proven module, the agent or orchestrator must manually build and tag the Docker image. The commit message should note the version bump so the image build is not forgotten.

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

> This is the single source of truth for module versioning and release gating across the platform.
