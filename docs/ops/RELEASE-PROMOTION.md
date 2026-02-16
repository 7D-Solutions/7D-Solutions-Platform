# Release and Promotion Pipeline

## Current Status (Phase 18, Bead bd-18a0)

**✅ Implemented:**
- Build all workspace binaries in release mode
- Compute SHA256 checksums for all artifacts
- Create release manifest with git metadata
- Upload artifacts to GitHub Actions (90-day retention)

**⚠️ Not Yet Implemented (Awaiting Phase 17):**
- Environment promotion semantics (dev → staging → prod)
- Schema version tracking and compatibility
- Projection version tracking
- Audit version tracking
- Artifact signing/attestation
- Deployment provenance recording

## Architecture Principles

### Immutability Invariant
**The exact artifact deployed must be traceable and immutable across environments.**

This means:
- Artifacts are built ONCE (on version tag or release)
- The same binary (with verified checksum) is promoted across environments
- NO rebuilding between environments (prevents "works in staging" drift)

### Failure Mode to Avoid
**Environment drift** - where staging and prod run different code despite "same version" due to rebuilding from source with different dependencies/timestamps.

## Current Workflow

### 1. Build Release Artifacts

Triggered on version tags (e.g., `v0.1.0`) or manual workflow dispatch:

```bash
git tag v0.1.0
git push origin v0.1.0
```

This triggers `.github/workflows/release.yml` which:
1. Builds all workspace binaries: `cargo build --release --workspace --bins`
2. Collects binaries from `target/release/`
3. Computes SHA256 checksums for each binary
4. Creates `manifest.json` with:
   - Version (git tag)
   - Git SHA
   - Build timestamp
   - Artifact checksums
5. Uploads artifacts to GitHub Actions (retention: 90 days)

### 2. Verify Artifacts

Download artifacts from GitHub Actions and verify checksums:

```bash
# Download artifacts (replace <run-id> with actual run ID)
gh run download <run-id> -n release-artifacts-<sha>

# Verify checksums
for file in *.sha256; do
  sha256sum -c "$file"
done
```

### 3. Promotion (Placeholder)

`.github/workflows/promote.yml` exists but is NOT YET IMPLEMENTED.

Future implementation (post-Phase 17) will:
1. Accept target environment (dev/staging/prod) and artifact SHA
2. Download artifacts by SHA (no rebuild)
3. Verify checksums match
4. Record promotion metadata:
   - Environment
   - Module version
   - Schema version (from Phase 17)
   - Projection version (from Phase 17)
   - Audit version (from Phase 17)
   - Deployment timestamp
5. Deploy to target environment
6. Verify version compatibility (schema migrations, projection rebuilds)

## Artifacts Produced

Current binaries (as of Phase 17):

| Binary | Module | Purpose |
|--------|--------|---------|
| `ar-rs` | modules/ar | Accounts Receivable service |
| `payments-rs` | modules/payments | Payments service |
| `subscriptions-rs` | modules/subscriptions | Subscriptions service |
| `gl-rs` | modules/gl | General Ledger service |
| `notifications-rs` | modules/notifications | Notifications service |
| `identity-auth` | platform/identity-auth | Identity & Auth service |

Each binary has:
- `<binary>` - Executable
- `<binary>.sha256` - SHA256 checksum file

Plus:
- `manifest.json` - Release metadata

## Checksum Verification

Each artifact has a `.sha256` file containing its SHA256 checksum:

```bash
# Verify single artifact
sha256sum -c ar-rs.sha256

# Verify all artifacts
sha256sum -c *.sha256
```

## Future Work (Post-Phase 17)

### 1. Version Contracts
Phase 17 will define:
- Schema versioning conventions
- Projection cursor version tracking
- Audit log version tracking

### 2. Promotion Logic
Once Phase 17 is complete, implement in `promote.yml`:
- Download artifacts by SHA (immutable)
- Verify checksums
- Check version compatibility:
  - Schema migrations required?
  - Projection rebuilds required?
  - Audit format compatible?
- Record deployment provenance
- Deploy to target environment

### 3. Deployment Metadata
Each deployment should record:
```json
{
  "environment": "prod",
  "deployed_at": "2026-02-16T10:30:00Z",
  "git_sha": "abc123...",
  "module_version": "0.1.0",
  "schema_version": "20260216_001",
  "projection_version": "v2",
  "audit_version": "v1",
  "artifacts": {
    "ar-rs": "sha256:123abc...",
    "payments-rs": "sha256:456def..."
  }
}
```

### 4. Rollback Support
With immutable artifacts and provenance:
- Rollback = promote previous artifact SHA
- No rebuild required
- Verify downgrade compatibility

## Testing

See `e2e-tests/tests/release_provenance_smoke_e2e.rs` for basic smoke test that verifies:
- Artifacts can be built
- Checksums are computed correctly
- Manifest structure is valid

## References

- `.github/workflows/release.yml` - Release artifact build
- `.github/workflows/promote.yml` - Promotion placeholder
- Phase 17 beads - Projection/audit versioning contracts
