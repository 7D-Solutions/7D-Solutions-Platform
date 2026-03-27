# 7D Solutions Platform

Backend module system for building vertical business applications. Verticals (e.g. Fireproof ERP, TrashTech) compose their products from these modules and build their own frontends in separate repos.

## Tech Stack

- **Language:** Rust
- **Web framework:** Axum
- **Database:** PostgreSQL (per-module isolation)
- **Messaging:** NATS JetStream
- **ORM/queries:** sqlx (compile-time checked)
- **Testing:** `cargo test` — integrated tests against real databases, no mocks
- **Containerization:** Docker Compose (compose files at repo root)
- **Monitoring:** Prometheus + Grafana

## Project Structure

```
├── platform/              # Tier 1 — shared runtime infrastructure
│   ├── audit/             # Append-only audit trail with field-level diffs
│   ├── control-plane/     # Tenant provisioning, platform billing orchestration
│   ├── event-bus/         # NATS JetStream wrapper, outbox relay, DLQ
│   ├── health/            # Readiness/liveness probe contract
│   ├── identity-auth/     # Authentication, RBAC, JWT, password reset
│   ├── projections/       # Cursor-based rebuild, blue-green swap
│   ├── security/          # AuthZ middleware, rate limiting, webhook verification
│   ├── tax-core/          # Tax jurisdiction resolution, local/zero providers
│   └── tenant-registry/   # Multi-tenant registry, lifecycle, plan management
│
├── modules/               # Tier 2 — business domain modules
│   ├── ap/                # Accounts payable (bills, POs, payment runs, vendors)
│   ├── ar/                # Accounts receivable (invoices, aging, dunning, Tilled integration)
│   ├── consolidation/     # Multi-entity financial consolidation, eliminations
│   ├── fixed-assets/      # Asset register, depreciation, disposals
│   ├── gl/                # General ledger (journals, trial balance, rev-rec, FX, accruals)
│   ├── integrations/      # External connectors, webhook routing, external refs
│   ├── inventory/         # Stock tracking, FIFO costing, reservations, cycle counts
│   ├── maintenance/       # Work orders, preventive maintenance plans, meters
│   ├── notifications/     # Event-driven notifications, scheduled dispatch
│   ├── party/             # Party master (customers, vendors, contacts, addresses)
│   ├── payments/          # Payment processing, reconciliation, retry logic
│   ├── pdf-editor/        # PDF template forms, annotations, submission validation
│   ├── reporting/         # Financial statements, aging reports, KPIs, forecasting
│   ├── shipping-receiving/# Inbound/outbound shipments, inventory integration
│   ├── subscriptions/     # Recurring billing, lifecycle state machine
│   ├── timekeeping/       # Time entries, approvals, billing, GL labor cost
│   ├── treasury/          # Bank accounts, bank reconciliation, cash position
│   └── ttp/               # Tenant technology platform (metering, billing, service agreements)
│
├── e2e-tests/             # Cross-module end-to-end test suite
├── tools/                 # CI scripts, compliance exports, demo seed
├── docs/                  # Architecture standards, governance, ops runbooks
│   ├── architecture/      # Module standard, layering rules, CI guardrails
│   ├── governance/        # Domain ownership, mutation classes, retention
│   └── VERSIONING.md      # Module versioning standard (SemVer, three gates)
├── docker-compose.yml     # Full stack (infra + platform + modules)
└── Cargo.toml             # Workspace root
```

## Quick Start

### Prerequisites

- Rust (stable toolchain)
- Docker & Docker Compose
- PostgreSQL 15+ (or use Docker)
- NATS Server (or use Docker)

### Development

```bash
# Start the data stack (Postgres, NATS)
docker compose -f docker-compose.data.yml up -d

# Start backend services
docker compose up -d

# Build all modules
cargo build --workspace

# Run tests for a specific module
cargo test -p ar

# Run the full e2e suite
cargo test -p e2e-tests
```

## Core Principles

1. **No cross-module source imports** — modules integrate via NATS events and HTTP contracts
2. **Independent versioning** — each module follows SemVer; proven modules (≥1.0.0) require version bumps
3. **Guard → Mutation → Outbox** — all state changes follow this atomic pattern
4. **EventEnvelope** — every event carries tenant_id, trace_id, idempotency_key, actor
5. **Per-module databases** — no shared tables between modules
6. **Real tests only** — integrated tests against real Postgres and NATS; no mocks, no stubs

## Documentation

- [Versioning & Release Gating](docs/VERSIONING.md)
- [Module Standard](docs/architecture/MODULE-STANDARD.md)
- [Monorepo Standard](docs/architecture/MONOREPO-STANDARD.md)
- [Layering Rules](docs/architecture/LAYERING-RULES.md)
- [CI Guardrails](docs/architecture/CI-GUARDRAILS.md)
- [Contract Standard](docs/architecture/CONTRACT-STANDARD.md)
- [Domain Ownership Registry](docs/governance/DOMAIN-OWNERSHIP-REGISTRY.md)
- [Mutation Classes](docs/governance/MUTATION-CLASSES.md)
- [Retention Classes](docs/governance/RETENTION-CLASSES.md)

## License

Proprietary — Copyright © 2026 7D Solutions
