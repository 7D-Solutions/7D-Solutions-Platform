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

## Adding new scenarios

1. Create `tools/perf/<scenario>.js`
2. Import from `./config/environments.js` and `./lib/auth.js`
3. Add a new step to `.github/workflows/perf.yml` or create a separate workflow
