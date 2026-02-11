# Layering Rules

**Version:** 1.0  
**Status:** Active  
**Last Updated:** 2026-02-11

## Overview

This document defines the dependency rules and architectural layers for the 7D Solutions Platform. Proper layering ensures modularity, testability, and maintainability.

## Three-Tier Architecture

```
┌─────────────────────────────────┐
│ TIER 3: PRODUCTS                │
│ Assembly, configuration only    │
└─────────────────────────────────┘
           ↓ depends on
┌─────────────────────────────────┐
│ TIER 2: MODULES                 │
│ Business logic, reusable        │
└─────────────────────────────────┘
           ↓ depends on
┌─────────────────────────────────┐
│ TIER 1: PLATFORM                │
│ Core runtime, infrastructure    │
└─────────────────────────────────┘
```

### Allowed Dependencies

✅ Products → Modules (via contracts)  
✅ Products → Platform (via contracts)  
✅ Modules → Platform (via contracts)  
✅ Modules → Packages (shared libraries)  

### Prohibited Dependencies

❌ Platform → Modules  
❌ Platform → Products  
❌ Modules → Products  
❌ Modules → Modules (source imports)  

## Module Internal Layering

Within each module, follow strict layering:

```
┌─────────────────────────────────┐
│ ROUTES (HTTP Layer)             │
│ Request/response handling       │
└─────────────────────────────────┘
           ↓
┌─────────────────────────────────┐
│ SERVICES (Application Layer)    │
│ Orchestration, workflows        │
└─────────────────────────────────┘
           ↓
┌─────────────────────────────────┐
│ REPOS (Data Access Layer)       │
│ Database queries                │
└─────────────────────────────────┘
           ↓
┌─────────────────────────────────┐
│ DOMAIN (Business Logic Layer)   │
│ Pure business rules             │
└─────────────────────────────────┘
```

### Layer Responsibilities

**Routes:**
- HTTP request/response
- Input validation
- Authentication/authorization
- Error handling

**Services:**
- Orchestrate domain objects
- Coordinate multiple repos
- Publish events
- Handle transactions

**Repos:**
- Database access
- ORM mapping
- Query execution
- Transaction management

**Domain:**
- Pure business logic
- Entities, value objects
- Business rules
- Domain events

### Allowed Internal Dependencies

✅ Routes → Services  
✅ Services → Repos  
✅ Services → Domain  
✅ Repos → Domain  

### Prohibited Internal Dependencies

❌ Domain → Repos (pure domain)  
❌ Domain → Services (pure domain)  
❌ Domain → Routes (pure domain)  
❌ Repos → Services (skip layers)  
❌ Repos → Routes (skip layers)  
❌ Services → Routes (skip layers)  

## Cross-Module Communication

### Synchronous (REST API)

```typescript
// ✅ GOOD: Via HTTP contract
const response = await fetch('http://customer-service/api/v1/customers/123');
const customer = await response.json();
```

```typescript
// ❌ BAD: Direct import
import { CustomerService } from '../../customer/services/CustomerService';
const customer = await CustomerService.findById('123');
```

### Asynchronous (Events)

```typescript
// ✅ GOOD: Via event bus
eventBus.publish('invoice.created', { invoiceId: '123' });

eventBus.subscribe('customer.updated', async (event) => {
  await handleCustomerUpdate(event.payload);
});
```

```typescript
// ❌ BAD: Direct function call
import { notifyCustomerUpdated } from '../../customer/notifications';
notifyCustomerUpdated({ customerId: '123' });
```

## Shared Code Rules

### Packages (Shared Libraries)

**Create package ONLY if:**
1. Used by 2+ modules
2. Stable, unlikely to change
3. No business logic

**Example:**

```
packages/
├── types/                  # Shared TypeScript types
├── validation/             # If used by 2+ modules
└── testing/                # Test utilities
```

### Copy vs. Share

**Prefer copying over premature abstraction.**

```typescript
// ✅ GOOD: Copy simple utility
// modules/billing/utils/formatCurrency.ts
export function formatCurrency(amount: number): string {
  return `$${amount.toFixed(2)}`;
}

// modules/inventory/utils/formatCurrency.ts
export function formatCurrency(amount: number): string {
  return `$${amount.toFixed(2)}`;
}
```

```typescript
// ❌ BAD: Premature abstraction (only 2 lines)
// packages/utils/formatCurrency.ts
export function formatCurrency(amount: number): string {
  return `$${amount.toFixed(2)}`;
}
```

**When to extract:**
- Function is 50+ lines
- Used by 3+ modules
- Business logic that MUST stay in sync

## Dependency Injection

### Constructor Injection

```typescript
// ✅ GOOD: Dependencies injected
export class CreateInvoiceService {
  constructor(
    private invoiceRepo: InvoiceRepository,
    private customerRepo: CustomerRepository,
    private eventBus: EventBus
  ) {}

  async execute(data: CreateInvoiceInput): Promise<Invoice> {
    // ...
  }
}
```

