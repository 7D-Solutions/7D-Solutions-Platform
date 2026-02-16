-- Add explicit constraints to scaffolding beads for autonomous execution

-- Add constraint to bd-18a0 (Release pipeline)
UPDATE issues
SET description = description || '

## ⚠️ SCOPE CONSTRAINT (Autonomous Execution)
**CRITICAL:** This bead is scope-locked to prevent rework after Phase 17.

**ALLOWED:**
- Build all workspace crates
- Produce artifacts with checksums (SHA256)
- Create manifest file

**NOT ALLOWED (will cause rework):**
- Environment promotion semantics
- Signing/attestation logic
- Schema/projection version conventions
- Any logic that depends on Phase 17 design decisions

Keep implementation to basic artifact build + checksum. Full promotion pipeline will be added after Phase 17 defines versioning conventions.'
WHERE id = 'bd-18a0';

-- Add constraint to all scaffolding beads
UPDATE issues
SET description = description || '

## ⚠️ SCAFFOLDING CONSTRAINTS (Autonomous Execution)
**CRITICAL:** This is scaffolding ONLY. Do not implement business logic.

**ALLOWED:**
- Create crate directory structure
- Add Cargo.toml with minimal dependencies
- Add src/lib.rs (or src/main.rs for tools) with placeholder functions
- Update root workspace Cargo.toml members
- Ensure cargo test --workspace passes

**NOT ALLOWED:**
- Business logic implementation
- Database migrations or schemas
- HTTP endpoints or routes
- Cross-module imports (AR, Payments, GL, etc.)
- Heavy dependencies or feature flags

This bead prepares the crate structure. Implementation comes in subsequent beads.'
WHERE id IN ('bd-17s0', 'bd-17s1', 'bd-17s2', 'bd-17s3', 'bd-17s4', 'bd-18s0', 'bd-18s1');
