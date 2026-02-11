# Module Boundary Enforcement Policy

This document defines the enforced boundaries between modules in the 7D Solutions Platform.

## Overview

Modules are independently deployable services with strict isolation guarantees. Cross-module integration happens exclusively through:

1. **API Contracts** (defined in `/contracts/`)
2. **Event Schemas** (defined in `/contracts/events/`)

Direct dependencies between module source code, databases, or internal APIs are **forbidden**.

## Enforced Rules

### 1. No Cross-Module Source Imports

**Rule:** Module source code must not import or reference code from other modules.

**Enforcement:** CI checks via grep on `src/` directories:
- `modules/ar/src/` must not reference `platform/identity-auth/src/`
- `platform/identity-auth/src/` must not reference `modules/ar/src/`

**Rationale:** Source-level coupling prevents independent deployment and creates hidden dependencies.

### 2. No Cross-Module Cargo Path Dependencies

**Rule:** Module `Cargo.toml` files must not use `path` dependencies pointing to other modules.

**Examples of violations:**
```toml
# In modules/ar/Cargo.toml - FORBIDDEN
auth-rs = { path = "../../platform/identity-auth" }

# In platform/identity-auth/Cargo.toml - FORBIDDEN
ar-rs = { path = "../../modules/ar" }
```

**Enforcement:** CI checks via grep on `Cargo.toml` files:
- Pattern: `path = "../(modules|platform)"`
- Any match fails the build

**Rationale:** Path dependencies create compile-time coupling and prevent versioned deployment.

### 3. No Cross-Module Database Writes

**Rule:** Migrations must be module-scoped. A module's SQL migrations must not:
- Reference tables from other modules
- Create foreign keys to other module databases
- Query or join across module schemas

**Examples of violations:**
```sql
-- In AR migration - FORBIDDEN
SELECT * FROM auth_users WHERE ...;

-- In AUTH migration - FORBIDDEN
CREATE TABLE auth_sessions (
  user_id UUID REFERENCES ar_customers(id)  -- Cross-module FK
);
```

**Enforcement:** `tools/check_migration_boundaries.sh` scans SQL files:
- AUTH migrations checked for `ar_*` table references
- AR migrations checked for `auth_*` table references
- Pattern matching: `\b(module_[a-z_]+|schema module|from module\.)`

**Rationale:** Direct database coupling creates hidden dependencies and breaks independent deployability.

### 4. Integration via Contracts Only

**Approved Integration Methods:**

| Method | Location | Purpose |
|--------|----------|---------|
| REST API | `/contracts/{module}/{module}-v{N}.yaml` | Synchronous service calls |
| Events | `/contracts/events/{event-name}.v{N}.json` | Asynchronous event-driven integration |

**Example - AR to AUTH Integration:**
1. AR validates tokens by calling AUTH's public REST API (`/contracts/auth/`)
2. AR does NOT import AUTH source code
3. AR does NOT query `auth_users` table directly
4. AR does NOT use Cargo path dependency on `auth-rs`

### 5. Shared Packages (Future)

**Current Policy:** No shared packages exist. Modules duplicate minimal common code.

**Future Policy (Not Yet Implemented):**
- Shared packages may be added under `/packages/{name}/`
- Each shared package must:
  - Be versioned (semantic versioning)
  - Have comprehensive tests
  - Have API documentation
  - Be approved by architecture review

**Request Process:**
1. File issue describing shared need
2. Justify why duplication is insufficient
3. Propose package API surface
4. Obtain approval before creating package

## Enforcement Summary

| Rule | Enforced By | Frequency |
|------|-------------|-----------|
| No source imports | CI grep check | Every push/PR |
| No path dependencies | CI grep check | Every push/PR |
| No cross-DB migrations | `check_migration_boundaries.sh` | Every push/PR |
| Contract validation | CI YAML/JSON parsing | Every push/PR |

## Violation Response

**If CI detects a violation:**
1. Build fails immediately
2. Developer receives clear error with offending file/line
3. Violation must be corrected before merge

**If violation discovered post-merge:**
1. Create rollback PR immediately
2. File incident report
3. Enhance CI checks to prevent recurrence

## Workspace Note

The repository includes a root `Cargo.toml` workspace for tooling convenience.

**Important:** Modules can be built independently even if workspace build fails due to dependency version mismatches. This is intentional - modules have independent release cycles.

To build a specific module:
```bash
cd platform/identity-auth && cargo build
cd modules/ar && cargo build
```

Workspace dependency alignment is recommended but not required for deployment.

## Questions and Exceptions

**Q: Can I create a shared utility module?**
A: Not yet. Current policy is strict module independence. Propose shared package via issue if duplication becomes severe.

**Q: Module A needs data from Module B. How do I integrate?**
A: Use Module B's public REST API (defined in `/contracts/`) or subscribe to Module B's events (defined in `/contracts/events/`).

**Q: Can I call another module's internal endpoint not in the contract?**
A: No. Only endpoints documented in `/contracts/` are stable APIs. Internal endpoints may change without notice.

**Q: What if I need a database join across modules?**
A: Create a read model via event sourcing. Module A publishes events, Module B consumes and maintains a local projection. See `/contracts/events/README.md`.

## References

- Contract Standard: `/docs/architecture/CONTRACT-STANDARD.md`
- Event Transport: `/contracts/events/README.md`
- Migration Tool: `/tools/check_migration_boundaries.sh`
- CI Workflow: `/.github/workflows/ci.yml`
