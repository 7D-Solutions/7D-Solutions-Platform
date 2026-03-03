# Fireproof Go-Live Release Process

## Tagging Scheme

Release tags follow this pattern:

```
{release-name}-v{MAJOR.MINOR.PATCH}
```

**Current release tag:** `fireproof-go-live-v1.0.0`

The tag is annotated (not lightweight) and points to the exact commit from which all module images are built. The manifest JSON at `docs/releases/fireproof-go-live-manifest.json` records the SHA, every crate version, and the corresponding image tag.

### Tag naming rules

- The release tag is repo-level — it pins the entire platform state, not individual modules.
- Individual module tags (e.g., `ar-v1.0.56`) continue to follow the per-module scheme from `docs/VERSIONING.md`.
- Subsequent go-live patches use `fireproof-go-live-v1.0.1`, `v1.0.2`, etc.
- A new minor release (e.g., added modules) uses `fireproof-go-live-v1.1.0`.

## Manifest Generation

The manifest is deterministic: same commit always produces the same file.

```bash
bash scripts/generate-release-manifest.sh
```

Options:
- `--release-name NAME` — defaults to `fireproof-go-live`
- `--out FILE` — defaults to `docs/releases/{release-name}-manifest.json`

The script reads every `Cargo.toml` under `modules/` and `platform/`, records the version, proven status, and computes the image tag using the Gate 2 convention (`{registry}/{name}:{version}-{sha7}`).

## Creating a Release

```bash
# 1. Ensure working tree is clean
git status

# 2. Generate the manifest (overwrites existing)
bash scripts/generate-release-manifest.sh

# 3. Commit the manifest
git add docs/releases/fireproof-go-live-manifest.json
git commit -m "[bd-xxx] Fireproof go-live v1.0.0 release manifest"

# 4. Create the annotated tag
git tag -a fireproof-go-live-v1.0.0 -m "Fireproof go-live release v1.0.0

Pins all 33 platform crates at the versions in docs/releases/fireproof-go-live-manifest.json.
Proven modules: ar (1.0.56), payments (1.1.14), ttp (2.1.5),
  control-plane (1.0.3), identity-auth (1.3.10), tenant-registry (1.0.3)."

# 5. Verify
git show fireproof-go-live-v1.0.0
jq . docs/releases/fireproof-go-live-manifest.json
```

## Rollback Procedure

Rollback means returning to a known-good release state. The manifest and tag make this reproducible.

### Minimal rollback (single module)

If one module is faulty but the rest of the release is fine:

1. Identify the previous good version from the module's `REVISIONS.md`.
2. Update the product's `MODULE-MANIFEST.md` to pin the previous version.
3. Redeploy that single service using the previous image tag from the registry.
4. Commit the manifest change with reason: `Rollback: {description}`.

### Full rollback (entire release)

If the whole release must be reverted:

1. Check out the previous release tag:
   ```bash
   git checkout fireproof-go-live-v1.0.0  # or whatever the last-known-good tag is
   ```
2. Read `docs/releases/fireproof-go-live-manifest.json` at that tag to get all image tags.
3. Redeploy all services using those exact image tags.
4. On `main`, create a new manifest pointing to the rolled-back versions and tag it as the next patch:
   ```bash
   # After deploying the rollback:
   git tag -a fireproof-go-live-v1.0.1 -m "Rollback to pre-{issue} state"
   ```

### Rollback order

When rolling back multiple services, reverse the deployment order:

1. **Modules first** (AR, Payments, TTP, etc.) — these are leaf services.
2. **Platform components second** (identity-auth, control-plane, tenant-registry) — these are shared infrastructure.
3. **Event bus last** — rolling back the event bus affects all producers and consumers.

This order minimizes the window where incompatible versions are running together.

### Rollback target tags

The `rollback_tag` field in the manifest JSON is `null` for the initial release (no previous version exists). After the first release, subsequent manifests should set `rollback_tag` to the previous release tag name, e.g.:

```json
"rollback_tag": "fireproof-go-live-v1.0.0"
```

This makes the rollback target explicit and machine-readable.

## Answering "What exact code is running?"

With this system, the answer is always:

1. Read the release tag: `git show fireproof-go-live-v1.0.0`
2. Read the manifest at that tag: `git show fireproof-go-live-v1.0.0:docs/releases/fireproof-go-live-manifest.json`
3. Every crate name, version, and image tag is listed.
