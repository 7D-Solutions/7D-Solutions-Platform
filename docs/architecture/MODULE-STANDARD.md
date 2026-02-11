# Module Standard

**Version:** 1.0  
**Status:** Active  
**Last Updated:** 2026-02-11

## Overview

This document defines the structure, boundaries, and rules for modules in the 7D Solutions Platform. Modules are independently versioned, reusable business components that implement domain logic.

## Module Definition

A **module** is:
- A self-contained business capability (billing, inventory, QMS)
- Independently versioned using SemVer
- Deployable as a standalone service
- Reusable across multiple products
- Contract-driven in its integration

A module is **NOT**:
- A utility library (use `packages/` for that)
- Product-specific logic (use `products/` for that)
- Platform infrastructure (use `platform/` for that)

## Directory Structure

### Standard Module Layout

```
modules/{module-name}/
├── domain/                 # Business logic (pure domain)
│   ├── entities/           # Domain entities, value objects
│   ├── services/           # Domain services
│   └── events/             # Domain events
│
├── repos/                  # Data access layer
│   ├── prisma/             # Prisma schema (if using Prisma)
│   ├── migrations/         # Database migrations
│   └── repositories/       # Repository implementations
│
├── services/               # Application services
│   ├── commands/           # Command handlers (writes)
│   ├── queries/            # Query handlers (reads)
│   └── workflows/          # Multi-step business workflows
│
├── routes/                 # HTTP handlers
│   ├── v1/                 # API version 1
│   └── v2/                 # API version 2 (if applicable)
│
├── contracts/              # Module's contract definitions
│   ├── openapi.yaml        # REST API spec
│   └── events.yaml         # Event schemas (AsyncAPI)
│
├── config/                 # Configuration
│   ├── default.yml         # Default configuration
│   └── schema.json         # Config validation schema
│
├── tests/
│   ├── unit/               # Unit tests (domain, services)
│   ├── integration/        # Integration tests (repos, API)
│   └── contract/           # Contract tests
│
├── docs/
│   ├── README.md           # Module overview
│   ├── API.md              # API documentation
│   └── CHANGELOG.md        # Version history
│
├── Dockerfile              # Container image definition
├── package.json            # Node.js dependencies + version
├── tsconfig.json           # TypeScript configuration
└── .env.example            # Environment variable template
```

## Layering Rules

Modules MUST follow strict layering:

```
routes → services → repos → domain
  ↓        ↓         ↓        ↓
HTTP   Orchestration  Data   Pure
API    Workflows    Access  Business
                             Logic
```

### Layer Responsibilities

#### Domain Layer

**Responsibilities:**
- Pure business logic
- Domain entities and value objects
- Business rule validation
- Domain events

**Rules:**
- NO external dependencies (DB, HTTP, etc.)
- NO framework dependencies
- Should be testable without infrastructure

#### Repos Layer

**Responsibilities:**
- Database access
- ORM mapping
- Query execution
- Transaction management

**Rules:**
- NO business logic
- Return domain entities, not DB models
- Abstract away DB implementation details

#### Services Layer

**Responsibilities:**
- Orchestrate domain objects
- Coordinate multiple repositories
- Handle transactions
- Publish events

**Rules:**
- NO HTTP concerns (use routes layer)
- NO DB concerns (use repos layer)
- Orchestration only

#### Routes Layer

**Responsibilities:**
- HTTP request/response handling
- Input validation
- Authentication/authorization
- Error handling

**Rules:**
- NO business logic (delegate to services)
- NO direct DB access (use services)
- Handle HTTP concerns only

## Module Boundaries

### Communication Between Modules

Modules MUST communicate via:
1. **REST API calls** - Synchronous requests
2. **Event bus** - Asynchronous events
3. **Shared database** (only if tightly coupled)

Modules MUST NOT:
- Import source code from other modules
- Access other modules' databases directly
- Share in-memory state

## Versioning

### SemVer for Modules

Format: `{module}/v{MAJOR}.{MINOR}.{PATCH}`

- **MAJOR:** Breaking API changes
- **MINOR:** New features (backward compatible)
- **PATCH:** Bug fixes

Examples:
- `billing/v1.0.0` - Initial release
- `billing/v1.1.0` - Add new endpoint
- `billing/v2.0.0` - Remove deprecated endpoint

### Version in Files

**package.json:**
```json
{
  "name": "@7d-platform/billing",
  "version": "2.3.1"
}
```

**Git tag:**
```bash
git tag billing-v2.3.1
```

**Docker image:**
```bash
ghcr.io/7d-solutions/billing:2.3.1
```

## Testing Standards

### Test Coverage Requirements

- **Domain layer:** 90%+ coverage
- **Services layer:** 80%+ coverage
- **Repos layer:** 70%+ coverage
- **Routes layer:** 70%+ coverage

## See Also

- [Monorepo Standard](MONOREPO-STANDARD.md) - Repository organization
- [Contract Standard](CONTRACT-STANDARD.md) - API/event schemas
- [Layering Rules](LAYERING-RULES.md) - Dependency management
- [CI Guardrails](CI-GUARDRAILS.md) - Automated enforcement
