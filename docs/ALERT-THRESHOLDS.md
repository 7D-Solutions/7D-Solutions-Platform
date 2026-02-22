# Alert Thresholds — HTTP Latency & Error Rate SLOs

**Phase:** P48-020
**Bead:** bd-1zkv
**Status:** Pre-production baselines — update after first 24–72 h of real production traffic

> **Scope:** Per-endpoint HTTP latency and error-rate SLOs. For business-logic thresholds
> (UNKNOWN states, DLQ exhaustion, invariant violations, outbox atomicity) see
> `docs/ops/ALERT-THRESHOLDS.md`.
>
> Alert rules derived from this document live in
> `infra/monitoring/alerts/latency-slo.yml`.

---

## 1. Evidence Basis

These baselines are **pre-production**. They derive from:

| Source | Evidence |
|--------|----------|
| SCALE-ENVELOPE.md (P50-020) | k6 design-time targets — 20 VUs read, 5 VUs billing, 10 VUs webhook |
| Phase 45 proof gate | Container restarts healthy in 30–45 s; NATS reconnect < 10 s |
| Phase 46 staging drills | Payment UNKNOWN resolved via Tilled webhook in ~5 s test mode |
| Argon2 cost analysis | bcrypt-class hash ~50 ms on production-grade vCPU |

**Update procedure:** After 72 h of real production traffic, run the PromQL queries
in Section 6 against the 72-hour window and replace the "Observed" column with real
values. Commit with tag `[re-sample YYYY-MM-DD]`.

---

## 2. Auth / Session Endpoints

**Service:** `identity-auth` (`:8080`)
**Prometheus metric:** `http_request_duration_seconds{path, method, status}`

| Endpoint | Method | Pre-prod p95 | Pre-prod p99 | Rationale |
|----------|--------|-------------|-------------|-----------|
| `/api/auth/login` | POST | 100 ms | 200 ms | Argon2 ~50 ms + JWT sign + DB lookup |
| `/api/auth/refresh` | POST | 20 ms | 50 ms | JWT verify + DB lookup; no Argon2 |
| `/api/ready` (auth) | GET | 15 ms | 50 ms | Single `SELECT 1` DB ping |

**SLO alert thresholds:**

| Endpoint | Warning (p95 >) | `for:` | Critical (p95 >) | `for:` |
|----------|-----------------|--------|------------------|--------|
| `/api/auth/login` | 250 ms | 5 m | 500 ms | 2 m |
| `/api/auth/refresh` | 50 ms | 5 m | 150 ms | 2 m |
| `/api/ready` (auth) | 100 ms | 5 m | 300 ms | 2 m |

**Error rate:** Auth 5xx (server errors, excludes 4xx credential rejections)
- Warning: > 1 % over 5 m
- Critical: > 5 % over 2 m

---

## 3. TTP Billing Run Trigger

**Service:** `ttp` (`:8100`)
**Prometheus metric:** `ttp_http_request_duration_seconds{method, route, status}`

| Endpoint | Method | Pre-prod p95 | Pre-prod p99 | Rationale |
|----------|--------|-------------|-------------|-----------|
| `/api/ttp/billing-runs` | POST | 1 200 ms | 2 500 ms | Multi-service fanout: TTP → AR write; k6 gate < 3 000 ms |

**SLO alert thresholds:**

| Endpoint | Warning (p95 >) | `for:` | Critical (p95 >) | `for:` |
|----------|-----------------|--------|------------------|--------|
| `/api/ttp/billing-runs` | 3 000 ms | 5 m | 5 000 ms | 2 m |

Warning fires at the k6 design-time ceiling. Critical fires at the k6 p99 ceiling.
Both sustained windows are generous to absorb first-run fanout (idempotency guard is
cheap on repeated calls).

**Error rate (5xx):** Warning > 1 %, Critical > 5 % over 5 m / 2 m.

---

## 4. AR Webhook Route

**Service:** `ar` (`:8086`)
**Prometheus metric:** `ar_http_request_duration_seconds{method, route, status}`

| Endpoint | Method | Pre-prod p95 | Pre-prod p99 | Rationale |
|----------|--------|-------------|-------------|-----------|
| `/api/ar/webhooks/tilled` | POST | 200 ms | 400 ms | HMAC verify + DB write + outbox; k6 gate < 500 ms |

**SLO alert thresholds:**

| Endpoint | Warning (p95 >) | `for:` | Critical (p95 >) | `for:` |
|----------|-----------------|--------|------------------|--------|
| `/api/ar/webhooks/tilled` | 500 ms | 5 m | 1 000 ms | 2 m |

**Error rate (5xx):** Warning > 1 %, Critical > 5 % over 5 m / 2 m.
> 401 responses (HMAC mismatch) are 4xx and expected during secret rotation — do not
> count toward the 5xx error rate.

---

## 5. Payments Checkout Session

**Service:** `payments` (`:8088`)
**Prometheus metric:** `payments_http_request_duration_seconds{method, route, status}`

| Endpoint | Method | Pre-prod p95 | Pre-prod p99 | Rationale |
|----------|--------|-------------|-------------|-----------|
| `POST /api/payments/checkout-sessions` | POST | 200 ms | 400 ms | DB insert + outbox; Phase 46 payment proof |
| `GET /api/payments/checkout-sessions/:id` | GET | 50 ms | 100 ms | Single-row DB read |
| `POST /api/payments/checkout-sessions/:id` (Tilled webhook) | POST | 200 ms | 400 ms | HMAC verify + DB update + outbox |

