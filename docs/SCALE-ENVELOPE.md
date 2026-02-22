# Scale Envelope — 7D Solutions Platform

**Phase:** P50-020
**Bead:** bd-24h9
**Date:** 2026-02-22
**Status:** Codified from architecture review + k6 scenario design (P50-010)

---

## 1. Tested Envelope

The multi-tenant scale scenario (`tools/perf/scale_multitenant.js`) defines the boundary conditions under which this envelope was established.

| Parameter             | Value                                                          |
|-----------------------|----------------------------------------------------------------|
| Tenants               | 5 (round-robin across VUs)                                     |
| Max concurrent VUs    | 20 (read burst), 5 (billing runs), 10 (webhook burst)          |
| Total wall time       | ~165 s                                                         |
| Read sustain duration | 60 s at 20 VUs                                                 |
| Billing sustain       | 60 s at 5 VUs (overlaps with read phase)                       |
| Webhook sustain       | 30 s at 10 VUs                                                 |
| Billing mode          | Safe / idempotent (far-future period `2099-01`, no real charges)|
| Webhook mode          | HMAC-signed fake `payment_intent` IDs (0 DB rows mutated)      |
| Environment           | Local Docker Compose stack; staging at `STAGING_HOST`          |

---

## 2. Pass/Fail Thresholds

### 2a. k6 Hard Gates (exits non-zero on breach)

| Metric                   | p95 SLO     | p99 Ceiling | Notes                                        |
|--------------------------|-------------|-------------|----------------------------------------------|
| `http_req_duration`      | < 2 000 ms  | < 4 000 ms  | All requests, wall-clock                     |
| `scale_cp_reads_ms`      | < 500 ms    | < 1 000 ms  | Control-plane read endpoints                 |
| `scale_ar_reads_ms`      | < 800 ms    | < 1 500 ms  | AR module reads (customers, invoices, aging) |
| `scale_billing_run_ms`   | < 3 000 ms  | < 5 000 ms  | POST /platform-billing-runs (multi-service)  |
| `scale_webhook_ms`       | < 500 ms    | < 1 000 ms  | Tilled webhook ingest path                   |
| `http_req_failed`        | rate < 1%   | —           | All HTTP errors (4xx/5xx)                    |
| `scale_errors`           | rate < 1%   | —           | check() failures across all phases           |
| `scale_webhook_errors`   | rate < 1%   | —           | Webhook-specific: 401/5xx = HMAC mismatch or service down |

### 2b. Projection Lag (Prometheus — manual check after run)

Metric: `payments_event_consumer_lag_messages`
Queried in `teardown()` from `PERF_PROMETHEUS_URL`; logged with severity level.

| Level    | Threshold            | Action                                                     |
|----------|----------------------|------------------------------------------------------------|
| OK       | < 50 messages        | Consumer draining normally; no action required             |
| WARNING  | 50 – 200 messages    | Consumer falling behind; review NATS consumer config       |
| CRITICAL | > 200 messages       | Consumer overloaded; scale payments service or tune batch size |

> **Why not a k6 hard gate?** Projection lag is a Prometheus pull metric scraped after VUs finish. k6 thresholds only apply to in-test custom metrics. Lag is logged with severity by `teardown()` and must be verified in CI output or Grafana before marking the run green.

---

## 3. Observed Bottlenecks

These are architectural findings from the load shape design. Update with empirical numbers from actual staging runs.

### 3a. AR Aging Report (`GET /api/ar/aging`)

**Risk:** High. The aging report aggregates across all invoices for the authenticated platform admin. At 20 concurrent VUs all issuing this query, it competes for DB read bandwidth. It is the heaviest read in the scenario.

**Observed behavior (design-time):** Expected to approach the p95 = 800 ms ceiling first when tenant invoice counts grow. AR reads are the most likely threshold to breach as data volume increases.

### 3b. Billing Run Fanout (`POST /api/control/platform-billing-runs`)

