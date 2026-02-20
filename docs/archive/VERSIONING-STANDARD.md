# Versioning Standard

**Version:** 1.0  
**Status:** Active  
**Last Updated:** 2026-02-11

## Overview

The 7D Solutions Platform uses independent versioning for each module, platform component, and product. This document defines the versioning strategy and release policies.

## Semantic Versioning

All components follow [Semantic Versioning 2.0.0](https://semver.org/).

### Version Format

```
{MAJOR}.{MINOR}.{PATCH}[-{PRERELEASE}][+{BUILD}]
```

- **MAJOR:** Incompatible API changes
- **MINOR:** Backward-compatible functionality additions
- **PATCH:** Backward-compatible bug fixes
- **PRERELEASE:** Optional pre-release identifier (alpha, beta, rc)
- **BUILD:** Optional build metadata

### Examples

- `1.0.0` - Initial stable release
- `1.1.0` - New feature added
- `1.1.1` - Bug fix
- `2.0.0-beta.1` - Pre-release version
- `2.0.0+20260211` - Build metadata

## Versioning Scope

### Module Versioning

Each module is versioned independently.

**Package file:**
```json
// modules/billing/package.json
{
  "name": "@7d-platform/billing",
  "version": "2.3.1"
}
```

**Git tag:**
```bash
billing-v2.3.1
```

**Docker image:**
```bash
ghcr.io/7d-solutions/billing:2.3.1
```

### Platform Versioning

Platform components are versioned together as a suite.

**Git tag:**
```bash
platform-v3.1.0
```

**Components:**
- `platform/identity:3.1.0`
- `platform/events:3.1.0`
- `platform/orchestration:3.1.0`

### Product Versioning

Products version independently from modules.

**Example:**
```
products/fireproof-erp/version: 5.2.0
  ├── uses billing/v2.3.1
  ├── uses inventory/v1.8.2
  └── uses qms/v3.0.0
```

## When to Bump Versions

### MAJOR Version (X.0.0)

Increment when making incompatible changes:

**API Changes:**
- Remove an endpoint
- Remove a field from response
- Rename a field
- Change field type
- Make optional field required
- Change HTTP status codes
- Remove query parameter

**Event Changes:**
- Remove event type
- Remove field from event payload
- Rename event type
- Change payload structure

**Database Changes:**
- Remove table or column (if exposed in API)
- Change primary key structure

**Behavioral Changes:**
- Fundamental algorithm changes
- Change default behavior

### MINOR Version (x.Y.0)

Increment when adding backward-compatible functionality:

**API Changes:**
- Add new endpoint
- Add optional field to request
- Add field to response
- Add new query parameter (optional)
- Deprecate endpoint (not remove)

**Event Changes:**
- Add new event type
- Add optional field to payload

**Database Changes:**
- Add new table
- Add nullable column

**Features:**
- New configuration option
- Performance improvements
- Enhanced logging

### PATCH Version (x.y.Z)

Increment for backward-compatible bug fixes:

**Bug Fixes:**
- Fix incorrect behavior
- Fix security vulnerability
- Fix edge case error

**Non-Functional:**
- Documentation updates
- Test improvements
- Refactoring (no behavior change)

## Version Compatibility

### Module-to-Module

Modules declare compatible versions:

```json
// modules/billing/package.json
{
  "name": "@7d-platform/billing",
  "version": "2.3.1",
  "peerDependencies": {
    "@7d-platform/customer": "^1.5.0"
  }
}
```

### Platform-to-Module

Modules declare minimum platform version:

```json
{
  "engines": {
    "platform": ">=3.0.0"
  }
}
```

### Product-to-Module

Products specify exact module versions:

```yaml
# products/fireproof-erp/compose/modules.yml
modules:
  billing:
    version: 2.3.1
    source: ghcr.io/7d-solutions/billing:2.3.1
  inventory:
    version: 1.8.2
    source: ghcr.io/7d-solutions/inventory:1.8.2
```

## Pre-Release Versions

### Alpha

Early development, unstable.

**Format:** `1.0.0-alpha.1`

**Usage:** Internal testing only

### Beta

Feature-complete, stabilizing.

**Format:** `1.0.0-beta.1`

**Usage:** Wider testing, early adopters

### Release Candidate

Final testing before release.

**Format:** `1.0.0-rc.1`

**Usage:** Production-like testing

### Progression

```
1.0.0-alpha.1 → 1.0.0-alpha.2 → ...
  ↓
1.0.0-beta.1 → 1.0.0-beta.2 → ...
  ↓
1.0.0-rc.1 → 1.0.0-rc.2 → ...
  ↓
1.0.0
```

## Deprecation Policy

### Deprecation Process

1. **Announce** - Add deprecation notice
2. **Document** - Provide migration guide
3. **Wait** - Minimum 6 months (2 MINOR versions)
4. **Remove** - MAJOR version bump

### Deprecation Notice

**In code:**
```typescript
/**
 * @deprecated Use createInvoiceV2 instead. Will be removed in v3.0.0.
 */
export function createInvoice() { /* ... */ }
```

**In API contract:**
```yaml
paths:
  /api/v1/invoices:
    post:
      deprecated: true
      description: |
        DEPRECATED: Use /api/v2/invoices instead.
        This endpoint will be removed in version 3.0.0.
```

**In response headers:**
```
Deprecation: true
Sunset: Sat, 31 Aug 2026 23:59:59 GMT
Link: </api/v2/invoices>; rel="successor-version"
```

## Version Discovery

### API Version in URL

```
/api/v1/invoices    # Version 1
/api/v2/invoices    # Version 2
```

### Service Version Header

```http
GET /health
X-Service-Version: billing/v2.3.1
X-Platform-Version: platform/v3.1.0
```

### Docker Image Tags

```bash
# Specific version
ghcr.io/7d-solutions/billing:2.3.1

# Major version (latest MINOR/PATCH)
ghcr.io/7d-solutions/billing:2

# Latest (NOT for production)
ghcr.io/7d-solutions/billing:latest
```

## Release Checklist

### Pre-Release

- [ ] All tests passing
- [ ] Contract validation passing
- [ ] No breaking changes in MINOR/PATCH
- [ ] CHANGELOG.md updated
- [ ] Migration guide (if MAJOR)
- [ ] Documentation updated

### Release

- [ ] Bump version in package.json / Cargo.toml
- [ ] Create git tag
- [ ] Build Docker image with version tag
- [ ] Push to registry
- [ ] Create GitHub release
- [ ] Notify consuming teams (if MAJOR)

### Post-Release

- [ ] Verify deployment
- [ ] Monitor metrics
- [ ] Update product dependencies

## Version Constraints

### Ranges

Use standard semver ranges:

```json
{
  "dependencies": {
    "@7d-platform/customer": "^1.5.0",    // >=1.5.0 <2.0.0
    "@7d-platform/billing": "~2.3.0",     // >=2.3.0 <2.4.0
    "@7d-platform/inventory": "1.8.2"     // Exact version
  }
}
```

### Production Dependencies

Products SHOULD pin exact versions:

```yaml
# products/fireproof-erp/compose/modules.yml
modules:
  billing:
    version: 2.3.1     # Exact version, no ranges
```

### Development Dependencies

Modules MAY use ranges for dev dependencies:

```json
{
  "devDependencies": {
    "jest": "^29.0.0",
    "typescript": "^5.0.0"
  }
}
```

## Zero-Downtime Updates

### Rolling Deployments

1. Deploy new version alongside old
2. Gradually shift traffic
3. Monitor error rates
4. Complete migration
5. Decommission old version

### Database Migrations

**Forward-compatible migrations:**

```sql
-- Phase 1: Add new column (MINOR version)
ALTER TABLE invoices ADD COLUMN customer_name VARCHAR(255);

-- Phase 2: Populate data
UPDATE invoices SET customer_name = (SELECT name FROM customers WHERE ...);

-- Phase 3: Make required (MAJOR version)
ALTER TABLE invoices ALTER COLUMN customer_name SET NOT NULL;
```

## See Also

- [Module Standard](MODULE-STANDARD.md) - Module structure
- [Contract Standard](CONTRACT-STANDARD.md) - API versioning
- [Release Policy](../governance/RELEASE-POLICY.md) - Release process
