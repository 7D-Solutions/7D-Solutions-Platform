# Module Standard

**Version:** 2.0
**Status:** Active
**Last Updated:** 2026-02-28

## Overview

This document defines the structure, boundaries, and rules for modules in the 7D Solutions Platform. Modules are independently versioned Rust crates that implement domain logic and deploy as standalone services.

## Module Definition

A **module** is:
- A self-contained business capability (AR, AP, inventory, GL)
- A Rust crate in the `modules/` directory, listed in the workspace `Cargo.toml`
- Independently versioned using SemVer (in `Cargo.toml` and optionally a `VERSION` file)
- Deployable as a standalone binary (`src/main.rs`)
- Contract-driven in its integration (NATS events, HTTP APIs)

A module is **NOT**:
- A shared library (use `platform/` crates for that)
- Platform infrastructure (use `platform/` for that)
- A CLI tool (use `tools/` for that)

## Directory Structure

### Standard Module Layout

Based on the actual pattern used across all modules (AP, AR, GL, inventory, treasury, etc.):

```
modules/{module-name}/
├── src/
│   ├── main.rs              # Binary entrypoint (Axum server, NATS setup)
│   ├── lib.rs               # Library root (re-exports)
│   ├── config.rs            # Configuration struct (env vars, ports)
│   ├── metrics.rs           # Prometheus metrics definitions
│   │
│   ├── domain/              # Business logic (pure domain)
│   │   ├── mod.rs
│   │   ├── {entity}/        # One sub-module per aggregate
│   │   │   ├── mod.rs       # Types, models, re-exports
│   │   │   ├── service.rs   # Domain service (guard → mutate → outbox)
│   │   │   └── models.rs    # Domain structs (optional, if mod.rs gets large)
│   │   └── ...
│   │
│   ├── db/                  # Database access layer
│   │   └── mod.rs           # sqlx queries, PgPool, migrations
│   │
│   ├── http/                # HTTP handlers (Axum routes)
│   │   ├── mod.rs           # Router assembly
│   │   ├── admin.rs         # Health, readiness, admin endpoints
│   │   └── {resource}.rs    # One file per resource (e.g., bills.rs, vendors.rs)
│   │
│   ├── events/              # Event definitions and consumers
│   │   ├── mod.rs           # Event types, subjects
│   │   ├── envelope.rs      # EventEnvelope construction
│   │   └── {event}.rs       # Specific event structs
│   │
│   ├── consumers/           # Inbound NATS event handlers (optional)
│   │   ├── mod.rs
│   │   └── {event_name}.rs
│   │
│   ├── outbox/              # Transactional outbox (optional)
│   │   └── mod.rs
│   │
│   └── integrations/        # Outbound calls to other modules (optional)
│       ├── mod.rs
│       └── {module}/mod.rs
│
├── tests/                   # Integration tests (real Postgres, no mocks)
│   ├── {feature}_integration.rs
│   └── ...
│
├── db/
│   └── migrations/          # SQL migration files (sqlx)
│
├── Cargo.toml               # Crate metadata + version
├── Dockerfile.workspace     # Container image (workspace-aware build)
├── README.md                # Module overview
├── VERSION                  # Plain-text version (optional, proven modules)
└── REVISIONS.md             # Version history (required for proven modules)
```

### Variations

Not every module uses every directory. Common variations:

- **Flat domain:** Smaller modules (e.g., notifications) may put domain logic directly in `src/` files instead of `src/domain/` subdirectories.
- **`ops/` directory:** Some modules (consolidation, party, ttp) use `src/ops/ready.rs` for readiness probes.
- **`services/` and `repos/`:** GL uses `src/services/` and `src/repos/` instead of `src/domain/` — this is an older pattern. New modules should use `src/domain/`.
- **`consumers/` vs inline:** Some modules handle inbound events inside `src/events/consumer.rs` rather than a separate `consumers/` directory.

## Layering Rules

Modules follow strict layering:

```
http → domain → db
  ↓       ↓       ↓
HTTP   Business  Data
API     Logic   Access
```

Events flow orthogonally: domain services publish via outbox, consumers feed into domain services.

### Layer Responsibilities

#### Domain Layer (`src/domain/`)

**Responsibilities:**
- Business logic, guard checks, state transitions
- Domain models (structs, enums, value objects)
- Business rule validation
- Event construction

**Rules:**
- NO direct HTTP concerns
- Domain services receive a `&PgPool` (or `&mut Transaction`) — DB access is co-located, not abstracted behind repository traits
- Guard functions validate preconditions before mutations

