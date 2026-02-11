# Contract Standard

**Version:** 1.0  
**Status:** Active  
**Last Updated:** 2026-02-11

## Overview

Contracts are the single source of truth for all integration points in the 7D Solutions Platform. This document defines how to create, version, and enforce API and event contracts.

## Contract Types

### REST API Contracts (OpenAPI)

**Location:** `contracts/api/`

**Format:** OpenAPI 3.x YAML

**Naming:** `{module}-{resource}-v{version}.yaml`

**Example:**
```yaml
# contracts/api/billing-invoice-v2.yaml
openapi: 3.0.3
info:
  title: Billing Invoice API
  version: 2.0.0
paths:
  /api/v2/invoices:
    post:
      operationId: createInvoice
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/CreateInvoiceRequest'
      responses:
        '201':
          description: Invoice created
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Invoice'
```

### Event Contracts (AsyncAPI)

**Location:** `contracts/events/`

**Format:** AsyncAPI 2.x YAML

**Naming:** `{domain}-events-v{version}.yaml`

**Example:**
```yaml
# contracts/events/billing-events-v1.yaml
asyncapi: 2.6.0
info:
  title: Billing Events
  version: 1.0.0
channels:
  invoice.created:
    publish:
      message:
        payload:
          type: object
          properties:
            invoiceId:
              type: string
            customerId:
              type: string
            amount:
              type: number
```

### Data Schemas

**Location:** `contracts/schemas/`

**Format:** JSON Schema

**Naming:** `{entity}.schema.json`

## Contract Development Workflow

### 1. Design First

Write contracts BEFORE implementing code.

```bash
# 1. Create contract
vim contracts/api/billing-invoice-v2.yaml

# 2. Validate contract
tools/scripts/validate-contract.sh contracts/api/billing-invoice-v2.yaml

# 3. Generate types
pnpm generate:types

# 4. Implement module using generated types
```

### 2. Versioning

Use semantic versioning for contracts:

- **MAJOR:** Breaking changes (remove fields, change types)
- **MINOR:** Additive changes (new endpoints, optional fields)
- **PATCH:** Documentation fixes only

### 3. Code Generation

Generate types from contracts:

```bash
# Generate TypeScript types from OpenAPI
openapi-typescript contracts/api/billing-invoice-v2.yaml -o modules/billing/types/api.ts

# Generate Rust types
openapi-generator generate -i contracts/api/billing-invoice-v2.yaml -g rust
```

## Breaking Changes

### What Constitutes a Breaking Change?

**API (REST):**
- ✅ Breaking: Remove endpoint
- ✅ Breaking: Rename field
- ✅ Breaking: Change field type
- ✅ Breaking: Make optional field required
- ❌ NOT Breaking: Add new endpoint
- ❌ NOT Breaking: Add optional field
- ❌ NOT Breaking: Deprecate (but not remove) endpoint

**Events:**
- ✅ Breaking: Remove event type
- ✅ Breaking: Remove payload field
- ✅ Breaking: Change payload structure
- ❌ NOT Breaking: Add new event type
- ❌ NOT Breaking: Add optional payload field

### Handling Breaking Changes

1. **Bump MAJOR version**
   - `billing-invoice-v2.yaml` → `billing-invoice-v3.yaml`

2. **Support both versions during migration**
   ```
   contracts/api/
   ├── billing-invoice-v2.yaml    # Legacy
   └── billing-invoice-v3.yaml    # New
   ```

3. **Deprecate old version**
   - Add `deprecated: true` to old contract
   - Set sunset date (minimum 6 months)

4. **Remove old version**
   - After sunset date
   - Requires approval from all consuming teams

## Contract Enforcement

### CI Validation

All contracts MUST pass validation before merge:

```bash
# Validate OpenAPI
tools/ci/validate-openapi.sh

# Validate AsyncAPI
tools/ci/validate-asyncapi.sh

# Check for breaking changes
tools/ci/check-breaking-changes.sh
```

### Contract Tests

Modules MUST have contract tests:

```typescript
// tests/contract/api.test.ts
import { validateResponse } from '@7d-platform/contract-testing';
import spec from '../../contracts/openapi.yaml';

it('POST /invoices returns valid response', async () => {
  const response = await request(app)
    .post('/api/v2/invoices')
    .send({ /* ... */ });

  expect(validateResponse(spec, '/invoices', 'post', response)).toPass();
});
```

### Provider Contracts

Modules that expose APIs MUST verify they implement the contract.

### Consumer Contracts

Modules that call APIs MUST verify the provider honors the contract.

## Best Practices

### 1. Use References

DRY - define schemas once:

```yaml
components:
  schemas:
    Money:
      type: object
      properties:
        amount:
          type: number
        currency:
          type: string

paths:
  /invoices:
    post:
      requestBody:
        schema:
          properties:
            total:
              $ref: '#/components/schemas/Money'
```

### 2. Document Everything

```yaml
paths:
  /invoices:
    post:
      summary: Create a new invoice
      description: |
        Creates a new invoice for the specified customer.
        The invoice will be in 'draft' status initially.
      parameters:
        - name: idempotencyKey
          description: Unique key to prevent duplicate invoices
          required: true
```

### 3. Use Examples

```yaml
components:
  schemas:
    Invoice:
      type: object
      example:
        id: inv_abc123
        customerId: cus_xyz789
        amount: 150.00
```

### 4. Validate Inputs

```yaml
components:
  schemas:
    CreateInvoiceRequest:
      type: object
      required:
        - customerId
        - items
      properties:
        customerId:
          type: string
          pattern: '^cus_[a-zA-Z0-9]{8}$'
        items:
          type: array
          minItems: 1
          maxItems: 100
```

## See Also

- [Module Standard](MODULE-STANDARD.md) - Module structure
- [Versioning Standard](VERSIONING-STANDARD.md) - SemVer policies
- [CI Guardrails](CI-GUARDRAILS.md) - Automated enforcement
