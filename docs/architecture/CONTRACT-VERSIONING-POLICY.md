# Contract Versioning & Deprecation Policy

**Version:** 1.0
**Status:** Active
**Last Updated:** 2026-02-12

## Overview

This policy defines how contracts (OpenAPI specifications and event schemas) are versioned, evolved, and deprecated in the 7D Solutions Platform. It ensures backward compatibility, smooth migrations, and production stability.

**Key Principles:**
- Contracts are independently versioned using Semantic Versioning
- Breaking changes require major version bumps
- Deprecation windows protect consumers
- Dual-publish/dual-consume enables zero-downtime migrations

---

## Table of Contents

1. [Semantic Versioning Rules](#semantic-versioning-rules)
2. [Breaking vs Non-Breaking Changes](#breaking-vs-non-breaking-changes)
3. [Event Schema Versioning](#event-schema-versioning)
4. [OpenAPI Versioning](#openapi-versioning)
5. [Deprecation Policy](#deprecation-policy)
6. [Dual-Publish & Dual-Consume](#dual-publish--dual-consume)
7. [Golden Examples & Contract Tests](#golden-examples--contract-tests)
8. [CI Enforcement](#ci-enforcement)
9. [Migration Checklist](#migration-checklist)

---

## Semantic Versioning Rules

All contracts follow [Semantic Versioning 2.0.0](https://semver.org/).

### Version Format

```
{MAJOR}.{MINOR}.{PATCH}
```

- **MAJOR:** Breaking changes (incompatible API changes)
- **MINOR:** Backward-compatible additions (new features)
- **PATCH:** Backward-compatible fixes (corrections, clarifications)

### Examples

- **1.0.0** → **1.1.0**: Add optional field (MINOR)
- **1.1.0** → **1.1.1**: Fix field description typo (PATCH)
- **1.1.1** → **2.0.0**: Remove required field (MAJOR - breaking)

### When to Bump

| Change Type | Version Bump | Example |
|-------------|--------------|---------|
| Add optional field | MINOR | `email` field added with `required: false` |
| Add new endpoint | MINOR | New `POST /api/invoices/bulk` endpoint |
| Add new event type | MINOR | New `invoice.cancelled` event |
| Fix schema bug | PATCH | Correct `minLength` from 2 to 3 |
| Remove field | MAJOR | Remove `legacy_id` field |
| Rename field | MAJOR | Rename `customer_id` to `customerId` |
| Change field type | MAJOR | Change `amount` from string to number |
| Make field required | MAJOR | Change `email` to `required: true` |

---

## Breaking vs Non-Breaking Changes

### Breaking Changes

Changes that can break existing consumers:

#### For OpenAPI (REST APIs):

❌ **Remove or rename endpoint**
```yaml
# BREAKING
- DELETE /api/v1/customers/{id}
+ DELETE /api/v2/customers/{customerId}  # Renamed parameter
```

❌ **Remove required field from response**
```json
// v1 (has customer_name)
{"id": "123", "customer_name": "Acme Corp"}

// v2 (removed customer_name) - BREAKING
{"id": "123"}
```

❌ **Change field type**
```yaml
# BREAKING
amount:
  type: string  # v1
  type: number  # v2 - consumers expecting string will break
```

❌ **Add required field to request**
```yaml
# BREAKING - existing requests will fail validation
required:
  - customer_id
  - email        # New required field
```

❌ **Narrow enum values**
```yaml
# BREAKING - consumers sending "pending" will be rejected
status:
  enum: [pending, active, cancelled]  # v1
  enum: [active, cancelled]           # v2 - removed "pending"
```

❌ **Change HTTP status codes**
```yaml
# BREAKING - consumers checking for 404 will break
responses:
  '404': ...  # v1
  '410': ...  # v2 - changed to 410 Gone
```

#### For Event Schemas:

❌ **Remove required field from payload**
```json
// v1
{"invoice_id": "123", "customer_id": "456"}

// v2 - BREAKING for consumers expecting customer_id
{"invoice_id": "123"}
```

❌ **Change field type or semantics**
```json
// v1 - amount in dollars (float)
{"amount": 100.50}

// v2 - amount in cents (int) - BREAKING
{"amount": 10050}
```

❌ **Change event subject naming**
```
ar.events.ar.invoice.issued  (v1)
ar.events.invoice.created     (v2) - BREAKING, different subject
```

❌ **Change envelope structure**
```json
// v1
{"event_id": "...", "payload": {...}}

// v2 - BREAKING
{"id": "...", "data": {...}}
```

### Non-Breaking Changes

Changes that are safe for existing consumers:

✅ **Add optional field**
```yaml
# Safe - consumers ignore unknown fields
customer:
  properties:
    id: string
    email: string      # New optional field
```

✅ **Add new endpoint**
```yaml
# Safe - doesn't affect existing endpoints
paths:
  /api/customers:     # Existing
  /api/customers/bulk: # New endpoint
```

✅ **Add new event type**
```
ar.events.ar.invoice.issued    # Existing
ar.events.ar.invoice.voided    # New event
```

✅ **Expand enum values**
```yaml
# Safe IF consumers handle unknown values gracefully
status:
  enum: [pending, active]        # v1
  enum: [pending, active, paused] # v2 - added "paused"
```

**⚠️ Warning:** Adding enum values is only safe if:
- Consumers use a default case for unknown values
- Documentation explicitly states enum may expand

✅ **Add examples or clarify documentation**
```yaml
# Safe - doesn't change behavior
description: "Customer email address (must be valid format)"
```

✅ **Make required field optional**
```yaml
# Safe - more permissive is backward compatible
required:
  - customer_id
  - email       # Remove from required (now optional)
```

---

## Event Schema Versioning

### File Naming Convention

Event schemas are versioned with `.v{MAJOR}` suffix:

```
contracts/events/{domain}-{entity}-{action}.v{MAJOR}.json
```

**Examples:**
```
contracts/events/ar-invoice-issued.v1.json
contracts/events/ar-invoice-issued.v2.json
contracts/events/payments-payment-succeeded.v1.json
contracts/events/gl-posting-requested.v1.json
```

### Event Subject Versioning

NATS subject pattern with version:

```
{module}.events.{module}.{entity}.{action}
```

**No version in subject** - use schema version instead:
- ✅ `ar.events.ar.invoice.issued` (schema: v1)
- ✅ `ar.events.ar.invoice.issued` (schema: v2)
- ❌ `ar.events.ar.invoice.issued.v1` (don't version subject)

**Rationale:** Subjects stay stable; schema evolution handles compatibility.

### Major Version Bump

When introducing a breaking change:

1. **Create new schema file:**
   ```
   contracts/events/ar-invoice-issued.v2.json
   ```

2. **Keep v1 schema during deprecation window** (see Deprecation Policy)

3. **Publish to same subject** with new schema version in envelope:
   ```json
   {
     "event_id": "...",
     "source_version": "2.0.0",  // ← Indicates schema v2
     "payload": {...}
   }
   ```

4. **Consumers check `source_version`** to handle multiple schemas:
   ```rust
   match envelope.source_version.split('.').next() {
       Some("1") => parse_v1_schema(&envelope),
       Some("2") => parse_v2_schema(&envelope),
       _ => Err("Unsupported schema version")
   }
   ```

### Minor/Patch Bump

For non-breaking changes:

1. **Update existing schema file** (no new file):
   ```
   contracts/events/ar-invoice-issued.v1.json  // Update in place
   ```

2. **Bump minor version in `$id`:**
   ```json
   {
     "$id": "https://7dsolutions.io/schemas/events/ar-invoice-issued.v1.1.json"
   }
   ```

3. **Update CHANGELOG** (see CI Enforcement section)

---

## OpenAPI Versioning

### File Naming Convention

OpenAPI specs include module name and version:

```
contracts/{module}/{module}-v{MAJOR}.{MINOR}.{PATCH}.yaml
```

**Examples:**
```
contracts/ar/ar-v1.0.0.yaml
contracts/ar/ar-v1.1.0.yaml
contracts/ar/ar-v2.0.0.yaml
contracts/payments/payments-v0.1.0.yaml
```

### URL Path Versioning

**Don't version in URL path** - use HTTP headers instead:

❌ **Bad:**
```
GET /api/v1/invoices
GET /api/v2/invoices
```

✅ **Good:**
```
GET /api/invoices
Headers:
  Accept: application/vnd.7dsolutions.ar.v1+json
  Accept: application/vnd.7dsolutions.ar.v2+json
```

**Rationale:**
- Cleaner URLs
- Version negotiation via Accept header
- Easier to deprecate old versions

**Exception:** If URL versioning is already established, continue for consistency.

### Major Version Bump

When introducing a breaking change:

1. **Create new OpenAPI file:**
   ```
   contracts/ar/ar-v2.0.0.yaml
   ```

2. **Implement version negotiation** in API gateway/module:
   ```rust
   match request.headers().get("Accept") {
       Some("application/vnd.7dsolutions.ar.v1+json") => handle_v1(request),
       Some("application/vnd.7dsolutions.ar.v2+json") => handle_v2(request),
       _ => handle_latest(request),
   }
   ```

3. **Keep v1 spec during deprecation window**

### Minor/Patch Bump

For non-breaking changes:

1. **Update existing OpenAPI file:**
   ```
   contracts/ar/ar-v1.1.0.yaml  // Bump minor version
   ```

2. **Update `info.version` field:**
   ```yaml
   info:
     version: "1.1.0"
   ```

3. **Update CHANGELOG**

---

## Deprecation Policy

### Deprecation Window

**Minimum deprecation period:**
- **MAJOR version deprecation:** 90 days (3 months)
- **MINOR version deprecation:** 30 days (1 month)
- **PATCH version deprecation:** Immediate (fixes only)

### Deprecation Process

#### 1. Announce Deprecation

**In contract documentation:**
```yaml
deprecated: true
x-deprecation-date: "2026-05-01"
x-removal-date: "2026-08-01"
x-migration-guide: "https://docs.7dsolutions.com/migrations/ar-v1-to-v2"
description: |
  DEPRECATED: This endpoint will be removed on 2026-08-01.
  Use GET /api/invoices with Accept: v2 instead.
```

**In CHANGELOG:**
```markdown
## [1.5.0] - 2026-05-01

### Deprecated
- `GET /api/customers/{id}/legacy` - Use `GET /api/customers/{id}` instead.
  Will be removed in v2.0.0 (planned for 2026-08-01).
```

#### 2. Communication

- Post in #engineering Slack channel
- Email to consuming teams
- Update API documentation portal
- Add runtime warnings (if possible)

#### 3. Monitor Usage

Track deprecated API/event usage:
```
# Metrics
deprecated_api_calls_total{endpoint="/api/legacy", version="v1"}
deprecated_event_consumed_total{event="ar.invoice.issued", version="v1"}
```

#### 4. Remove After Window

After deprecation window expires:
- Remove deprecated endpoints/fields
- Stop publishing deprecated events
- Bump to next MAJOR version

---

## Dual-Publish & Dual-Consume

During major version transitions, support both old and new versions simultaneously.

### Dual-Publish (Event Producers)

**Publish both v1 and v2 events** during migration:

```rust
// Publish v1 event for legacy consumers
let v1_event = InvoiceIssuedV1 {
    invoice_id: invoice.id,
    customer_id: invoice.customer_id,
    amount_due_minor: invoice.amount,
    currency: "USD".to_string(),
};
event_bus.publish("ar.events.ar.invoice.issued", v1_event).await?;

// Publish v2 event for new consumers
let v2_event = InvoiceIssuedV2 {
    invoice_id: invoice.id,
    customer: CustomerRef {
        id: invoice.customer_id,
        name: invoice.customer_name,
    },
    amount: Money {
        value_minor: invoice.amount,
        currency: "USD".to_string(),
    },
};
event_bus.publish("ar.events.ar.invoice.issued", v2_event).await?;
```

**Both events:**
- Use same NATS subject
- Have different `source_version` in envelope
- Contain same business event data (different structure)

### Dual-Consume (Event Consumers)

**Accept both v1 and v2 events** during migration:

```rust
async fn handle_invoice_issued(envelope: EventEnvelope) -> Result<(), Error> {
    let major_version = envelope.source_version
        .split('.')
        .next()
        .unwrap_or("1");

    match major_version {
        "1" => {
            let event: InvoiceIssuedV1 = serde_json::from_value(envelope.payload)?;
            process_v1_event(event).await
        }
        "2" => {
            let event: InvoiceIssuedV2 = serde_json::from_value(envelope.payload)?;
            process_v2_event(event).await
        }
        _ => Err(Error::UnsupportedVersion(envelope.source_version)),
    }
}
```

### Migration Timeline

**Phase 1: Preparation (Week 0)**
- Implement v2 schema
- Add v2 handling to consumers (dual-consume)
- Test in staging

**Phase 2: Dual-Publish (Weeks 1-12)**
- Producer emits both v1 and v2
- All consumers handle both versions
- Monitor for errors

**Phase 3: Deprecation Announcement (Week 4)**
- Announce v1 deprecation
- Set removal date (Week 12)
- Update documentation

**Phase 4: Consumer Migration (Weeks 4-11)**
- Teams update consumers to prefer v2
- Remove v1 handling code
- Verify no v1 consumption

**Phase 5: Stop Dual-Publish (Week 12)**
- Producer stops emitting v1
- Only v2 events published
- Monitor for 1 week

**Phase 6: Cleanup (Week 13)**
- Remove v1 schema from repo
- Archive v1 documentation
- Update contract tests

---

## Golden Examples & Contract Tests

### Golden Examples

Every schema MUST include at least one golden example.

#### Event Schema Example

**In `contracts/events/ar-invoice-issued.v1.json`:**
```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://7dsolutions.io/schemas/events/ar-invoice-issued.v1.json",
  "title": "ar.invoice.issued",
  "description": "Event emitted when an AR invoice is created and issued",
  "type": "object",
  "examples": [
    {
      "event_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
      "occurred_at": "2026-02-12T19:30:00Z",
      "tenant_id": "tenant-123",
      "source_module": "ar",
      "source_version": "1.2.3",
      "correlation_id": "corr-456",
      "payload": {
        "invoice_id": "inv-789",
        "customer_id": "cust-101",
        "amount_due_minor": 10000,
        "currency": "USD",
        "due_date": "2026-03-15"
      }
    }
  ],
  "properties": {
    ...
  }
}
```

#### OpenAPI Example

**In `contracts/ar/ar-v1.0.0.yaml`:**
```yaml
paths:
  /api/invoices:
    post:
      summary: Create new invoice
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/CreateInvoiceRequest'
            examples:
              basic:
                summary: Basic invoice
                value:
                  customer_id: "cust-123"
                  amount_minor: 10000
                  currency: "USD"
                  due_date: "2026-03-15"
              with_line_items:
                summary: Invoice with line items
                value:
                  customer_id: "cust-123"
                  line_items:
                    - description: "Consulting services"
                      amount_minor: 10000
                  currency: "USD"
                  due_date: "2026-03-15"
```

### Contract Tests

**When schema changes:**

1. **Update golden examples** - Add/modify examples to cover new fields
2. **Update contract tests** - Ensure tests validate new schema
3. **Test backwards compatibility** - Verify old consumers still work

#### Contract Test Structure

```
tools/contract-tests/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   └── tests/
│       ├── ar_events.rs          # AR event schema tests
│       ├── payments_events.rs     # Payments event schema tests
│       └── openapi_validation.rs  # OpenAPI spec validation
```

#### Example Contract Test

```rust
// tools/contract-tests/src/tests/ar_events.rs

#[test]
fn test_ar_invoice_issued_v1_schema() {
    let schema_path = "contracts/events/ar-invoice-issued.v1.json";
    let schema = load_json_schema(schema_path).expect("Failed to load schema");

    // Validate schema is valid JSON Schema
    assert!(schema.get("$schema").is_some());

    // Validate examples exist
    let examples = schema.get("examples").expect("Schema must have examples");
    assert!(examples.as_array().unwrap().len() > 0);

    // Validate each example against schema
    for example in examples.as_array().unwrap() {
        validate_against_schema(&schema, example).expect("Example must validate");
    }
}

#[test]
fn test_ar_invoice_issued_backwards_compatibility() {
    // Load v1 and v2 schemas
    let v1 = load_json_schema("contracts/events/ar-invoice-issued.v1.json").unwrap();
    let v2 = load_json_schema("contracts/events/ar-invoice-issued.v2.json").unwrap();

    // Verify v1 examples still validate against v1 schema
    let v1_examples = v1.get("examples").unwrap().as_array().unwrap();
    for example in v1_examples {
        validate_against_schema(&v1, example).expect("v1 example must validate");
    }

    // Verify v2 is a superset of v1 (non-breaking)
    // This test would fail if v2 removes required fields from v1
    check_schema_compatibility(&v1, &v2).expect("v2 must be compatible with v1");
}
```

---

## CI Enforcement

### Schema Change Validation

GitHub Actions CI checks enforce contract versioning rules on every PR.

#### 1. Version Bump Check

**Fail if schema changes without version bump:**

```yaml
# .github/workflows/contract-validation.yml
- name: Check schema version bump
  run: |
    # Get changed schema files
    changed_schemas=$(git diff --name-only origin/main...HEAD | grep 'contracts/events/.*\.json')

    for schema in $changed_schemas; do
      # Check if $id version was bumped
      old_version=$(git show origin/main:$schema | jq -r '."$id"' | grep -oP 'v\d+\.\d+')
      new_version=$(cat $schema | jq -r '."$id"' | grep -oP 'v\d+\.\d+')

      if [ "$old_version" == "$new_version" ]; then
        echo "ERROR: Schema $schema changed but version not bumped"
        echo "Old: $old_version, New: $new_version"
        exit 1
      fi
    done
```

#### 2. Examples Validation

**Fail if examples missing or invalid:**

```yaml
- name: Validate schema examples
  run: |
    for schema in contracts/events/*.json; do
      # Check examples exist
      if ! jq -e '.examples' $schema > /dev/null; then
        echo "ERROR: Schema $schema missing examples field"
        exit 1
      fi

      # Validate each example against schema
      python3 tools/validate-examples.py $schema
    done
```

#### 3. Breaking Change Detection

**Warn on potential breaking changes:**

```yaml
- name: Detect breaking changes
  run: |
    # Compare schemas and detect breaking changes
    python3 tools/detect-breaking-changes.py \
      --old origin/main \
      --new HEAD \
      --schemas 'contracts/events/*.json'

    # If breaking changes detected, require MAJOR version bump
```

#### 4. Changelog Check

**Fail if CHANGELOG not updated:**

```yaml
- name: Check CHANGELOG updated
  run: |
    # Check if contract files changed
    contract_changes=$(git diff --name-only origin/main...HEAD | grep '^contracts/')

    if [ -n "$contract_changes" ]; then
      # Require CHANGELOG.md update
      if ! git diff --name-only origin/main...HEAD | grep -q 'CHANGELOG.md'; then
        echo "ERROR: Contract files changed but CHANGELOG.md not updated"
        echo "Changed files:"
        echo "$contract_changes"
        exit 1
      fi
    fi
```

### CI Workflow Complete Example

```yaml
name: Contract Validation

on:
  pull_request:
    paths:
      - 'contracts/**'

jobs:
  validate-contracts:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0  # Need history for comparison

      - name: Validate JSON schemas
        run: |
          for schema in contracts/events/*.json; do
            if ! jq empty $schema 2>/dev/null; then
              echo "ERROR: Invalid JSON in $schema"
              exit 1
            fi
          done

      - name: Check version bumps
        run: ./tools/ci/check-contract-versions.sh

      - name: Validate examples
        run: ./tools/ci/validate-contract-examples.sh

      - name: Detect breaking changes
        run: ./tools/ci/detect-breaking-changes.sh

      - name: Check CHANGELOG updated
        run: ./tools/ci/check-changelog-updated.sh

      - name: Run contract tests
        run: |
          cd tools/contract-tests
          cargo test --verbose
```

---

## Migration Checklist

Use this checklist when introducing a new contract version:

### For Event Schema Changes

**Planning Phase:**
- [ ] Determine if change is breaking or non-breaking
- [ ] Choose version bump (MAJOR, MINOR, or PATCH)
- [ ] Write migration guide document
- [ ] Communicate change to consuming teams

**Implementation Phase:**
- [ ] Create new schema file (if MAJOR) or update existing (if MINOR/PATCH)
- [ ] Add golden examples to new schema
- [ ] Update contract tests to cover new schema
- [ ] Implement dual-publish in producer (if MAJOR)
- [ ] Implement dual-consume in all consumers (if MAJOR)
- [ ] Add schema version handling logic

**Testing Phase:**
- [ ] Test v1 examples still validate against v1 schema
- [ ] Test v2 examples validate against v2 schema
- [ ] Test backwards compatibility (if non-breaking)
- [ ] Run contract tests in CI
- [ ] Test dual-publish/dual-consume in staging

**Release Phase:**
- [ ] Update CHANGELOG.md with deprecation notice
- [ ] Announce deprecation in #engineering
- [ ] Set deprecation window (90 days for MAJOR)
- [ ] Deploy dual-publish to production
- [ ] Monitor consumer migration progress

**Cleanup Phase (After Deprecation Window):**
- [ ] Verify all consumers migrated to new version
- [ ] Stop publishing old version
- [ ] Remove old schema handling code
- [ ] Archive old schema documentation
- [ ] Update contract tests to remove old version

### For OpenAPI Changes

**Planning Phase:**
- [ ] Determine if change is breaking or non-breaking
- [ ] Choose version bump (MAJOR, MINOR, or PATCH)
- [ ] Write migration guide
- [ ] Communicate to API consumers

**Implementation Phase:**
- [ ] Create new OpenAPI file (if MAJOR) or update existing (if MINOR/PATCH)
- [ ] Add request/response examples
- [ ] Implement version negotiation (Accept header)
- [ ] Support both v1 and v2 endpoints (if MAJOR)
- [ ] Update API documentation

**Testing Phase:**
- [ ] Test v1 requests still work
- [ ] Test v2 requests work as expected
- [ ] Test version negotiation via Accept header
- [ ] Run contract tests

**Release Phase:**
- [ ] Update CHANGELOG.md
- [ ] Announce deprecation
- [ ] Deploy version negotiation
- [ ] Monitor API version usage metrics

**Cleanup Phase:**
- [ ] Verify all clients migrated
- [ ] Remove old API version
- [ ] Archive old documentation

---

## See Also

- [Contract Standard](CONTRACT-STANDARD.md) - Contract structure and organization
- [Contract Testing Standard](CONTRACT-TESTING-STANDARD.md) - Testing requirements
- [Versioning Standard](VERSIONING-STANDARD.md) - Module and product versioning
- [Release Policy](../governance/RELEASE-POLICY.md) - Release process and cadence
