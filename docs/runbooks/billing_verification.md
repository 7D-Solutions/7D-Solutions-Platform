# Billing / Payment Verification Runbook

**Phase:** 46 — Production Cutover
**Bead:** bd-2ly4
**Status:** PROVEN — 2026-02-22

## Purpose

Prove the end-to-end billing/payment path is correct and idempotent under real production conditions. This runbook covers:

1. Tilled webhook signature verification (HMAC-SHA256)
2. Invoice lifecycle (draft → paid) via webhook event
3. Idempotency: duplicate webhook events produce exactly one DB record, no double-posting
4. Log capture for audit/debugging

## Prerequisites

| Requirement | Value |
|-------------|-------|
| Production VPS | SSH-accessible as `deploy` user |
| `PROD_HOST` | Set in `scripts/production/.env.production` |
| `TILLED_WEBHOOK_SECRET` | Set in `/etc/7d/production/secrets.env` (root-owned 0600) |
| AR service | Running and healthy on `localhost:8086` (VPS-local port) |
| `openssl` | Available on CI runner (for HMAC signature computation) |

## Invariants

- **No real money moves:** All verification uses `livemode=false` payloads. Tilled test mode does not charge.
- **Idempotency at DB level:** The AR `ar_webhooks` table enforces unique `(event_id, app_id)`. Duplicate delivery returns HTTP 200 (accepted) but creates no additional record.
- **Signature required, always:** The webhook endpoint never bypasses HMAC verification. A missing or invalid `tilled-signature` header returns 401.

## Step 1 — Dry-run validation

Before running against production, validate the script environment:

```bash
bash scripts/production/payment_verify.sh \
  --dry-run \
  --host prod.7dsolutions.example.com \
  --secret "$TILLED_WEBHOOK_SECRET"
```

Expected output:
```
=== Environment validation ===
  ✓  PROD_HOST       = prod.7dsolutions.example.com (SSH to localhost)
  ✓  Webhook secret  = (set, N chars)
  ✓  Timeout         = 20s

=== Dry-run mode — planned steps (no network calls) ===
  ▶  1. POST localhost:8086/api/ar/customers  — create test customer
  ▶  2. POST localhost:8086/api/ar/invoices   — create draft invoice
  ▶  3. Compute Tilled HMAC-SHA256 signature locally
  ▶  4. POST localhost:8086/api/ar/webhooks/tilled — deliver webhook (livemode=false)
  ▶  5. GET  localhost:8086/api/ar/invoices/<id>   — assert status = paid
  ▶  6. POST localhost:8086/api/ar/webhooks/tilled — REPLAY same event (idempotency)
  ▶  7. GET  localhost:8086/api/ar/webhooks?event_type=... — assert 1 record

Dry-run PASSED — environment valid.
```

## Step 2 — Full production verification

```bash
PROD_HOST=prod.7dsolutions.example.com \
TILLED_WEBHOOK_SECRET="$(sudo cat /etc/7d/production/secrets.env | grep TILLED_WEBHOOK_SECRET | cut -d= -f2)" \
  bash scripts/production/payment_verify.sh
```

The script executes 6 steps via SSH to the production VPS:

| Step | Action | Success signal |
|------|--------|---------------|
| 1 | Create AR customer (test fixture) | HTTP 201, `id` returned |
| 2 | Create AR invoice (open, $10.00) | HTTP 201, `id` + `tilled_invoice_id` returned |
| 3 | Compute HMAC-SHA256 signature locally | Signature string `t=…,v1=…` |
| 4 | POST webhook (livemode=false) | HTTP 200 |
| 5 | GET invoice, assert `status=paid` | HTTP 200, `"status":"paid"` |
| 6 | REPLAY same webhook event_id | HTTP 200 (idempotency — no duplicate record) |
| 7 | GET webhooks, assert count=1 for event_id | Exactly 1 `ar_webhooks` row |

Expected final output:
```
────────────────────────────────────────────────────────────
Production payment verification PROOF:
  Customer ID          : <id>
  Invoice ID           : <id>
  Tilled Invoice ID    : tilled_test_inv_<id>
  Event ID             : evt_prod_verify_<ts>_<pid>
  livemode             : false (Tilled test mode — no real money moved)
  Invoice final status : paid
  Webhook records      : 1 (expected 1 — idempotency PROVEN)

invoice → webhook → posting (test mode): PROVEN
Webhook replay idempotency:               PROVEN

Production payment verification PASSED.
```

## Step 3 — Capture log bundle

After the verification run, capture a log bundle covering the run window:

```bash
bash scripts/production/log_bundle.sh \
  --since 30m \
  --services "7d-ar,7d-payments,7d-subscriptions,7d-nats" \
  --out /tmp
```

Transfer to local machine for archival:

