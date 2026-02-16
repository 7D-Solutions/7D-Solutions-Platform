# Retention Classes

**Phase 16: Operational Readiness - Data Lifecycle Management**

This document declares retention classes for all domain concept categories across the 7D Solutions Platform. Every data type has a defined lifecycle to prevent uncontrolled storage growth and ensure compliance with data retention policies.

## Retention Class Taxonomy

### Permanent (Forever)
**Definition**: Data required for financial, legal, or regulatory compliance. Never deleted.

**Characteristics**:
- Immutable after creation
- Subject to audit requirements
- Required for tax reporting, financial reconciliation, compliance investigations

**Examples**:
- Journal entries (GL)
- Financial statements
- Period close snapshots
- Audit trails

---

### Long-Term (7 years)
**Definition**: Business-critical data with legal retention requirements. Retained for statute of limitations.

**Characteristics**:
- May transition to archival storage after 2 years
- Required for dispute resolution, tax audits
- Accessibility can degrade over time (e.g., cold storage)

**Examples**:
- Invoices (AR)
- Payment records
- Customer contracts
- Subscription history

---

### Medium-Term (1 year)
**Definition**: Operational data supporting active business processes. Retained for operational needs.

**Characteristics**:
- Supports reporting, analytics, and debugging
- Can be purged after operational relevance expires
- May be aggregated/summarized before deletion

**Examples**:
- Event bus messages (NATS)
- Notification delivery logs
- API request logs
- Performance metrics (detailed)

---

### Short-Term (90 days)
**Definition**: Ephemeral operational data with temporary relevance.

**Characteristics**:
- Used for immediate troubleshooting
- High volume, low long-term value
- Aggressive purge policy to control costs

**Examples**:
- DLQ events (after resolution)
- Debugging traces
- Session tokens
- Rate limiting counters

---

## Module-Specific Retention Classes

### AR (Accounts Receivable)

| Domain Concept | Retention Class | Rationale |
|----------------|-----------------|-----------|
| `invoices` | **Long-Term (7 years)** | Tax reporting, dispute resolution |
| `invoice_line_items` | **Long-Term (7 years)** | Itemized billing detail for audits |
| `invoice_finalization_attempts` | **Medium-Term (1 year)** | Operational debugging; superseded by invoice state |
| `idempotency_keys` | **Medium-Term (1 year)** | Prevent duplicate operations; can purge after invoice lifecycle complete |
| `ar_outbox_events` | **Medium-Term (1 year)** | Event provenance; purge after downstream consumption confirmed |

---

### Payments

| Domain Concept | Retention Class | Rationale |
|----------------|-----------------|-----------|
| `payment_intents` | **Long-Term (7 years)** | Financial transaction record; compliance |
| `payment_method_records` | **Long-Term (7 years)** | PCI compliance; fraud investigation |
| `payment_gateway_logs` | **Medium-Term (1 year)** | Reconciliation; purge after confirmed settlement |
| `payment_webhooks_raw` | **Short-Term (90 days)** | Debugging; superseded by processed payment state |
| `payments_outbox_events` | **Medium-Term (1 year)** | Event provenance |

---

### Subscriptions

| Domain Concept | Retention Class | Rationale |
|----------------|-----------------|-----------|
| `subscriptions` | **Long-Term (7 years)** | Contract record; revenue recognition |
| `subscription_state_history` | **Long-Term (7 years)** | Lifecycle audit trail; churn analysis |
| `billing_cycles` | **Long-Term (7 years)** | Links subscription to invoices; financial reconciliation |
| `cycle_attempts` | **Medium-Term (1 year)** | Retry/idempotency; can purge after cycle success |
| `bill_runs` | **Medium-Term (1 year)** | Operational record; summarized in invoices |
| `subscriptions_outbox_events` | **Medium-Term (1 year)** | Event provenance |

---

### GL (General Ledger)

| Domain Concept | Retention Class | Rationale |
|----------------|-----------------|-----------|
| `journal_entries` | **Permanent (Forever)** | Financial record of record; tax, audit, compliance |
| `journal_lines` | **Permanent (Forever)** | Itemized GL detail; cannot delete without parent entry |
| `account_balances` | **Permanent (Forever)** | Derived from journal entries; verifiable rebuild required |
| `chart_of_accounts` | **Permanent (Forever)** | Defines account structure; historical reporting requires immutability |
| `accounting_periods` | **Permanent (Forever)** | Time boundaries for financial reporting |
| `period_close_snapshots` | **Permanent (Forever)** | Immutable snapshot; regulatory requirement |
| `gl_outbox_events` | **Medium-Term (1 year)** | Event provenance; purge after confirmed consumption |
| `gl_processed_events` | **Medium-Term (1 year)** | Idempotency; can purge after journal entry immutability confirmed |

