# Private Crate Registry

Platform crates are published to a private GitHub Packages registry so that downstream repos (Fireproof-ERP, future consumers) can depend on versioned releases instead of path dependencies.

## Registry Details

| Field | Value |
|-------|-------|
| Registry name | `7d-platform` |
| Index URL | `sparse+https://cargo.pkg.github.com/7D-Solutions/index/` |
| Auth | GitHub PAT with `read:packages` (consumers) or `write:packages` (CI publish) |

## Published Crates

All crates under `platform/` are published: event-bus, platform-contracts, audit, projections, security, tenant-registry, control-plane, identity-auth, doc-mgmt, health, tax-core.

Module crates (`modules/`) are **not** published — they are deployable services, not libraries.

## Consumer Setup

### 1. Add the registry to `.cargo/config.toml`

```toml
[registries.7d-platform]
index = "sparse+https://cargo.pkg.github.com/7D-Solutions/index/"
```

### 2. Set your auth token

Create a GitHub Personal Access Token (classic) with `read:packages` scope. Then either:

**Option A — Environment variable (recommended for CI):**
```bash
export CARGO_REGISTRIES_7D_PLATFORM_TOKEN=ghp_your_token_here
```

**Option B — Cargo credentials file (`~/.cargo/credentials.toml`):**
```toml
[registries.7d-platform]
token = "ghp_your_token_here"
```

### 3. Add dependencies in your `Cargo.toml`

```toml
[dependencies]
event-bus = { version = "0.1", registry = "7d-platform" }
platform_contracts = { version = "0.1", registry = "7d-platform" }
```

### 4. Pin versions

Use exact or compatible version constraints. Since all platform crates are currently v0.x, any minor bump may contain breaking changes per SemVer convention:

```toml
# Exact pin (safest for v0.x crates)
event-bus = { version = "=0.1.0", registry = "7d-platform" }

# Compatible range (once crates reach 1.0+)
event-bus = { version = "1.0", registry = "7d-platform" }
```

## CI Publishing

Crates are published automatically by the `gate2-crate-publish` job in `.github/workflows/ci.yml`. This runs on push to main when a version-intent change is detected (same trigger as Gate 2 image builds).

The publish script (`scripts/ci/publish-platform-crates.sh`) publishes crates in dependency order and skips versions that already exist in the registry — publishes are immutable.

### Required CI Secret

`CRATE_REGISTRY_TOKEN`: A GitHub PAT with `write:packages` scope, stored as a repository secret.

## Immutability

Once a version is published, it cannot be overwritten. To ship a fix, bump the version in `Cargo.toml` (following the rules in `docs/VERSIONING.md`) and merge to main. The next CI run will publish the new version.
