# 7D Solutions Platform

**Enterprise-grade modular platform for building vertical business applications**

## Overview

The 7D Solutions Platform is a multi-product software factory built on a three-tier architecture that separates runtime capabilities (platform), reusable business components (modules), and composed applications (products).

**Organizational Model:**
- **Company:** 7D Solutions
- **Repository:** 7D Solutions Platform (the platform itself)
- **Products:** Fireproof ERP and future vertical applications built FROM the platform

## Architecture

### Three-Tier Model

```
┌─────────────────────────────────────────────────────┐
│ TIER 3: PRODUCTS (Assembly Layer)                   │
│ products/fireproof-erp/                             │
│ - Configuration + composition ONLY                  │
│ - NO business logic                                 │
│ - Wires modules together                            │
└─────────────────────────────────────────────────────┘
                        ↓ depends on
┌─────────────────────────────────────────────────────┐
│ TIER 2: MODULES (Business Components)               │
│ modules/{ar, subscriptions, payments,               │
│          notifications, qms, inventory, audit, ...} │
│ - Independently versioned (SemVer)                  │
│ - No cross-module imports                           │
│ - Contract-driven integration                       │
│ - Product-agnostic primitives                       │
│                                                      │
│ Note: End-to-end capabilities (like billing) are    │
│ composed in products from primitive modules.        │
│ No "god modules" - keep primitives separate.        │
└─────────────────────────────────────────────────────┘
                        ↓ depends on
┌─────────────────────────────────────────────────────┐
│ TIER 1: PLATFORM (Core Runtime)                     │
│ platform/{identity, orchestration, events, ...}     │
│ - Identity & authentication                         │
│ - Event bus + scheduling/dispatch (runtime only)    │
│ - Bootstrapping & observability                     │
│ - NO product-specific logic                         │
│                                                      │
│ Note: Cross-module business workflows are composed  │
│ at the product layer using contracts/events         │
│ (choreography), not managed by a centralized        │
│ platform engine.                                     │
└─────────────────────────────────────────────────────┘
```

### Directory Structure

```
7D-Solutions-Platform/
├── platform/           # TIER 1: Core runtime capabilities
│   ├── identity/       # Auth, RBAC, multi-tenancy
│   ├── orchestration/  # Scheduler + job dispatch (runtime only; no cross-module business workflows)
│   ├── events/         # Event bus, message broker
│   ├── bootstrap/      # System initialization
│   └── observability/  # Metrics, logging, tracing
│
├── modules/            # TIER 2: Reusable business components
│   ├── ar/             # Accounts receivable (invoicing, aging)
│   ├── subscriptions/  # Recurring billing, plan management
│   ├── payments/       # Payment processing, gateway integration
│   ├── notifications/  # Email, SMS, webhooks
│   ├── qms/            # Quality management system
│   ├── inventory/      # Stock tracking, warehousing
│   ├── document-control/ # Document management
│   └── audit/          # Audit trails, compliance
│
├── products/           # TIER 3: Composed applications
│   └── fireproof-erp/  # Manufacturing ERP product
│       ├── config/     # Product-specific configuration
│       └── compose/    # Module assembly definitions
│
├── contracts/          # Source of truth for integration
│   ├── api/            # OpenAPI 3.x specifications
│   ├── events/         # AsyncAPI event schemas
│   └── schemas/        # JSON schemas, Protobuf
│
├── packages/           # Strict shared libraries
│   ├── types/          # TypeScript type definitions
│   └── utils/          # ONLY if used by 2+ modules
│
├── infra/              # Infrastructure as code
│   ├── docker/         # Compose files, Dockerfiles
│   ├── k8s/            # Kubernetes manifests
│   └── terraform/      # Cloud provisioning
│
└── tools/              # Development & CI tooling
    ├── ci/             # GitHub Actions, build scripts
    ├── scripts/        # Automation utilities
    └── generators/     # Code generation templates
```

## Core Principles

