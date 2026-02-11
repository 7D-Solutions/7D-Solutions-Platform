# CI Guardrails

**Version:** 1.0  
**Status:** Active  
**Last Updated:** 2026-02-11

## Overview

Automated checks enforce architectural standards and prevent violations before code reaches production. This document defines the guardrails built into the CI/CD pipeline.

## Pre-Merge Checks

All checks MUST pass before merging to main.

### 1. Contract Validation

**What:** Validates OpenAPI and AsyncAPI contracts

**Tool:** `tools/ci/validate-contracts.sh`

**Checks:**
- Valid OpenAPI 3.x syntax
- Valid AsyncAPI 2.x syntax
- No breaking changes in MINOR/PATCH versions
- All examples validate against schemas

**Example:**
```bash
# Validate all contracts
tools/ci/validate-contracts.sh

# Check specific contract
tools/ci/validate-contracts.sh contracts/api/billing-v2.yaml
```

**Failure Example:**
```
❌ Contract validation failed
File: contracts/api/billing-v2.yaml
Error: Breaking change detected in MINOR version
  - Removed field: customerId (line 45)

Action required:
  1. Revert breaking change, OR
  2. Bump MAJOR version (v2.1.0 → v3.0.0)
```

### 2. Layer Boundary Checks

**What:** Ensures modules follow layering rules

**Tool:** `tools/ci/check-layer-violations.sh`

**Checks:**
- No cross-module source imports
- Domain layer has no external deps
- Routes don't call repos directly
- Repos don't call services

**Example:**
```bash
tools/ci/check-layer-violations.sh modules/billing
```

**Failure Example:**
```
❌ Layer violation detected
File: modules/billing/domain/Invoice.ts
Line: 42
Issue: Domain layer importing from repos layer

import { InvoiceRepository } from '../repos/InvoiceRepository';

Domain must be pure business logic with no external dependencies.
```

### 3. Circular Dependency Detection

**What:** Detects circular dependencies

**Tool:** `tools/ci/check-circular-deps.sh`

**Checks:**
- No circular imports within module
- No circular module dependencies

**Example:**
```bash
tools/ci/check-circular-deps.sh
```

**Failure Example:**
```
❌ Circular dependency detected

ServiceA → ServiceB → ServiceC → ServiceA

This creates brittle, hard-to-test code. Refactor to break the cycle.
```

### 4. Cross-Module Import Detection

**What:** Prevents direct source imports between modules

**Tool:** `tools/ci/check-cross-module-imports.sh`

**Checks:**
- No `import ... from '../../{other-module}'`
- Modules communicate via contracts only

**Example:**
```bash
tools/ci/check-cross-module-imports.sh
```

**Failure Example:**
```
❌ Cross-module import detected
File: modules/billing/services/InvoiceService.ts
Line: 3
Issue: Direct import from customer module

import { Customer } from '../../customer/domain/Customer';

Modules must communicate via contracts:
  1. Define REST API in contracts/api/
  2. Call API via HTTP
  3. OR use event bus
```

### 5. Version Consistency

**What:** Ensures version tags match package versions

**Tool:** `tools/ci/check-version-consistency.sh`

**Checks:**
- package.json version matches git tag
- Docker image tag matches package version
- CHANGELOG.md has entry for version

**Example:**
```bash
tools/ci/check-version-consistency.sh modules/billing
```

**Failure Example:**
```
❌ Version mismatch detected
package.json: 2.3.1
Git tag: billing-v2.3.0

Update git tag or package.json to match.
```

### 6. Test Coverage

**What:** Enforces minimum test coverage

**Tool:** Built into test framework

**Thresholds:**
- Domain layer: 90%
- Services layer: 80%
- Repos layer: 70%
- Routes layer: 70%

**Failure Example:**
```
❌ Test coverage below threshold
Layer: domain
Coverage: 85% (required: 90%)
Missing coverage:
  - domain/entities/Invoice.ts:42-55
  - domain/services/TaxCalculator.ts:78-90
```

### 7. Security Scanning

**What:** Scans for vulnerabilities

**Tools:**
- `npm audit` / `cargo audit`
- Snyk / Dependabot
- Container scanning

**Checks:**
- No high/critical CVEs
- No secrets in code
- No hardcoded credentials

### 8. Contract Tests

**What:** Verifies modules implement their contracts

**Tool:** Built into test suite

**Checks:**
- Provider implements contract
- Consumer honors contract
- No breaking changes

## GitHub Actions Workflow

```yaml
# .github/workflows/ci.yml
name: CI

on:
  pull_request:
  push:
    branches: [main]

jobs:
  validate-contracts:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Validate contracts
        run: tools/ci/validate-contracts.sh

  check-layering:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Check layer violations
        run: tools/ci/check-layer-violations.sh

  check-circular-deps:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Check circular dependencies
        run: tools/ci/check-circular-deps.sh

  check-cross-module-imports:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Check cross-module imports
        run: tools/ci/check-cross-module-imports.sh

  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Run tests
        run: pnpm test
      - name: Check coverage
        run: pnpm test:coverage

  security:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Security audit
        run: pnpm audit
      - name: Container scan
        run: docker scan billing:latest
```

## Pre-Commit Hooks

Catch issues before CI:

```bash
# Install hooks
tools/scripts/install-hooks.sh
```

```bash
#!/bin/bash
# .git/hooks/pre-commit

# Format code
pnpm format

# Lint
pnpm lint

# Quick checks
tools/ci/check-cross-module-imports.sh --fast
tools/ci/check-layer-violations.sh --fast

# Run fast tests
pnpm test:unit
```

## Breaking the Seal

Sometimes you need to override guardrails temporarily.

### Emergency Bypass

```bash
# Merge with bypass (requires approval)
git commit -m "fix: critical security patch" --no-verify
```

**Requirements:**
- Document reason in commit message
- Create follow-up issue
- Get approval from tech lead

### Permanent Exceptions

Add to `.ci-exceptions.yml`:

```yaml
# Legacy code awaiting refactor
layer-violations:
  - modules/legacy-billing/domain/Invoice.ts:42

# Temporary override during migration
cross-module-imports:
  - modules/billing-v2/services/Migration.ts
    expires: 2026-03-31
    reason: "Migration from v1 to v2"
```

## Metrics Dashboard

Track architectural health:

**Key Metrics:**
- % PRs passing all checks first try
- Average time to fix violations
- Number of bypasses per month
- Test coverage trend
- Contract compliance rate

**Dashboard:** https://dashboard.7dsolutions.com/architecture

## Notification Rules

**Slack notifications:**
- #engineering - All violations
- #architecture - Breaking changes, bypasses
- #security - Security scan failures

**Email notifications:**
- Tech leads - Weekly summary
- Module owners - Violations in their modules

## See Also

- [Monorepo Standard](MONOREPO-STANDARD.md) - Repository structure
- [Layering Rules](LAYERING-RULES.md) - Architecture rules
- [Contract Standard](CONTRACT-STANDARD.md) - Contract validation
