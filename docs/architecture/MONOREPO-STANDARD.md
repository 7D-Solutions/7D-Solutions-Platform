# Monorepo Standard

**Version:** 1.0  
**Status:** Active  
**Last Updated:** 2026-02-11

## Overview

This document defines the organizational structure and rules for the 7D Solutions Platform monorepo. The repository houses platform runtime, reusable modules, and product compositions in a single codebase.

## Directory Structure

### Top-Level Organization

```
7D-Solutions-Platform/
├── platform/           # TIER 1: Core runtime (identity, events, orchestration)
├── modules/            # TIER 2: Business components (billing, inventory, QMS)
├── products/           # TIER 3: Composed applications (Fireproof ERP)
├── contracts/          # API/event schemas (source of truth)
├── packages/           # Shared libraries (strict 2+ users rule)
├── infra/              # Infrastructure as code
├── tools/              # CI/CD, scripts, generators
└── docs/               # Architecture & governance documentation
```

### Tier Definitions

#### TIER 1: Platform Layer

**Location:** `platform/`

**Purpose:** Core runtime capabilities available to all modules and products.

**Contents:**
- `identity/` - Authentication, RBAC, multi-tenancy
- `orchestration/` - Workflow engine, job scheduling
- `events/` - Event bus, message broker (NATS)
- `bootstrap/` - System initialization, health checks
- `observability/` - Metrics, logging, tracing

**Rules:**
- NO product-specific logic
- NO module-specific logic
- Platform services are versioned independently
- Breaking changes require MAJOR version bump

#### TIER 2: Module Layer

**Location:** `modules/`

**Purpose:** Reusable business components that implement domain logic.

**Structure:**
```
modules/
├── billing/
│   ├── domain/         # Business logic, entities
│   ├── repos/          # Data access
│   ├── services/       # Application services
│   ├── routes/         # HTTP handlers
│   ├── contracts/      # Local contract definitions
│   └── tests/
└── inventory/
    └── ... (same structure)
```

**Rules:**
- Each module is independently versioned (`billing/v2.3.1`)
- NO cross-module source imports
- Communication via contracts only (API calls, events)
- Each module MAY have its own database
- Modules MUST be reusable across products

#### TIER 3: Product Layer

**Location:** `products/`

**Purpose:** Composed applications that wire modules together.

**Structure:**
```
products/
└── fireproof-erp/
    ├── config/         # Module configuration
    ├── compose/        # Module assembly definitions
    ├── frontend/       # UI specific to this product
    └── deploy/         # Deployment configurations
```

**Rules:**
- NO business logic (assembly only)
- References modules by contract, not source
- Configuration-driven composition
- Products version independently from modules

### Supporting Directories

#### Contracts (`contracts/`)

**Purpose:** Single source of truth for all integration points.

```
contracts/
├── api/                # OpenAPI 3.x specs
│   ├── billing-v1.yaml
│   └── inventory-v2.yaml
├── events/             # AsyncAPI specs
│   ├── order-events.yaml
│   └── inventory-events.yaml
└── schemas/            # JSON Schema, Protobuf
    └── common/
```

**Rules:**
- ALL module-to-module communication defined here
- Versioned alongside module code
- Generated types MUST match contracts exactly

#### Packages (`packages/`)

**Purpose:** Strict shared libraries used by 2+ modules.

```
packages/
├── types/              # Shared TypeScript types
└── validation/         # Common validation logic (if used by 2+ modules)
```

**Rules:**
- ONLY create if used by 2+ modules
- Prefer copying code over premature abstraction
- Versioned independently
- NO "utils" or "common" junk folders

#### Infrastructure (`infra/`)

**Purpose:** Deployment and infrastructure definitions.

```
infra/
├── docker/
│   ├── docker-compose.platform.yml
│   ├── docker-compose.dev.yml
│   └── Dockerfile.module-template
├── k8s/
│   ├── platform/
│   ├── modules/
│   └── ingress/
└── terraform/
    ├── aws/
    └── gcp/
```

#### Tools (`tools/`)

**Purpose:** Development and CI/CD automation.

```
tools/
├── ci/                 # GitHub Actions workflows
├── scripts/            # Bash/Node.js automation
└── generators/         # Code generation templates
```

## Naming Conventions

### Directories

