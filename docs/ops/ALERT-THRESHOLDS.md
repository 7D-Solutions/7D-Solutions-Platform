# Alert Thresholds

**Phase 16: Operational Readiness**

This document defines alert thresholds for the 7D Solutions Platform. All thresholds are derived from explicit SLO (Service Level Objective) constraints and production failure modes.

## Philosophy

Alerts must:
1. **Be actionable** - Every page requires human intervention
2. **Have clear severity** - Warning vs. Critical distinction guides response urgency
3. **Link to SLOs** - Thresholds derive from business commitments, not arbitrary numbers
4. **Prevent alert fatigue** - Warnings for early intervention, critical for immediate response

## Alert Categories

### 1. UNKNOWN Protocol Violations

**What**: UNKNOWN state indicates business logic uncertainty requiring resolution before retry

**SLO**: UNKNOWN duration should not exceed retry windows (max 6 hours for billing)

#### Thresholds

| Metric | Warning | Critical | Rationale |
|--------|---------|----------|-----------|
| `unknown_invoice_age_seconds` | > 3600 (1 hour) | > 14400 (4 hours) | Billing cycles must resolve within 6-hour window; 4-hour alert allows 2-hour resolution time |
| `unknown_payment_age_seconds` | > 1800 (30 min) | > 3600 (1 hour) | Payment UNKNOWNs block customer experience; faster resolution required |
| `unknown_subscription_age_seconds` | > 3600 (1 hour) | > 14400 (4 hours) | Subscription state drift impacts billing accuracy |

**Response**:
- **Warning**: Investigate root cause, prepare manual intervention
- **Critical**: Manual resolution required immediately; customer impact imminent

---

### 2. Retry Exhaustion

**What**: Events that have exhausted all retry attempts and landed in DLQ

**SLO**: DLQ arrival rate should trend toward zero; non-zero indicates systemic issues

#### Thresholds

| Metric | Warning | Critical | Rationale |
|--------|---------|----------|-----------|
| `dlq_events_total` (rate) | > 5/min | > 20/min | Occasional retries acceptable; sustained failures indicate platform degradation |
| `dlq_events_by_subject{subject="gl.posting.requested"}` | > 1/min | > 5/min | GL posting failures break financial integrity; low tolerance |
| `dlq_events_by_subject{subject="invoice.finalization.requested"}` | > 2/min | > 10/min | Invoice failures impact customer billing |
| `dlq_events_by_subject{subject="payment.processing.requested"}` | > 1/min | > 5/min | Payment failures block revenue collection |

**Response**:
- **Warning**: Review DLQ contents, identify patterns, prepare hotfix
- **Critical**: Halt upstream producers if possible; all-hands incident response

---

### 3. Invariant Violations

**What**: Database-backed assertions about module state integrity

**SLO**: Invariant violations indicate data corruption; zero tolerance in production

#### Thresholds

| Metric | Warning | Critical | Rationale |
|--------|---------|----------|-----------|
| `ar_invariant_violations_total` | > 0 | > 10 | Any violation is concerning; sustained violations require rollback |
| `subscriptions_invariant_violations_total` | > 0 | > 10 | Billing state corruption impacts revenue accuracy |
| `gl_invariant_violations_total` | > 0 | > 5 | Financial integrity violations are highest severity; low threshold |
| `payments_invariant_violations_total` | > 0 | > 10 | Payment state drift can cause double-charging or revenue loss |

**Invariant Types** (Phase 15):
- `no_unknown_outside_retry_window`: UNKNOWNs older than 6 hours
- `no_duplicate_invoices_per_cycle`: Multiple invoices for same billing cycle
- `no_retroactive_state_changes`: State mutations outside valid lifecycle windows
- `no_orphaned_finalization_attempts`: Finalization attempts without invoice records

**Response**:
- **Warning**: Immediate investigation; freeze related deployments
- **Critical**: Emergency rollback; halt affected workflows; on-call escalation

---

### 4. Outbox Atomicity Failures

**What**: Domain mutations without corresponding outbox events (state drift)

**SLO**: Outbox atomicity must be 100%; failures indicate transaction bugs

#### Thresholds

| Metric | Warning | Critical | Rationale |
|--------|---------|----------|-----------|
| `outbox_insert_failures_total` | > 0 | > 1 | Outbox failures risk state drift; zero tolerance |
| `state_mutations_without_events` | > 0 | > 1 | Calculated as domain writes minus outbox writes; drift detector |

**Response**:
- **Warning**: Verify transaction boundaries in code; prepare patch
- **Critical**: Rollback deployment; manual event emission may be required

---

### 5. Cross-Module Event Flow

**What**: Events emitted by upstream modules not consumed by downstream

**SLO**: Event delivery latency should be < 5 seconds p99

#### Thresholds

| Metric | Warning | Critical | Rationale |
|--------|---------|----------|-----------|
| `event_processing_lag_seconds{consumer="gl-posting"}` | > 30 | > 300 (5 min) | GL lag delays financial reconciliation |
| `event_processing_lag_seconds{consumer="payment-succeeded"}` | > 10 | > 60 | Payment lag delays invoice finalization |
| `nats_consumer_pending_messages` | > 1000 | > 10000 | Backlog indicates consumer degradation |

**Response**:
- **Warning**: Scale consumers, investigate slow queries
- **Critical**: Event bus degradation; check NATS health, scale infrastructure