#### DB Layer (`src/db/`)

**Responsibilities:**
- SQL query execution via sqlx
- Migration management
- Shared query helpers

**Rules:**
- NO business logic
- Return domain structs, not raw rows where practical

#### HTTP Layer (`src/http/`)

**Responsibilities:**
- Axum route handlers
- Request/response types (serde)
- Input validation
- Tenant extraction from JWT claims

**Rules:**
- NO business logic (delegate to domain services)
- NO raw SQL (use domain/db layer)
- Handle HTTP concerns only

#### Events Layer (`src/events/`)

**Responsibilities:**
- Event struct definitions (serde Serialize/Deserialize)
- EventEnvelope construction with constitutional metadata
- NATS subject naming

**Rules:**
- Events are immutable value objects
- Every event carries: `tenant_id`, `trace_id`, `caused_by`, `timestamp`

## Module Boundaries

### Communication Between Modules

Modules communicate via:
1. **NATS event bus** — Asynchronous events (primary pattern)
2. **HTTP API calls** — Synchronous requests (via integration clients)

Modules MUST NOT:
- Import source code from other modules (enforced by `tools/ci/lint-no-cross-module-imports.sh`)
- Access other modules' databases directly (enforced by `tools/ci/lint-no-raw-db-connect.sh`)
- Share in-memory state

### Atomicity Pattern

All state changes follow: **Guard → Mutation → Outbox** within a single database transaction. The outbox relay publishes events to NATS after commit.

## Versioning

### SemVer for Modules

Version lives in `Cargo.toml` (and optionally a `VERSION` file for proven modules):

```toml
[package]
name = "ar"
version = "1.0.20"
```

- **MAJOR:** Breaking API/event changes
- **MINOR:** New features (backward compatible)
- **PATCH:** Bug fixes

See [Versioning Standard](../VERSIONING.md) for full rules including the proven/unproven distinction.

### Version in Files

**Cargo.toml** (source of truth):
```toml
version = "1.0.20"
```

**VERSION file** (proven modules):
```
1.0.20
```

**Docker image:**
```bash
ghcr.io/7d-solutions/ar:1.0.20
```

## Documentation Requirements

Every module MUST have the following documentation artifacts. These are not optional — a module without registry entries is invisible to the platform.

### At Module Creation (scaffold bead)

| Artifact | Location | Purpose |
|---|---|---|
| **Module Authority Matrix entry** | `docs/architecture/MODULE-AUTHORITY-MATRIX.md` | Declares what this module owns, may mutate, produces, and consumes |
| **Domain Ownership Registry entry** | `docs/governance/DOMAIN-OWNERSHIP-REGISTRY.md` | Declares tables, responsibilities, and external dependencies |
| **Event Taxonomy ownership line** | `docs/architecture/EVENT-TAXONOMY.md` | Declares `{domain}.*` event namespace (if module emits events) |

### Before v0.1.0 Release

| Artifact | Location | Purpose |
|---|---|---|
| **Vision/Spec document** | `docs/architecture/{MODULE}-VISION.md` or `{MODULE}-MODULE-SPEC.md` | Domain authority, state machines, integration map, roadmap, decision log |
| **Docker entries** | `docker-compose.data.yml` + `docker-compose.services.yml` | Database container and service container |

### At v1.0.0 (proven module)

| Artifact | Location | Purpose |
|---|---|---|
| **REVISIONS.md** | `modules/{name}/REVISIONS.md` | Version history with migration notes (required for every version bump) |

### Validation

The scaffold bead for any new module MUST include creating the three registry entries as acceptance criteria. Code review should verify these entries exist before closing the scaffold bead.

---

## Testing Standards

### Rules

- All integration tests use **real Postgres** — no mocks, no stubs, no test doubles
- Tests live in `tests/` as separate integration test files (Rust convention)
- Each test creates a unique `tenant_id` for isolation
- Tests run with `serial_test` where needed to avoid port/state conflicts
- Use `./scripts/cargo-slot.sh test -p {crate}` to run tests (never `cargo test` directly)

### Test File Naming

- `{feature}_integration.rs` — standard pattern
- `{feature}_e2e.rs` — end-to-end tests involving multiple concerns

## See Also

- [Monorepo Standard](MONOREPO-STANDARD.md) — Repository organization
- [Contract Standard](CONTRACT-STANDARD.md) — API/event schemas
- [Versioning Standard](../VERSIONING.md) — Module versioning, three gates, proven/unproven rules
- [Test Standard](TEST-STANDARD.md) — Testing conventions