```bash
scp deploy@prod.7dsolutions.example.com:/tmp/7d-log-bundle-<YYYYMMDD-HHMMSS>.tar.gz ./logs/
```

Bundle contents:
- `manifest.txt` — capture metadata
- `container-list.txt` — `docker ps` at capture time
- `logs/7d-ar.log` — AR service log (webhook events, invoice state changes)
- `logs/7d-payments.log` — Payment module log
- `logs/7d-subscriptions.log` — Subscriptions log
- `logs/7d-nats.log` — NATS event bus log
- `nats-monitoring.json` — JetStream, connection count snapshot

## Idempotency Proof (code level)

Idempotency is enforced at two layers:

### Layer 1 — Database UNIQUE constraint

```sql
-- ar_webhooks table (modules/ar/migrations/)
UNIQUE (event_id, app_id)
```

On duplicate delivery, `INSERT` fails silently and returns HTTP 200 with a
"duplicate event — skipped" log line. No second posting is created.

### Layer 2 — Test coverage (real PostgreSQL, no mocks)

The AR webhook test suite (`modules/ar/tests/webhook_tests.rs`) proves
idempotency against real PostgreSQL:

```
test test_receive_webhook_duplicate_event_id ... ok  (count=1 enforced)
test test_receive_webhooks_out_of_order      ... ok
test test_replay_webhook_already_processed  ... ok
test test_replay_webhook_with_force          ... ok  (force=true re-delivers)
```

Full run result (2026-02-22):
```
running 12 tests
test test_receive_webhook_missing_signature_header   ... ok
test test_receive_webhook_stale_timestamp_rejected   ... ok
test test_list_webhooks_by_status                    ... ok
test test_replay_webhook_already_processed           ... ok
test test_replay_webhook_failed                      ... ok
test test_receive_webhook_invalid_signature          ... ok
test test_receive_webhook_valid_signature            ... ok
test test_get_webhook_success                        ... ok
test test_list_webhooks_by_event_type                ... ok
test test_receive_webhook_duplicate_event_id         ... ok
test test_replay_webhook_with_force                  ... ok
test test_receive_webhooks_out_of_order              ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; finished in 4.41s
```

## Payments Module Proof

The Payments module has its own Tilled signature verification and idempotency proof.
Run the full local proof at any time:

```bash
bash scripts/proof_payments.sh
```

Local proof result (2026-02-22):
```
  ✓ cargo build -p payments-rs
  ✓ cargo test -p payments-rs (all suites)
  ✓ Tilled signature: 11 vectors PROVEN (positive + negative + rotation)
  ✓ UNKNOWN protocol: 5 vectors PROVEN

payments proof: 4 pass / 0 fail
PROOF PASSED — safe to promote.
```

Individual test counts:
- Unit tests (lib): 58 pass
- Webhook signature vectors: 11 pass (HMAC-SHA256, replay protection, rotation overlap)
- Reconciliation tests: 9 pass (idempotency, concurrent safety, UNKNOWN protocol)
- Payment attempt tests: 5 pass
- Collection handler tests: 3 pass
- Contract tests: 5 pass

## Signature Verification Vectors (payments)

| Vector | Result |
|--------|--------|
| Valid fresh signature accepted | PASS |
| Tampered body rejected (HMAC mismatch) | PASS |
| Wrong secret rejected | PASS |
| Stale timestamp (>5 min past) rejected as replay | PASS |
| Future timestamp (>5 min ahead) rejected | PASS |
| Missing header returns MissingSignature | PASS |
| Malformed header (no t= or v1=) rejected | PASS |
| Empty secret slice returns not-configured error | PASS |
| Rotation overlap: old secret still accepted | PASS |
| Rotation overlap: unknown secret rejected | PASS |
| HMAC-SHA256 signature mismatch error message | PASS |

## Log Bundle Artifact (2026-02-22 verification)

Bundle captured: `/tmp/7d-log-bundle-20260222-142951.tar.gz`
Window: `since=1h`
Services: `7d-ar`, `7d-payments`, `7d-subscriptions`, `7d-nats`
Logs captured: 4 / Skipped: 0

## Rollback

If the verification fails:

1. Check AR service logs: `docker logs 7d-ar --since 30m`
2. Verify `TILLED_WEBHOOK_SECRET` is set in `/etc/7d/production/secrets.env`
3. Verify AR service health: `curl http://localhost:8086/api/health`
4. If webhook returns 401: re-check HMAC secret matches Tilled dashboard config
5. If idempotency fails (count > 1): check DB for `UNIQUE` constraint on `ar_webhooks(event_id, app_id)` — may need migration

## Repeat Schedule

Run this verification after:
- Any production deploy that touches AR or Payments
- Tilled webhook secret rotation (see `docs/runbooks/key_rotation.md`)
- Any billing incident or idempotency concern

See `scripts/production/payment_verify.sh` for the canonical live-run script.
