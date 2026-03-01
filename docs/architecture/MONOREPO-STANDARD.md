# Monorepo Standard

**Version:** 2.0
**Status:** Active
**Last Updated:** 2026-02-28

## Overview

This document defines the organizational structure and rules for the 7D Solutions Platform monorepo. The repository is a Rust workspace housing platform crates, business modules, CLI tools, and end-to-end tests.

## Directory Structure

### Top-Level Organization

```
7D-Solutions-Platform/
├── platform/           # TIER 1: Shared infrastructure crates
├── modules/            # TIER 2: Business domain services
├── tools/              # CLI tools, CI scripts, generators
├── e2e-tests/          # Cross-module end-to-end tests
├── contracts/          # OpenAPI + AsyncAPI specs (source of truth)
├── infra/              # Infrastructure (monitoring configs)
├── docs/               # Architecture & governance documentation
├── scripts/            # Build helpers, agent tooling, automation
├── .github/workflows/  # CI/CD pipelines
├── Cargo.toml          # Workspace root
├── Cargo.lock          # Locked dependencies
└── docker-compose*.yml # Local development stack
```

### Tier Definitions

#### TIER 1: Platform Layer

**Location:** `platform/`

**Purpose:** Shared infrastructure crates used by multiple modules. These are library crates (no `main.rs`) except for `identity-auth` and `control-plane` which are services.

**Crates:**

| Crate | Purpose |
|---|---|
| `identity-auth` | Authentication, JWT, RBAC, password management (service) |
| `control-plane` | Tenant provisioning, platform billing orchestration (service) |
| `event-bus` | NATS event bus, EventEnvelope, outbox relay, DLQ |
| `tenant-registry` | Tenant CRUD, lifecycle, plan management |
| `security` | JWT middleware, RBAC enforcement, rate limiting, CORS |
| `audit` | Append-only audit log, field diffing, policy enforcement |
| `projections` | Cursor-based event replay, blue-green rebuild |
| `tax-core` | Tax computation, jurisdiction resolution |
| `health` | Standardized health/readiness probes |

**Rules:**
- NO product-specific or module-specific business logic
- Platform crates are independently versioned
- Breaking changes require MAJOR version bump

#### TIER 2: Module Layer

**Location:** `modules/`

**Purpose:** Business domain services. Each module is a standalone Rust binary (Axum + sqlx + NATS).

**Current modules:**

| Module | Domain |
|---|---|
| `ar` | Accounts Receivable (invoices, payments, credit notes, dunning, Tilled) |
| `ap` | Accounts Payable (bills, POs, vendors, payment runs) |
| `gl` | General Ledger (journals, trial balance, period close, rev rec) |
| `payments` | Payment processing and reconciliation |
| `subscriptions` | Subscription lifecycle and billing cycles |
| `notifications` | Event-driven notifications, scheduled dispatch |
| `inventory` | Stock management, reservations, FIFO, cycle counts |
| `treasury` | Bank accounts, statement import, reconciliation |
| `fixed-assets` | Asset register, depreciation, disposals |
| `consolidation` | Multi-entity financial consolidation |
| `timekeeping` | Time entries, approvals, billing integration |
| `party` | Party master (customers, vendors, contacts, addresses) |
| `integrations` | External connectors, webhook routing |
| `ttp` | Tenant-to-Platform billing, metering, service agreements |
| `maintenance` | Work orders, preventive maintenance, meters |
| `shipping-receiving` | Inbound/outbound shipments, tracking |
| `reporting` | Cross-module reporting, KPIs, aging, forecasts |
| `pdf-editor` | PDF form templates, submissions, annotation rendering |

**Rules:**
- Each module is independently versioned (in `Cargo.toml`)
- NO cross-module source imports (CI-enforced)
- Communication via NATS events or HTTP API calls only
- Each module owns its own Postgres database
- See [Module Standard](MODULE-STANDARD.md) for internal layout

#### Tools

**Location:** `tools/`

**Purpose:** CLI tools and CI automation.

**Crates:**

| Tool | Purpose |
|---|---|
| `contract-tests` | Validates OpenAPI and event schema contracts |
| `simulation` | Failure injection and load simulation |
| `tenantctl` | Tenant management CLI (bulk ops, fleet health) |
| `projection-rebuild` | Blue-green projection rebuild runner |
| `compliance-export` | Evidence pack generation for audits |
| `demo-seed` | Deterministic demo data seeder |
| `stabilization-gate` | Pre-release quality gate (benchmarks, recon, projections) |

**CI lint scripts** (`tools/ci/`):

| Script | Enforces |
|---|---|
| `lint-no-cross-module-imports.sh` | No `use` imports across module boundaries |
| `lint-no-raw-db-connect.sh` | No direct database connections outside `db/` layer |
| `lint-event-metadata-present.sh` | Constitutional metadata on all events |
| `lint-no-ignored-tests.sh` | No `#[ignore]` tests without justification |
| `check-contract-versions.sh` | Contract version consistency |
| `check-changelog-updated.sh` | Changelog entries for changes |
| `validate-contract-examples.sh` | Contract examples match schemas |