1. **Platform ≠ Product** - The platform is a reusable foundation, not a finished application
2. **Independent Versioning** - Modules follow SemVer: `component/vX.Y.Z`
3. **No Business Logic in Products** - Products are assembly layers only
4. **Contract-Driven Integration** - No source imports between modules
5. **Composed Capabilities** - End-to-end features (like billing) are assembled in products from primitive modules; no "god modules"
6. **No Junk Folders** - Eliminate `utils/`, `common/`, `shared/` directories
7. **Strict Layering** - Within modules: domain → repos → services → routes
8. **Reusability Test** - If a module can't be reused in a different product, it's not a proper module

## Prohibited Patterns

- ❌ Cross-module source imports (use contracts instead)
- ❌ Business logic in `products/` (assembly only)
- ❌ Product-specific logic in modules (keep generic)
- ❌ "God modules" that combine AR + Payments + Subscriptions; keep primitives separate and compose at the product layer
- ❌ Global utility folders (use packages/ with 2+ users rule)
- ❌ Breaking API changes without MAJOR version bump
- ❌ Single version for entire repository (each module independent)

## Quick Start

### Prerequisites

- Node.js 20+
- Docker & Docker Compose
- pnpm 9+

### Installation

```bash
# Install dependencies
pnpm install

# Build all modules
pnpm build

# Start platform services
docker compose -f infra/docker/docker-compose.platform.yml up -d

# Run specific product
cd products/fireproof-erp
pnpm dev
```

### Development Workflow

1. **Create a module:**
   ```bash
   tools/scripts/create-module.sh payments v1.0.0
   ```

2. **Define contracts:**
   ```bash
   # Create OpenAPI spec
   contracts/api/payments-v1.yaml

   # Generate types
   pnpm generate:contracts
   ```

3. **Compose a product:**
   ```bash
   # Edit product composition
   products/fireproof-erp/compose/modules.yml
   
   # Example: TrashTech billing = compose ar + subscriptions + payments + notifications

   # Wire modules
   products/fireproof-erp/config/module-config.yml
   ```

## Documentation

### Architecture Standards
- [Monorepo Standard](docs/architecture/MONOREPO-STANDARD.md) - Repository organization rules
- [Module Standard](docs/architecture/MODULE-STANDARD.md) - Module structure and boundaries
- [Contract Standard](docs/architecture/CONTRACT-STANDARD.md) - API and event schema guidelines
- [Versioning Standard](docs/architecture/VERSIONING-STANDARD.md) - Semantic versioning policies
- [Layering Rules](docs/architecture/LAYERING-RULES.md) - Dependency management
- [CI Guardrails](docs/architecture/CI-GUARDRAILS.md) - Automated enforcement
- [ADR Template](docs/architecture/ADR-TEMPLATE.md) - Architecture decision records

### Governance
- [Code Ownership](docs/governance/CODE-OWNERSHIP.md) - Maintainer responsibilities
- [Change Control](docs/governance/CHANGE-CONTROL.md) - Review and approval process
- [Release Policy](docs/governance/RELEASE-POLICY.md) - Versioning and deployment

## Technology Stack

### Platform Layer
- **Identity:** Rust-based auth service (auth-rs)
- **Events:** NATS JetStream
- **Observability:** Prometheus + Grafana
- **API Gateway:** Traefik with service discovery

### Module Layer
- **Language:** TypeScript (Node.js) or Rust
- **Framework:** Express.js / Axum
- **Database:** PostgreSQL (per-module)
- **Testing:** Jest / cargo test

### Product Layer
- **Frontend:** React + TypeScript
- **Build:** Vite / Turbo
- **Deployment:** Docker Compose / Kubernetes

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development guidelines.

## License

Proprietary - Copyright © 2026 7D Solutions

## Support

- **Documentation:** https://docs.7dsolutions.com
- **Issues:** Internal issue tracker
- **Email:** support@7dsolutions.com