---

### 6. Database Connection Pool Exhaustion

**What**: All database connections in use; queries blocked

**SLO**: Connection pool utilization should stay below 80%

#### Thresholds

| Metric | Warning | Critical | Rationale |
|--------|---------|----------|-----------|
| `db_pool_utilization_percent` | > 80% | > 95% | High utilization risks query timeouts |
| `db_connection_wait_time_seconds` | > 1.0 | > 5.0 | Waiting for connections indicates pool saturation |

**Response**:
- **Warning**: Review slow queries, increase pool size if needed
- **Critical**: Scale database or kill long-running queries

---

### 7. Period Close Integrity

**What**: Accounting period close snapshots must be immutable and complete

**SLO**: Period close must complete within 30 minutes; failures block reporting

#### Thresholds

| Metric | Warning | Critical | Rationale |
|--------|---------|----------|-----------|
| `period_close_duration_seconds` | > 1800 (30 min) | > 3600 (1 hour) | Prolonged close indicates data volume growth or query degradation |
| `period_close_failures_total` | > 0 | > 2 | Period close failures block financial statements |
| `period_close_snapshot_errors` | > 0 | > 0 | Snapshot errors indicate data corruption; zero tolerance |

**Response**:
- **Warning**: Optimize close queries, review data growth
- **Critical**: Manual intervention required; delay reporting until resolved

---

## Alert Configuration Examples

### Prometheus Alert Rules

```yaml
groups:
  - name: unknown_protocol
    interval: 60s
    rules:
      - alert: UnknownInvoiceDurationCritical
        expr: unknown_invoice_age_seconds > 14400
        for: 5m
        labels:
          severity: critical
          module: ar
        annotations:
          summary: "Invoice in UNKNOWN state for >4 hours"
          description: "Invoice {{ $labels.invoice_id }} has been UNKNOWN for {{ $value }}s (>4h threshold)"

      - alert: UnknownInvoiceDurationWarning
        expr: unknown_invoice_age_seconds > 3600
        for: 5m
        labels:
          severity: warning
          module: ar
        annotations:
          summary: "Invoice in UNKNOWN state for >1 hour"
          description: "Invoice {{ $labels.invoice_id }} approaching retry window limit"

  - name: invariant_violations
    interval: 30s
    rules:
      - alert: GLInvariantViolationCritical
        expr: increase(gl_invariant_violations_total[5m]) > 5
        labels:
          severity: critical
          module: gl
        annotations:
          summary: "Critical GL invariant violations detected"
          description: "{{ $value }} GL invariant violations in last 5 minutes; financial integrity at risk"

      - alert: ARInvariantViolationWarning
        expr: increase(ar_invariant_violations_total[5m]) > 0
        labels:
          severity: warning
          module: ar
        annotations:
          summary: "AR invariant violations detected"
          description: "{{ $value }} AR invariant violations detected; investigate immediately"

  - name: dlq_exhaustion
    interval: 60s
    rules:
      - alert: DLQEventRateCritical
        expr: rate(dlq_events_total[5m]) > 20/60
        for: 2m
        labels:
          severity: critical
        annotations:
          summary: "Critical DLQ event rate: >20/min"
          description: "Sustained retry exhaustion indicates platform failure"

      - alert: GLPostingDLQCritical
        expr: rate(dlq_events_by_subject{subject="gl.posting.requested"}[5m]) > 5/60
        for: 1m
        labels:
          severity: critical
          module: gl
        annotations:
          summary: "GL posting events exhausting retries"
          description: "Financial integrity loop broken; immediate intervention required"
```

---

## Escalation Policy

| Severity | Response Time | Escalation Path |
|----------|---------------|-----------------|
| **Critical** | < 15 minutes | On-call engineer → Engineering lead → CTO |
| **Warning** | < 2 hours | On-call engineer → Team lead (business hours) |

---

## Runbooks

Each alert should link to a runbook:
- `docs/ops/runbooks/UNKNOWN-RESOLUTION.md` - Manual UNKNOWN resolution
- `docs/ops/runbooks/DLQ-REPLAY.md` - DLQ event replay procedures
- `docs/ops/runbooks/INVARIANT-INVESTIGATION.md` - Invariant violation root cause analysis
- `docs/ops/runbooks/PERIOD-CLOSE-RECOVERY.md` - Period close failure recovery

---

## Monitoring Coverage

**Metrics to collect** (Phase 16):
- Counter: `{module}_invariant_violations_total` (all 5 modules)
- Histogram: `unknown_{entity}_age_seconds` (invoices, payments, subscriptions)
- Counter: `dlq_events_total{subject, reason}`
- Gauge: `db_pool_utilization_percent{module}`
- Histogram: `event_processing_lag_seconds{consumer}`
- Histogram: `period_close_duration_seconds`

**Export endpoints**: `/metrics` available on all 5 modules (AR, GL, Payments, Subscriptions, Notifications)

---

## Review Schedule

This document should be reviewed:
- **Quarterly**: Adjust thresholds based on production experience
- **After incidents**: Update based on alert effectiveness
- **Before major releases**: Verify new features have appropriate alerts

---

**Document Owner**: Platform Team
**Last Updated**: 2026-02-16 (Phase 16)
**Next Review**: 2026-05-16
