# Versioning

The 7D Solutions Platform uses **independent versioning** for modules, platform components, and products, following [Semantic Versioning 2.0.0](https://semver.org/).

## Quick Reference

### Version Format

```
{MAJOR}.{MINOR}.{PATCH}[-{PRERELEASE}][+{BUILD}]
```

- **MAJOR** - Breaking changes
- **MINOR** - New features (backward compatible)
- **PATCH** - Bug fixes

### Examples

```
1.0.0         Initial release
1.1.0         New feature
1.1.1         Bug fix
2.0.0-beta.1  Pre-release
2.0.0         Breaking change
```

## Independent Versioning

Each module has its own version:

```
modules/billing/        v2.3.1
modules/inventory/      v1.8.0
modules/qms/            v3.0.0
```

Products compose modules with specific versions:

```yaml
# products/fireproof-erp/v5.2.0
modules:
  billing: 2.3.1
  inventory: 1.8.0
  qms: 3.0.0
```

## When to Bump Versions

### MAJOR (X.0.0) - Breaking Changes

Increment when:
- Removing an API endpoint
- Removing a field from response
- Renaming a field
- Changing field type
- Making optional field required

**Example:**
```typescript
// v1.x.x
interface Invoice {
  customerId: string;
  total: number;
}

// v2.0.0 - BREAKING: renamed field
interface Invoice {
  customer_id: string;  // renamed from customerId
  total: number;
}
```

### MINOR (x.Y.0) - New Features

Increment when:
- Adding new endpoint
- Adding optional field
- Adding new feature
- Deprecating (not removing) endpoint

**Example:**
```typescript
// v1.0.0
interface Invoice {
  customerId: string;
  total: number;
}

// v1.1.0 - MINOR: added optional field
interface Invoice {
  customerId: string;
  total: number;
  taxAmount?: number;  // new optional field
}
```

### PATCH (x.y.Z) - Bug Fixes

Increment when:
- Fixing incorrect behavior
- Fixing security vulnerability
- Performance improvements
- Documentation updates

**Example:**
```typescript
// v1.1.0 - Bug: tax calculation wrong
calculateTax(amount: number): number {
  return amount * 0.8;  // Bug: should be 0.08
}

// v1.1.1 - PATCH: fixed tax calculation
calculateTax(amount: number): number {
  return amount * 0.08;  // Fixed
}
```

## Git Tags

Tag format: `{module}-v{version}`

```bash
# Module
git tag billing-v2.3.1

# Platform
git tag platform-v3.1.0

# Product
git tag fireproof-erp-v5.2.0
```

## Docker Images

Image tags: `{registry}/{module}:{version}`

```bash
# Specific version (production)
ghcr.io/7d-solutions/billing:2.3.1

# Major version (auto-updates MINOR/PATCH)
ghcr.io/7d-solutions/billing:2

# Latest (development only)
ghcr.io/7d-solutions/billing:latest
```

## Pre-Release Versions

### Alpha
```
1.0.0-alpha.1
1.0.0-alpha.2
```
Early development, unstable, internal testing only.

### Beta
```
1.0.0-beta.1
1.0.0-beta.2
```
Feature-complete, stabilizing, wider testing.

### Release Candidate
```
1.0.0-rc.1
1.0.0-rc.2
```
Final testing before release, production-like environment.

### Progression
```
1.0.0-alpha.1 → ... → 1.0.0-beta.1 → ... → 1.0.0-rc.1 → 1.0.0
```

## Version Compatibility

### Caret (^) - Compatible MINOR versions

```json
{
  "dependencies": {
    "@7d-platform/billing": "^2.3.0"
  }
}
```
Allows: `>=2.3.0 <3.0.0`

### Tilde (~) - Compatible PATCH versions

```json
{
  "dependencies": {
    "@7d-platform/billing": "~2.3.0"
  }
}
```
Allows: `>=2.3.0 <2.4.0`

### Exact - Lock to specific version

```json
{
  "dependencies": {
    "@7d-platform/billing": "2.3.1"
  }
}
```
Allows: `2.3.1` only (recommended for production)

## Deprecation Policy

1. **Announce** - Add deprecation notice (version N)
2. **Wait** - Minimum 6 months / 2 MINOR versions
3. **Remove** - MAJOR version bump (version N+3)

**Example:**
```typescript
// v2.1.0 - Deprecate
/**
 * @deprecated Use createInvoiceV2 instead. Will be removed in v3.0.0.
 */
export function createInvoice() { }

// v2.2.0 - Still works
// v2.3.0 - Still works

// v3.0.0 - REMOVED
// createInvoice no longer exists
```

## Detailed Documentation

For complete versioning guidelines, see:
- [Versioning Standard](docs/architecture/VERSIONING-STANDARD.md) - Detailed policies
- [Release Policy](docs/governance/RELEASE-POLICY.md) - Release process
- [Change Control](docs/governance/CHANGE-CONTROL.md) - PR process

## Quick Commands

```bash
# Bump version
npm version major|minor|patch

# Tag release
git tag billing-v2.3.1
git push --tags

# Build Docker image
docker build -t ghcr.io/7d-solutions/billing:2.3.1 .
docker push ghcr.io/7d-solutions/billing:2.3.1
```

## Support

Questions? See [CONTRIBUTING.md](CONTRIBUTING.md) or ask in #engineering.