- Use kebab-case: `document-control/`, not `DocumentControl/`
- Be specific: `inventory-management/`, not `inventory/` if ambiguous
- Avoid abbreviations: `quality-management/`, not `qms/`

### Modules

- Format: `{domain}/{version}`
- Example: `billing/v2.3.1`
- Version in git tags: `billing-v2.3.1`

### Contracts

- Format: `{module}-{resource}-{version}.yaml`
- Example: `billing-invoice-v2.yaml`

## Dependency Rules

### Allowed Dependencies

```
products → modules → platform
products → contracts
modules → contracts
modules → platform
modules → packages (if 2+ users)
```

### Prohibited Dependencies

```
platform → modules     # Platform must be generic
platform → products    # Platform doesn't know about products
modules → products     # Modules must be product-agnostic
modules → modules      # No source imports (use contracts)
```

## Anti-Patterns to Avoid

### 1. Junk Folders

❌ **BAD:**
```
utils/
common/
shared/
helpers/
lib/
```

✅ **GOOD:**
```
packages/validation/   # If used by 2+ modules
# OR copy the code into each module that needs it
```

### 2. Circular Dependencies

❌ **BAD:**
```
moduleA → moduleB → moduleA
```

✅ **GOOD:**
```
moduleA → events → moduleB
(communicate via event bus)
```

### 3. Leaky Abstractions

❌ **BAD:**
```
products/fireproof-erp/
└── business-logic/    # Business logic in product layer
```

✅ **GOOD:**
```
modules/manufacturing/
└── domain/            # Business logic in module
```

### 4. Monolithic Versioning

❌ **BAD:**
```
package.json (root)
  "version": "3.2.1"   # Single version for everything
```

✅ **GOOD:**
```
modules/billing/package.json       → "version": "2.1.0"
modules/inventory/package.json     → "version": "1.5.3"
platform/identity/Cargo.toml       → version = "3.0.0"
```

## Enforcement

### CI Checks

The following checks MUST pass before merge:

1. **No circular dependencies** - `tools/ci/check-deps.sh`
2. **No cross-module imports** - `tools/ci/check-imports.sh`
3. **Contract validation** - `tools/ci/validate-contracts.sh`
4. **Layer violations** - `tools/ci/check-layers.sh`

### Pre-commit Hooks

Developers SHOULD install pre-commit hooks:

```bash
tools/scripts/install-hooks.sh
```

These enforce:
- Naming conventions
- File structure
- Basic dependency rules

## Migration Strategy

### Moving from Old Structure

If migrating existing code:

1. **Identify layers** - Is this platform, module, or product code?
2. **Extract contracts** - Define APIs before moving code
3. **Break dependencies** - Replace source imports with contract calls
4. **Verify independently** - Each module should build/test standalone
5. **Version separately** - Assign initial versions to each module

### Example Migration

**Before:**
```
src/
├── auth/          # Mixed concerns
├── billing/       # Mixed concerns
└── inventory/     # Mixed concerns
```

**After:**
```
platform/
└── identity/      # Auth runtime

modules/
├── billing/       # Business logic
└── inventory/     # Business logic

contracts/
├── api/billing-v1.yaml
└── api/inventory-v1.yaml
```

## Questions & Answers

**Q: When should I create a new module vs. extending an existing one?**

A: Create a new module if:
- It serves a distinct business domain
- It could be reused in a different product
- It would have 500+ lines of domain logic

Extend an existing module if:
- It's a minor feature addition
- It's tightly coupled to existing logic
- It shares the same database schema

**Q: Can modules share a database?**

A: Prefer separate databases, but sharing is allowed if:
- Modules are in the same bounded context
- Database operations are behind module APIs
- Schema changes are coordinated

**Q: What if I need code from another module?**

A: Use contracts:
1. Define an API endpoint in `contracts/api/`
2. Call the endpoint via HTTP
3. OR subscribe to events via `contracts/events/`

**Q: Can products contain any code?**

A: Yes, but ONLY:
- UI components specific to the product
- Configuration and wiring logic
- Deployment scripts

NO business logic.

## See Also

- [Module Standard](MODULE-STANDARD.md) - Module structure details
- [Contract Standard](CONTRACT-STANDARD.md) - API/event schema guidelines
- [Versioning Standard](VERSIONING-STANDARD.md) - SemVer policies
- [Layering Rules](LAYERING-RULES.md) - Dependency management
