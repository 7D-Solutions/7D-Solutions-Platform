# 7D Platform — k6 Performance Harness

Lightweight performance harness using [k6](https://k6.io). All tests hit real services — no mocks, no stubs.

## Directory layout

```
tools/perf/
├── config/
│   └── environments.js   # URL presets + credential env var loading
├── lib/
│   └── auth.js           # Token acquisition (login or pre-minted JWT)
├── smoke.js              # Smoke scenario — 5 critical endpoints, 1 VU × 1 iteration
└── README.md
```

## Prerequisites

Install k6 (one-time):

```bash
# macOS
brew install k6

# Linux / CI
curl -fsSL https://github.com/grafana/k6/releases/download/v0.55.0/k6-v0.55.0-linux-amd64.tar.gz \
  | tar -xzf - --strip-components=1 k6-v0.55.0-linux-amd64/k6
sudo mv k6 /usr/local/bin/k6
```

## Running locally (Docker Compose stack)

Start the full platform stack first, then seed a test user:

```bash
# Bring up the platform (auth, control-plane, AR, TTP)
docker compose \
  -f docker-compose.infrastructure.yml \
  -f docker-compose.platform.yml \
  -f docker-compose.modules.yml \
  up -d

# Create a platform admin (if not already done)
./scripts/seed-platform-admin.sh --email perf@test.7d.local --password 'PerfTest1!'

# Run the smoke scenario
PERF_AUTH_EMAIL=perf@test.7d.local \
PERF_AUTH_PASSWORD='PerfTest1!' \
k6 run tools/perf/smoke.js
```

Expected output on a healthy local stack: all checks green, 0 errors.

## Running against staging

```bash
PERF_ENV=staging \
STAGING_HOST=staging.7dsolutions.app \
PERF_AUTH_EMAIL=perf@staging.7d.internal \
PERF_AUTH_PASSWORD='StrongPass1!' \
k6 run tools/perf/smoke.js
```

If you already have a valid JWT, skip the login step:

```bash
PERF_ENV=staging \
STAGING_HOST=staging.7dsolutions.app \
PERF_AUTH_TOKEN='eyJ...' \
k6 run tools/perf/smoke.js
```

## Environment variables

| Variable               | Default                                   | Purpose                                  |
|------------------------|-------------------------------------------|------------------------------------------|
| `PERF_ENV`             | `local`                                   | Preset: `local` or `staging`             |
| `STAGING_HOST`         | —                                         | Staging VPS hostname/IP (required when `PERF_ENV=staging`) |
| `PERF_AUTH_EMAIL`      | —                                         | Login email                              |
| `PERF_AUTH_PASSWORD`   | —                                         | Login password                           |
| `PERF_AUTH_TOKEN`      | —                                         | Pre-minted JWT; skips login              |
| `PERF_TENANT_ID`       | `00000000-0000-0000-0000-000000000000`    | Tenant scope for auth                    |
| `PERF_AUTH_URL`        | from preset                               | Override auth-lb base URL                |
| `PERF_CONTROL_PLANE_URL` | from preset                             | Override control-plane base URL          |
| `PERF_AR_URL`          | from preset                               | Override AR module base URL              |
| `PERF_TTP_URL`         | from preset                               | Override TTP module base URL             |

## Running in CI (workflow_dispatch)

The workflow at `.github/workflows/perf.yml` exposes a manual trigger:

1. Go to **Actions → Performance — k6 → Run workflow**
2. Set **env** to `staging`, supply the **staging_host**, and optionally the tenant UUID
3. Add `PERF_AUTH_EMAIL` and `PERF_AUTH_PASSWORD` as repository secrets (Settings → Secrets → Actions)
4. Click **Run workflow**

The job installs k6, runs the smoke scenario followed by the baseline billing-spine
scenario, and fails the workflow if any threshold is breached in either run.
k6 summary JSON files are uploaded as workflow artifacts (retained 90 days) under
`perf-summaries-<git-sha>-<timestamp>`, even when a threshold fails, so engineers
can inspect the numbers that tripped the gate.

## Thresholds (smoke)

| Metric                       | Threshold        |
|------------------------------|-----------------|
| `http_req_failed`            | `rate < 1%`     |
| `http_req_duration` (p95)    | `< 2 000 ms`    |
| `smoke_control_plane_ms` (p95) | `< 1 000 ms`  |
| `smoke_ar_ms` (p95)          | `< 1 500 ms`    |
| `smoke_errors`               | `rate < 1%`     |

## Baseline — billing spine capacity

`tools/perf/baseline_billing_spine.js` establishes the operating envelope for
the billing spine (control-plane + tenant-registry + AR module).  Run it
against a live stack and export results as a CI artifact:

```bash
PERF_AUTH_EMAIL=perf@test.7d.local \
PERF_AUTH_PASSWORD='PerfTest1!' \
k6 run tools/perf/baseline_billing_spine.js \
     --summary-export=perf_summary.json
```

### Load shape

| Stage      | Duration | VUs |
|------------|----------|-----|
| Ramp-up    | 30 s     | 1 → 10 |
| Sustain    | 60 s     | 10 |
| Ramp-down  | 10 s     | 10 → 0 |

Total run time: ~100 s.  No real charges are created.  ~20% of iterations
issue a single customer-create (write-light); the remaining 80% are pure reads.

### Thresholds (baseline pass/fail gate)

| Metric                      | Threshold        | Tier               |
|-----------------------------|------------------|--------------------|
| `http_req_failed`           | `rate < 1%`      | All requests       |
| `http_req_duration` (p95)   | `< 1 000 ms`     | All requests       |
| `billing_cp_reads_ms` (p95) | `< 500 ms`       | Control-plane reads |
| `billing_ar_reads_ms` (p95) | `< 800 ms`       | AR module reads    |
| `billing_errors`            | `rate < 1%`      | check() failures   |

k6 exits non-zero if any threshold is breached, making it a hard CI gate.

### Endpoints exercised

| Group                         | Method | Path                          | Auth? |
|-------------------------------|--------|-------------------------------|-------|
| cp: readiness                 | GET    | /api/ready                    | No    |
| cp: tenant list               | GET    | /api/tenants                  | Yes   |
| cp: ttp plan catalog          | GET    | /api/ttp/plans                | Yes   |
| ar: customer list             | GET    | /api/ar/customers             | Yes   |
| ar: invoice list              | GET    | /api/ar/invoices              | Yes   |
| ar: subscription list         | GET    | /api/ar/subscriptions         | Yes   |
| ar: aging report              | GET    | /api/ar/aging                 | Yes   |
| ar: write-light customer create | POST | /api/ar/customers             | Yes   |

### CI artifact — perf_summary.json

The `--summary-export` flag writes a JSON file compatible with k6 Cloud and
standard CI artifact retention.  Key fields consumed downstream (bd-1obl):

```
metrics.billing_cp_reads_ms.values.p(95)
metrics.billing_ar_reads_ms.values.p(95)
metrics.http_req_failed.values.rate
metrics.billing_errors.values.rate
metrics.billing_write_ops.values.count
```

Compare successive runs to detect regressions.  A >20% increase in p95
latency vs the prior recorded baseline warrants investigation.

## Scale: multi-tenant (P50-010)

`tools/perf/scale_multitenant.js` models ≥5 tenants concurrently across three
sequential phases: read burst, billing runs, and webhook burst.

### Load shape

| Phase            | Start  | Duration    | VUs | Exec function   |
|------------------|--------|-------------|-----|-----------------|
| Read burst       | t=0s   | ramp 30s, sustain 60s, down 10s | 0→20→0 | `readPhase`   |
| Billing runs     | t=30s  | ramp 10s, sustain 60s, down 5s  | 0→5→0  | `billingPhase` |
| Webhook burst    | t=115s | ramp 10s, sustain 30s, down 5s  | 0→10→0 | `webhookPhase` |

Total wall time: ~165s.  Phases overlap (billing starts while reads are at
sustained load).

### Running locally

```bash
PERF_AUTH_EMAIL=perf@test.7d.local \
PERF_AUTH_PASSWORD='PerfTest1!' \
PERF_TILLED_WEBHOOK_SECRET='whsec_test' \
k6 run tools/perf/scale_multitenant.js \
     --summary-export=scale_multitenant_summary.json
```

### Running against staging

```bash
PERF_ENV=staging \
STAGING_HOST=staging.7dsolutions.app \
PERF_AUTH_EMAIL=perf@staging.7d.internal \
PERF_AUTH_PASSWORD='StrongPass1!' \
PERF_TILLED_WEBHOOK_SECRET='<tilled-webhook-secret-from-vault>' \
k6 run tools/perf/scale_multitenant.js \
     --summary-export=scale_multitenant_summary.json
```

### Environment variables (scale_multitenant-specific)

| Variable                   | Default                              | Purpose                                      |
|----------------------------|--------------------------------------|----------------------------------------------|
| `PERF_TILLED_WEBHOOK_SECRET` | —                                  | HMAC secret for webhook signatures (required for Phase 3) |
| `PERF_BILLING_PERIOD`      | `2099-01`                            | Billing period for safe/idempotent billing runs |
| `PERF_TENANT_1` … `PERF_TENANT_5` | `00000000-…-000000000001` … `5` | Override per-slot tenant UUIDs |
| `PERF_PROMETHEUS_URL`      | `http://localhost:9090`              | Prometheus base URL for teardown lag query   |
| `PERF_PAYMENTS_URL`        | from preset (port 8088)              | Override payments service base URL           |

All standard variables (`PERF_ENV`, `STAGING_HOST`, `PERF_AUTH_*`, etc.) still apply.

### Thresholds (scale pass/fail gate)

| Metric                        | p95 SLO      | p99 Ceiling  | Tier                        |
|-------------------------------|--------------|--------------|------------------------------|
| `http_req_failed`             | `rate < 1%`  | —            | All requests (HTTP errors)   |
| `http_req_duration`           | `< 2 000 ms` | `< 4 000 ms` | All requests, wall-clock     |
| `scale_cp_reads_ms`           | `< 500 ms`   | `< 1 000 ms` | Control-plane reads          |
| `scale_ar_reads_ms`           | `< 800 ms`   | `< 1 500 ms` | AR module reads              |
| `scale_billing_run_ms`        | `< 3 000 ms` | `< 5 000 ms` | Billing run (multi-service)  |
| `scale_webhook_ms`            | `< 500 ms`   | `< 1 000 ms` | Tilled webhook ingest        |
| `scale_errors`                | `rate < 1%`  | —            | check() failures (all phases)|
| `scale_webhook_errors`        | `rate < 1%`  | —            | Webhook-specific success gate|

**Projection lag** (Prometheus, checked in teardown — not a k6 gate):

| Level    | Threshold            |
|----------|----------------------|
| OK       | < 50 messages        |
| WARNING  | 50 – 200 messages    |
| CRITICAL | > 200 messages       |

See `docs/SCALE-ENVELOPE.md` for full bottleneck analysis and safe operating limits.

### Endpoints exercised

| Phase    | Method | Path                                       | Auth? |
|----------|--------|--------------------------------------------|-------|
| reads    | GET    | /api/ready                                 | No    |
| reads    | GET    | /api/tenants                               | Yes   |
| reads    | GET    | /api/ttp/plans                             | Yes   |
| reads    | GET    | /api/ar/customers                          | Yes   |
| reads    | GET    | /api/ar/invoices                           | Yes   |
| reads    | GET    | /api/ar/subscriptions                      | Yes   |
| reads    | GET    | /api/ar/aging                              | Yes   |
| billing  | POST   | /api/control/platform-billing-runs         | Yes   |
| webhooks | POST   | /api/payments/webhook/tilled               | No (HMAC) |

### Billing safe mode

`billingPhase` posts a fixed far-future period (`PERF_BILLING_PERIOD`, default
`2099-01`).  The first iteration creates invoices; all subsequent iterations
hit the idempotency guard and return `already_billed` — no duplicate charges.
This exercises the full control-plane → tenant-registry → AR write path under
concurrent load without polluting production or staging billing data.

### Webhook safe payloads

`webhookPhase` sends `payment_intent.succeeded` events with randomly-generated
`data.object.id` values (`pi_perf_<ts>_<vu>`).  These IDs do not exist in the
`checkout_sessions` table, so the `UPDATE … WHERE processor_payment_id = ?`
affects 0 rows — the service returns 200 OK, exercising the signature-validate
→ parse → DB-update path at load without mutating real payment records.

Each request carries a freshly-computed `tilled-signature: t=<ts>,v1=<hmac>`
header using the `PERF_TILLED_WEBHOOK_SECRET` value.  If the secret is not set,
Phase 3 VUs sleep and skip silently.

### Projection lag reporting

After all VUs finish, `teardown()` queries Prometheus for the
`payments_event_consumer_lag_messages` metric and logs the result.  If
Prometheus is unreachable, operators should check the Grafana
"Payments — Consumer Lag" panel manually.

### CI artifact — scale_multitenant_summary.json

Key fields consumed by bd-t47u (P50-030 CI workflow) and documented in `docs/SCALE-ENVELOPE.md`:

```
metrics.scale_cp_reads_ms.values.p(95)
metrics.scale_cp_reads_ms.values.p(99)
metrics.scale_ar_reads_ms.values.p(95)
metrics.scale_ar_reads_ms.values.p(99)
metrics.scale_billing_run_ms.values.p(95)
metrics.scale_billing_run_ms.values.p(99)
metrics.scale_webhook_ms.values.p(95)
metrics.scale_webhook_ms.values.p(99)
metrics.scale_errors.values.rate
metrics.scale_webhook_errors.values.rate
metrics.scale_billing_ops.values.count
metrics.scale_webhook_ops.values.count
metrics.http_req_failed.values.rate
```

## Scale test CI workflow (manual dispatch)

The workflow at `.github/workflows/scale.yml` runs the full multi-tenant scale
scenario against staging on demand.

### Triggering the workflow

1. Go to **Actions → Scale Test — k6 multi-tenant → Run workflow**
2. Fill in the inputs:

   | Input             | Required | Description                                        |
   |-------------------|----------|----------------------------------------------------|
   | `staging_host`    | Yes      | VPS hostname or IP (e.g. `staging.7dsolutions.app`) |
   | `tenant_id`       | No       | Tenant UUID for auth scope (default: platform tenant) |
   | `billing_period`  | No       | Safe billing period (default: `2099-01`)           |
   | `prometheus_url`  | No       | Prometheus base URL for lag dump (e.g. `http://staging.7dsolutions.app:9090`) |

3. Click **Run workflow**.

### Required secrets

Set these in **repo → Settings → Secrets → Actions**:

| Secret                       | Purpose                                       |
|------------------------------|-----------------------------------------------|
| `PERF_AUTH_EMAIL`            | Login email for the test account              |
| `PERF_AUTH_PASSWORD`         | Login password                                |
| `SCALE_TILLED_WEBHOOK_SECRET`| HMAC secret for Tilled webhook signatures (Phase 3) |
| `PERF_AUTH_TOKEN`            | (Optional) Pre-minted JWT; skips the login step |

If `SCALE_TILLED_WEBHOOK_SECRET` is not set, webhook Phase 3 VUs sleep and
skip silently — the run still validates Phases 1 and 2.

### Artifacts

Every run (including threshold failures) uploads an artifact bundle retained
for 90 days:

```
scale-test-<sha>-<timestamp>/
  scale-summary-<sha>-<timestamp>.json      # k6 metrics (timings, thresholds, counters)
  SCALE-ENVELOPE-snapshot-<timestamp>.md   # thresholds in effect at run time
  prometheus-lag-dump-<timestamp>.json     # consumer-lag point-in-time (only if prometheus_url set)
```

### Interpreting results

**Job status:**

- **Green** — all k6 thresholds passed. Check the Prometheus lag dump (if captured) to confirm
  the payments consumer drained before marking the run clean.
- **Red** — at least one threshold breached. Download `scale-summary-*.json` and look for
  `"thresholds"` entries with `"ok": false`. Compare against the values in
  `SCALE-ENVELOPE-snapshot-*.md` to identify which tier failed.

**Key fields in `scale-summary-*.json`:**

```
metrics.scale_cp_reads_ms.values.p(95)      # Control-plane p95 — gate: < 500 ms
metrics.scale_cp_reads_ms.values.p(99)      # Control-plane p99 — gate: < 1 000 ms
metrics.scale_ar_reads_ms.values.p(95)      # AR reads p95 — gate: < 800 ms
metrics.scale_ar_reads_ms.values.p(99)      # AR reads p99 — gate: < 1 500 ms
metrics.scale_billing_run_ms.values.p(95)   # Billing run p95 — gate: < 3 000 ms
metrics.scale_billing_run_ms.values.p(99)   # Billing run p99 — gate: < 5 000 ms
metrics.scale_webhook_ms.values.p(95)       # Webhook ingest p95 — gate: < 500 ms
metrics.scale_webhook_ms.values.p(99)       # Webhook ingest p99 — gate: < 1 000 ms
metrics.http_req_failed.values.rate         # HTTP error rate — gate: < 1%
metrics.scale_errors.values.rate            # check() failure rate — gate: < 1%
metrics.scale_webhook_errors.values.rate    # Webhook-specific error rate — gate: < 1%
```

**Consumer lag (`prometheus-lag-dump-*.json`):**

The `data.result[*].value[1]` field holds the current lag in messages.

| Lag              | Level    | Action                                                    |
|------------------|----------|-----------------------------------------------------------|
| < 50 messages    | OK       | Consumer draining normally; no action                     |
| 50 – 200         | WARNING  | Consumer falling behind; review NATS consumer config      |
| > 200 messages   | CRITICAL | Consumer overloaded; scale payments service or tune batch |

If Prometheus was unreachable, the dump contains `"status":"error"` — check the
Grafana **Payments — Consumer Lag** panel manually.

**After a breach:** Update `docs/SCALE-ENVELOPE.md` (Run History section) with the
actual p95/p99 values, classify the bottleneck against Section 3, and apply the
relevant mitigation from Section 5.

---

## Adding new scenarios

1. Create `tools/perf/<scenario>.js`
2. Import from `./config/environments.js` and `./lib/auth.js`
3. Add a new step to `.github/workflows/perf.yml` or create a separate workflow