---

### Notifications

| Domain Concept | Retention Class | Rationale |
|----------------|-----------------|-----------|
| `notification_delivery_logs` | **Medium-Term (1 year)** | Delivery confirmation; customer support |
| `notification_templates` | **Long-Term (7 years)** | Audit trail of customer communications |
| `email_send_logs` | **Short-Term (90 days)** | Debugging; superseded by delivery status |
| `sms_send_logs` | **Short-Term (90 days)** | Debugging; high volume, low long-term value |
| `notifications_outbox_events` | **Short-Term (90 days)** | Notifications are stateless; no long-term event provenance needed |

---

## Cross-Module Shared Data

| Domain Concept | Retention Class | Rationale |
|----------------|-----------------|-----------|
| `event_bus_messages` (NATS) | **Medium-Term (1 year)** | Event replay capability; purge after consumption confirmed |
| `dlq_events` | **Medium-Term (1 year)** | Incident investigation; purge after resolution |
| `idempotency_keys` (global) | **Medium-Term (1 year)** | Prevent duplicate operations; purge after operation finality |
| `correlation_ids` (audit trail) | **Permanent (Forever)** | Cross-module transaction tracing; compliance requirement |

---

## Archival Strategy

### Hot Storage (Active)
- **Duration**: 0-90 days
- **Access**: Real-time queries
- **Cost**: High performance SSD

### Warm Storage (Recent)
- **Duration**: 90 days - 2 years
- **Access**: Occasional queries (analytics, reporting)
- **Cost**: Standard SSD

### Cold Storage (Archival)
- **Duration**: 2-7 years
- **Access**: Rare (compliance, audits)
- **Cost**: Object storage (S3 Glacier)

### Permanent Storage
- **Duration**: Forever
- **Access**: Rare but required
- **Cost**: Immutable object storage with versioning

---

## Purge Automation

### Automated Purge Jobs

| Job Name | Schedule | Target | Retention Cutoff |
|----------|----------|--------|------------------|
| `purge-dlq-resolved` | Daily 2am UTC | `dlq_events` (resolved) | > 90 days |
| `purge-outbox-consumed` | Weekly Sunday 2am UTC | `*_outbox_events` (published) | > 1 year |
| `purge-notification-logs` | Daily 3am UTC | `email_send_logs`, `sms_send_logs` | > 90 days |
| `archive-invoices-warm` | Monthly 1st 3am UTC | `invoices` | 90 days - 2 years |
| `archive-payments-cold` | Monthly 1st 4am UTC | `payment_intents` | 2-7 years |

### Manual Review Required

Data requiring manual approval before deletion:
- Journal entries (never delete; flag only)
- Period close snapshots (never delete; flag only)
- Customer payment disputes (purge after legal resolution + 1 year)

---

## Compliance Alignment

### Regulatory Requirements

| Regulation | Retention Minimum | Applies To |
|------------|-------------------|------------|
| **SOX (Sarbanes-Oxley)** | 7 years | Journal entries, financial statements, audit trails |
| **IRS (US Tax Code)** | 7 years | Invoices, payments, revenue records |
| **GDPR (Right to Erasure)** | Customer request | Customer PII; conflicts resolved in favor of financial records |
| **PCI DSS** | 3 months (logs), 1 year (transactions) | Payment card data, access logs |

### GDPR Right to Erasure (Exceptions)

When a customer requests data deletion under GDPR Article 17, the following data **cannot** be deleted due to legal obligations:
- Invoices (tax reporting)
- Payment records (fraud prevention, chargebacks)
- Journal entries (financial compliance)

**Approach**: Pseudonymize customer PII in retained records while preserving financial integrity.

---

## Monitoring Retention Compliance

### Metrics to Track

- `retention_violations_total{class, module}` - Data exceeding retention class without archival
- `storage_growth_rate_gb_per_month{class}` - Growth rate by retention class
- `purge_job_failures_total{job_name}` - Failed purge jobs requiring investigation

### Alert Thresholds

| Metric | Warning | Critical |
|--------|---------|----------|
| `retention_violations_total` | > 100 records | > 1000 records |
| `purge_job_failures_total` | > 1 failure/week | > 3 failures/week |
| `storage_growth_rate_gb_per_month` | > 50 GB/month (Medium/Short) | > 100 GB/month |

---

## Review and Updates

This document should be reviewed:
- **Annually**: Align with updated regulations
- **Before new modules**: Define retention classes for new domain concepts
- **After compliance audits**: Incorporate findings

---

**Document Owner**: Platform Team + Legal
**Last Updated**: 2026-02-16 (Phase 16)
**Next Review**: 2027-02-16