#### End-to-End Tests

**Location:** `e2e-tests/`

**Purpose:** Cross-module integration tests that exercise full workflows across multiple services. Uses real Postgres — no mocks, no stubs.

#### Contracts

**Location:** `contracts/`

**Purpose:** Source of truth for all API and event schemas. Organized by module.

```
contracts/
├── api/                 # Shared API definitions
├── ar/                  # AR module contracts
├── gl/                  # GL module contracts
├── payments/            # Payments contracts
├── inventory/           # Inventory contracts
├── ...                  # One directory per module
└── README.md
```

#### Infrastructure

**Location:** `infra/`

**Purpose:** Monitoring and infrastructure configuration.

```
infra/
└── monitoring/          # Prometheus/Grafana configs
```

**Docker Compose files** (at repo root):

| File | Purpose |
|---|---|
| `docker-compose.yml` | Main orchestration (includes other files) |
| `docker-compose.data.yml` | PostgreSQL databases per module |
| `docker-compose.infrastructure.yml` | NATS, Redis, shared infra |
| `docker-compose.services.yml` | Module service containers |
| `docker-compose.platform.yml` | Platform service containers |
| `docker-compose.modules.yml` | Additional module containers |
| `docker-compose.monitoring.yml` | Prometheus, Grafana |

### Supporting Directories

#### Scripts (`scripts/`)

Build helpers and automation. Key scripts:

| Script | Purpose |
|---|---|
| `cargo-slot.sh` | Build slot system (4 parallel build dirs) — **always use instead of `cargo` directly** |
| `pre-commit-version-check.sh` | Pre-commit hook enforcing version bumps for proven modules |
| `agent-runner.sh` | Autonomous agent lifecycle runner |
| `agent-mail-helper.sh` | Inter-agent communication |

#### Documentation (`docs/`)

Architecture decisions, governance, and standards:

```
docs/
├── architecture/        # Standards (this file), vision docs, taxonomies
├── governance/          # Domain ownership, authority matrices
├── frontend/            # Frontend standards (separate repos build UIs)
├── plans/               # Bead plan flowcharts (.drawio)
├── consumer-guide/      # Integration guide for downstream consumers
└── VERSIONING.md        # Module versioning standard
```

## Naming Conventions

### Directories

- Use kebab-case: `fixed-assets/`, `shipping-receiving/`
- Short domain names are fine: `ar/`, `ap/`, `gl/`, `ttp/`
- Crate names match directory names in `Cargo.toml`

### Crate Names

Some crates use a `-rs` suffix to avoid conflicts with Rust reserved names:

| Directory | Crate name |
|---|---|
| `modules/party` | `party-rs` |
| `modules/integrations` | `integrations-rs` |
| `modules/inventory` | `inventory-rs` |

### Versions

- Version in `Cargo.toml`: `version = "1.0.20"`
- Git tags: `ar-v1.0.20`
- Docker images: `ghcr.io/7d-solutions/ar:1.0.20`

## Dependency Rules

### Allowed Dependencies

```
e2e-tests → modules (via HTTP/NATS, not source)
modules   → platform crates (Cargo dependency)
tools     → platform crates (Cargo dependency)
```

### Prohibited Dependencies

```
platform → modules     # Platform must be generic
modules  → modules     # No source imports (use events/HTTP)
```

Note: modules depend on `platform/event-bus`, `platform/security`, etc. as Cargo crate dependencies. This is correct — platform crates are shared libraries. The prohibition is on module-to-module source imports.

## Anti-Patterns to Avoid

### 1. Direct Cargo Calls

Always use `./scripts/cargo-slot.sh` instead of `cargo` directly. The slot system prevents build lock contention when multiple agents work in parallel.

### 2. Circular Dependencies

Modules MUST NOT depend on each other via Cargo. Communication is via NATS events (asynchronous) or HTTP calls (synchronous).

### 3. Business Logic in Platform

Platform crates provide infrastructure. Business rules belong in modules.

### 4. Mocks in Integration Tests

All tests use real Postgres. No mock databases, no stub services, no test doubles.

## Enforcement

### CI Checks (`.github/workflows/`)

| Workflow | Purpose |
|---|---|
| `ci.yml` | Build, lint, test (all workspace crates) |
| `contract-validation.yml` | Contract schema validation |
| `hardening.yml` | Security audit, `deny(unsafe_code)`, dependency check |
| `nightly.yml` | Extended test suite |
| `perf.yml` | Performance benchmarks |
| `scale.yml` | Multi-tenant scale tests |
| `promote.yml` | Promotion gate (pre-release checks) |
| `release.yml` | Release automation |

### Pre-commit Hooks

```bash
scripts/pre-commit-version-check.sh   # Enforces version bump for proven modules
```

## See Also

- [Module Standard](MODULE-STANDARD.md) — Internal module structure
- [Contract Standard](CONTRACT-STANDARD.md) — API/event schema guidelines
- [Versioning Standard](../VERSIONING.md) — Module versioning, three gates, proven/unproven rules
- [Test Standard](TEST-STANDARD.md) — Testing conventions
