# Contract Catalog — 7D Solutions Platform

## Overview

This directory contains the authoritative public contracts for all modules in the 7D Solutions Platform. Contracts define the integration surface between modules and external systems.

## Contract Types

### 1. REST API Contracts (OpenAPI)

OpenAPI 3.0.3 YAML specifications defining HTTP endpoints, request/response schemas, and authentication requirements.

### 2. Event Contracts (JSON Schema)

Event schemas defining asynchronous communication between modules via the event bus.

## Contract Governance

- **Source of Truth:** YAML files in this directory are authoritative
- **Generated Specs:** Rust code may generate candidate specs, but they must be reviewed and committed manually
- **Breaking Changes:** Require MAJOR version bump, CHANGELOG update, and contract version update
- **CI Validation:** CI will validate that generated specs match committed specs

See [CONTRACT-STANDARD.md](../docs/architecture/CONTRACT-STANDARD.md) for full governance rules.

## Available Contracts

### Module: Auth

**Path:** `auth/auth-v1.yaml`

**Purpose:** Authentication and authorization API

**Endpoints:**
- User authentication
- Token management
- Session handling

**Version:** 0.1.0

**Documentation:** See [auth-v1.yaml](auth/auth-v1.yaml)

---

### Module: AR (Accounts Receivable)

**Path:** `ar/ar-v1.yaml`

**Purpose:** Billing records and payment method references

**Endpoints:**
- Customer management
- Invoice management
- Charge and refund operations
- Payment method references (non-sensitive)
- Dispute handling
- Webhook management

**Key Principles:**
- Does NOT store raw card data or sensitive payment secrets
- Delegates actual payment processing to external processor
- Stores only processor-issued identifiers and metadata

**Version:** 0.1.0

**Documentation:** See [ar-v1.yaml](ar/ar-v1.yaml)

---

### Module: Subscriptions

**Path:** `subscriptions/subscriptions-v1.yaml`

**Purpose:** Recurring billing logic and service agreements

**Endpoints:**
- Subscription management (create, pause, resume, cancel)
- Subscription plan management
- Bill run execution

**Key Principles:**
- Owns subscription schedules and billing logic
- Does NOT own invoice data (calls AR API)
- Does NOT own payment state or ledger entries
- Never stores financial truth

**Version:** 0.1.0

**State Machine:**
```
active → paused → resumed → cancelled
```

**Idempotency:** Bill runs use `bill_run_id` to prevent duplicate invoices

**Documentation:** See [subscriptions-v1.yaml](subscriptions/subscriptions-v1.yaml)

---

### Events

**Path:** `events/`

**Purpose:** Asynchronous event schemas for inter-module communication

**Available Events:**
- `gl-posting-request.v1.json` - Request GL posting
- `gl-posting-accepted.v1.json` - GL posting accepted
- `gl-posting-rejected.v1.json` - GL posting rejected

**Event Naming Convention:**
```
<domain>.<entity>.<action>
```

**Transport:** NATS event bus (immutable, asynchronous, decoupled)

**Documentation:** See [events/README.md](events/README.md)

---

## Module Interaction Patterns

### Subscriptions → AR → Payments

```
┌──────────────┐      ┌──────┐      ┌──────────┐
│ Subscriptions│─────▶│  AR  │─────▶│ Payments │
└──────────────┘      └──────┘      └──────────┘
     (schedule)      (invoice)      (charge)
```

**Flow:**
1. Subscriptions executes bill run
2. Subscriptions calls AR: `POST /api/ar/invoices`
3. AR emits event: `ar.invoice.issued`
4. Payments listens and processes payment

**Invariants:**
- Subscriptions never writes invoices directly
- Subscriptions never calls Payments
- AR is source of truth for invoice state

---

## Contract Testing

Contract tests validate that implementations match their specifications.

**Location:** `modules/{module}/tests/contract/`

**Tools:**
- OpenAPI validation
- JSON Schema validation
- Pact (for consumer-driven contracts)

**CI Enforcement:**
- All contracts must have passing tests
- Generated specs must match committed specs
- Breaking changes trigger major version bump

---

## Versioning Strategy

### SemVer for Contracts

**Format:** `{module}-v{MAJOR}.{MINOR}.{PATCH}`

**Rules:**
- **MAJOR:** Breaking changes (remove endpoint, change required field)
- **MINOR:** Additive changes (new endpoint, new optional field)
- **PATCH:** Documentation or non-functional changes

**Examples:**
- `ar-v1.0.0` - Initial release
- `ar-v1.1.0` - Add new endpoint
- `ar-v2.0.0` - Remove deprecated endpoint

### Version Compatibility

- Consumers must specify version in URL: `/api/v1/subscriptions`
- Multiple versions can coexist during migration
- Deprecated versions maintained for 6 months minimum

---

## Adding a New Contract

1. **Create contract file:**
   ```bash
   mkdir -p contracts/{module-name}
   touch contracts/{module-name}/{module-name}-v1.yaml
   ```

2. **Define OpenAPI spec:**
   - Follow existing patterns (see `ar/ar-v1.yaml`)
   - Include clear descriptions
   - Document ownership boundaries

3. **Update this catalog:**
   - Add entry to "Available Contracts" section
   - Document endpoints and key principles

4. **Add contract tests:**
   - Create tests in `modules/{module}/tests/contract/`
   - Validate against spec

5. **Commit and PR:**
   - Commit contract file
   - Update CHANGELOG
   - Request review from module owner

---

## See Also

- [MODULE-STANDARD.md](../docs/architecture/MODULE-STANDARD.md) - Module structure
- [CONTRACT-STANDARD.md](../docs/architecture/CONTRACT-STANDARD.md) - Contract governance
- [BOUNDARY-ENFORCEMENT.md](../docs/architecture/BOUNDARY-ENFORCEMENT.md) - Module boundaries
- [VERSIONING-STANDARD.md](../docs/architecture/VERSIONING-STANDARD.md) - Version strategy