```typescript
// ❌ BAD: Direct instantiation
export class CreateInvoiceService {
  async execute(data: CreateInvoiceInput): Promise<Invoice> {
    const invoiceRepo = new PrismaInvoiceRepository();  // ❌
    const customerRepo = new PrismaCustomerRepository(); // ❌
    // ...
  }
}
```

### Interface Dependencies

```typescript
// ✅ GOOD: Depend on interface
export interface InvoiceRepository {
  findById(id: string): Promise<Invoice | null>;
  save(invoice: Invoice): Promise<void>;
}

export class CreateInvoiceService {
  constructor(private invoiceRepo: InvoiceRepository) {}
}
```

```typescript
// ❌ BAD: Depend on concrete implementation
import { PrismaInvoiceRepository } from '../repos/PrismaInvoiceRepository';

export class CreateInvoiceService {
  constructor(private invoiceRepo: PrismaInvoiceRepository) {} // ❌
}
```

## Testing Boundaries

### Unit Tests

Test each layer independently:

```typescript
// domain/entities/Invoice.test.ts
describe('Invoice', () => {
  it('calculates total', () => {
    const invoice = new Invoice(/* ... */);
    expect(invoice.calculateTotal()).toEqual(Money.dollars(150));
  });
});
```

### Integration Tests

Test layer interactions:

```typescript
// services/CreateInvoiceService.test.ts
describe('CreateInvoiceService', () => {
  it('creates invoice and publishes event', async () => {
    const service = new CreateInvoiceService(mockRepo, mockEventBus);
    const invoice = await service.execute(/* ... */);
    expect(mockEventBus.published).toContainEqual({
      type: 'invoice.created'
    });
  });
});
```

### Contract Tests

Test module boundaries:

```typescript
// tests/contract/api.test.ts
it('POST /invoices matches contract', async () => {
  const response = await request(app).post('/api/v1/invoices').send(/* ... */);
  expect(validateOpenAPIResponse(spec, response)).toPass();
});
```

## Enforcement

### Static Analysis

Use linting rules to enforce layering:

```json
// .eslintrc.json
{
  "rules": {
    "no-restricted-imports": [
      "error",
      {
        "patterns": [
          "../../../modules/*",  // No cross-module imports
          "../../routes/*"       // Domain can't import routes
        ]
      }
    ]
  }
}
```

### CI Checks

```bash
# Check for circular dependencies
tools/ci/check-circular-deps.sh

# Check for cross-module imports
tools/ci/check-cross-module-imports.sh

# Validate layer boundaries
tools/ci/check-layer-violations.sh
```

### Architecture Tests

```typescript
// tests/architecture/layering.test.ts
import { analyze } from 'dependency-analyzer';

it('domain layer has no external dependencies', () => {
  const analysis = analyze('modules/billing/domain');
  expect(analysis.externalDependencies).toEqual([]);
});

it('services layer depends only on repos and domain', () => {
  const analysis = analyze('modules/billing/services');
  const allowedDeps = ['repos', 'domain', 'platform'];
  expect(analysis.dependencies).toBeSubsetOf(allowedDeps);
});
```

## Common Violations

### 1. Skip Layer

```typescript
// ❌ Routes calling repos directly
export function createInvoiceRoute(invoiceRepo: InvoiceRepository) {
  return async (req: Request, res: Response) => {
    const invoice = await invoiceRepo.save(/* ... */);  // ❌ Skip services
    res.json(invoice);
  };
}
```

**Fix:** Route → Service → Repo

### 2. Reverse Dependency

```typescript
// ❌ Domain calling repos
export class Invoice {
  async save(): Promise<void> {
    await invoiceRepo.save(this);  // ❌ Domain shouldn't know about repos
  }
}
```

**Fix:** Service orchestrates domain and repos

### 3. God Service

```typescript
// ❌ One service doing everything
export class InvoiceService {
  async createInvoice() { /* ... */ }
  async updateInvoice() { /* ... */ }
  async deleteInvoice() { /* ... */ }
  async sendInvoice() { /* ... */ }
  async calculateTax() { /* ... */ }
  // 50 more methods...
}
```

**Fix:** Split into focused command/query services

### 4. Anemic Domain

```typescript
// ❌ Domain with no logic
export class Invoice {
  id: string;
  customerId: string;
  total: number;
  // Just data, no behavior
}

// Business logic leaks into services
export class InvoiceService {
  async calculateTotal(invoice: Invoice): Promise<number> {
    // ❌ This belongs in domain
    return invoice.items.reduce((sum, item) => sum + item.amount, 0);
  }
}
```

**Fix:** Push logic into domain

```typescript
// ✅ Rich domain
export class Invoice {
  calculateTotal(): Money {
    return this.items.reduce((sum, item) => sum.add(item.amount), Money.zero());
  }
}
```

## See Also

- [Monorepo Standard](MONOREPO-STANDARD.md) - Repository structure
- [Module Standard](MODULE-STANDARD.md) - Module architecture
- [CI Guardrails](CI-GUARDRAILS.md) - Automated checks