**Risk:** Medium. Each call fans out: control-plane → tenant-registry lookup → AR write. At 5 VUs hitting this concurrently, write pressure on the AR Postgres instance is the limiting factor. The idempotency guard returns early on duplicate periods, so repeated runs are cheap. The first run for a new period is expensive.

**Observed behavior (design-time):** The p95 = 3 000 ms gate is generous to accommodate the multi-service round-trip. At > 10 concurrent billing-run VUs, DB write contention in AR is expected to breach this gate.

### 3c. Projection Lag Under Webhook Burst

**Risk:** Medium. The payments NATS consumer processes webhook events sequentially per payment intent. At 10 VUs firing webhooks with random fake IDs (no DB mutation), the consumer processes each event cheaply. If real payment IDs were used and DB updates were heavy, consumer lag would accumulate.

**Observed behavior (design-time):** With fake IDs (0-row updates), lag should remain near zero. If the consumer group is misconfigured (e.g., max-pending too low), lag accumulates even on cheap events.

---

## 4. Safe Operating Limits

Based on the tested envelope and architectural analysis.

| Dimension                    | Safe Limit        | Rationale                                                      |
|------------------------------|-------------------|----------------------------------------------------------------|
| Concurrent read VUs          | ≤ 20              | Tested at 20; AR aging is the binding constraint               |
| Concurrent billing-run VUs   | ≤ 5               | AR write contention escalates sharply above 5                  |
| Concurrent webhook VUs       | ≤ 10              | HMAC validation is CPU-cheap; consumer lag is the risk         |
| Active tenants per billing run | ≤ 5             | Tested at 5; each adds one AR write per cycle                  |
| Projection lag ceiling       | < 200 messages    | Beyond 200, consumer cannot drain before next webhook burst    |
| Webhook failure rate         | < 1%              | Above 1% indicates HMAC mismatch or secret rotation in progress|
| AR aging p99 latency         | < 1 500 ms        | Beyond 1 500 ms, the read path is under DB read pressure       |

**Do not exceed these limits on the current single-VPS staging stack without first establishing a new baseline run and updating this document.**

---

## 5. Mitigation Notes

> Scope: config and runtime limits only. No feature work.

### M-1: AR Aging Query Performance

If `scale_ar_reads_ms` p99 exceeds 1 500 ms in a staging run, add a compound index on `(tenant_id, status, due_date)` to the AR invoices table via a migration. This index is already present in the AR schema design; verify it is not missing in the deployed migration set.

### M-2: Billing Run Concurrency Cap

If billing-run errors appear above 1% at > 5 concurrent VUs, configure the control-plane to enforce a soft concurrency limit via a Postgres advisory lock or a semaphore-style counter in Redis (if available). At the current scale (≤ 5 tenants), the idempotency guard is sufficient.

### M-3: Projection Lag — Consumer Batch Size

If teardown logs `WARNING` or `CRITICAL` projection lag, increase the NATS consumer `max_ack_pending` (or equivalent) from the default to 200 messages. This allows the payments consumer to prefetch more events without stalling. Change in `docker-compose.modules.yml` or the relevant NATS consumer configuration.

---

## 6. How to Update This Document

After each staging run:

1. Export k6 summary: `--summary-export=scale_multitenant_summary.json`
2. Record actual p95/p99 values in a new row in the table below.
3. Update **Section 4** if limits need revision.
4. Commit with `[bd-xxx] P50: Update SCALE-ENVELOPE with run YYYY-MM-DD`.

### Run History

| Date       | VUs (peak) | Tenants | CP p95  | AR p95  | Billing p95 | Webhook p95 | Lag (peak) | Result |
|------------|-----------|---------|---------|---------|-------------|-------------|------------|--------|
| _baseline_ | 20        | 5       | < 500ms | < 800ms | < 3 000ms   | < 500ms     | < 50 msg   | (design-time targets — no run yet) |