**SLO alert thresholds:**

| Endpoint | Warning (p95 >) | `for:` | Critical (p95 >) | `for:` |
|----------|-----------------|--------|------------------|--------|
| Create checkout session | 500 ms | 5 m | 1 000 ms | 2 m |
| Tilled webhook ingest | 500 ms | 5 m | 1 000 ms | 2 m |

**Error rate (5xx):** Warning > 1 %, Critical > 5 % over 5 m / 2 m.

---

## 6. Instrumentation Gap — Control-Plane

**Service:** `control-plane` (`:8091`)

The control-plane is scraped by Prometheus (`job="control-plane"`) but **does not
expose a Prometheus HTTP latency histogram**. The `/metrics` endpoint is not
implemented. Available metric: `up{job="control-plane"}` only.

Affected endpoints without latency alerting:
- `GET /api/tenants` — tenant list
- `GET /api/tenants/:id` — tenant detail
- `POST /api/control/platform-billing-runs` — platform billing trigger

**Action required:** Add HTTP latency middleware to control-plane (similar to
`platform/identity-auth/src/middleware/metrics.rs`) and expose `GET /metrics`.
Create a follow-on bead for this instrumentation. Once added, define thresholds
at: CP reads p95 < 300 ms, CP billing trigger p95 < 3 000 ms (matching
SCALE-ENVELOPE targets).

---

## 7. Re-Sampling Ops Checklist

**Frequency:** Monthly (after first production traffic) or after any traffic pattern change.

### PromQL Queries — Run Against 72-Hour Window

```promql
# Auth login p95 (72h)
histogram_quantile(0.95,
  sum(rate(http_request_duration_seconds_bucket{path="/api/auth/login"}[72h])) by (le))

# Auth refresh p95 (72h)
histogram_quantile(0.95,
  sum(rate(http_request_duration_seconds_bucket{path="/api/auth/refresh"}[72h])) by (le))

# Auth /api/ready p95 (72h)
histogram_quantile(0.95,
  sum(rate(http_request_duration_seconds_bucket{path="/api/ready"}[72h])) by (le))

# TTP billing run p95 (72h)
histogram_quantile(0.95,
  sum(rate(ttp_http_request_duration_seconds_bucket{route="/api/ttp/billing-runs"}[72h])) by (le))

# AR webhook p95 (72h)
histogram_quantile(0.95,
  sum(rate(ar_http_request_duration_seconds_bucket{route="/api/ar/webhooks/tilled"}[72h])) by (le))

# Payments checkout create p95 (72h)
histogram_quantile(0.95,
  sum(rate(payments_http_request_duration_seconds_bucket{route="/api/payments/checkout-sessions", method="POST"}[72h])) by (le))

# Auth 5xx error rate (72h)
sum(rate(http_request_duration_seconds_count{status=~"5.."}[72h]))
/ sum(rate(http_request_duration_seconds_count[72h]))

# AR 5xx error rate (72h)
sum(rate(ar_http_requests_total{status=~"5.."}[72h]))
/ sum(rate(ar_http_requests_total[72h]))

# Payments 5xx error rate (72h)
sum(rate(payments_http_requests_total{status=~"5.."}[72h]))
/ sum(rate(payments_http_requests_total[72h]))

# TTP 5xx error rate (72h)
sum(rate(ttp_http_requests_total{status=~"5.."}[72h]))
/ sum(rate(ttp_http_requests_total[72h]))
```

### Update Procedure

1. Run the queries above in Grafana against the last 72 h.
2. Update the "Pre-prod p95 / p99" columns in Sections 2–5 with observed values.
3. Recalculate alert thresholds: warning = 2× observed p95, critical = 4× observed p95
   (or observed p99, whichever is lower). Round up to the nearest 50 ms.
4. Update `infra/monitoring/alerts/latency-slo.yml` to match.
5. Commit with prefix `[re-sample YYYY-MM-DD]` and update **Last Sampled** below.

### Re-Sample Schedule

| Event | Action |
|-------|--------|
| First 24–72 h production traffic | Mandatory first re-sample — replace pre-production baselines |
| Monthly (first Monday) | Routine review — adjust if observed values drift > 20 % |
| After major release | Re-sample within 48 h of deploy |
| After incident | Re-sample to verify recovery to baseline |

---

## 8. Related Documents

| Document | Purpose |
|----------|---------|
| `docs/ops/ALERT-THRESHOLDS.md` | Business-logic thresholds (UNKNOWN, DLQ, invariants, outbox) |
| `docs/SCALE-ENVELOPE.md` | k6 multi-tenant scale test envelope and safe operating limits |
| `infra/monitoring/alerts/latency-slo.yml` | Prometheus alert rules (generated from this doc) |
| `docs/OPS-TRIAGE.md` | Operator triage runbook with Prometheus/Grafana access |

---

**Document Owner:** Platform Team
**Last Updated:** 2026-02-22 (P48-020, bd-1zkv)
**Last Sampled:** Pre-production (design-time baselines — no real traffic yet)
**Next Review:** After first 72 h of real production traffic
