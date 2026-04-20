# QBO Production Cutover Checklist

**Scope:** Intuit QuickBooks Online integration, integrations-rs module.
**Owner:** Platform Orchestrator.
**Dependent bead:** bd-r2l8z (production smoke runbook).

This checklist defines every preflight check, cutover step, validation gate, and rollback criterion for switching the QBO integration from sandbox credentials to production. All verification steps run against real Postgres, real NATS, and live Intuit endpoints. No mocks.

---

## Credential Inventory

| Secret file | Value in production | Wrong value (sandbox) |
|---|---|---|
| `qbo_client_id` | Production Intuit app client ID | Sandbox app client ID |
| `qbo_client_secret` | Production Intuit app client secret | Sandbox app secret |
| `qbo_redirect_uri` | `https://app.7dsolutions.com/api/integrations/oauth/callback/quickbooks` | Any localhost or sandbox URL |
| `oauth_encryption_key` | Strong random 32-byte hex | Carry over from sandbox |
| `QBO_SANDBOX` env var | Must be absent or `0` | `1` |
| `QBO_BASE_URL` env var | Absent (service defaults to production) or `https://quickbooks.api.intuit.com/v3` | Any `sandbox-quickbooks` URL |

---

## Phase 1: Intuit Developer Portal (days before cutover)

These steps require a human with access to the Intuit developer portal at https://developer.intuit.com.

- [ ] **App is in Production status.** The Intuit app must have completed Intuit's review process and show status "Production" (not "Development"). Development apps cannot issue production tokens.
- [ ] **Production app created under correct company account.** Confirm the app belongs to the 7D Solutions organizational account, not a personal developer account.
- [ ] **Redirect URI registered.** In the production app settings under "Keys & credentials", the redirect URI `https://app.7dsolutions.com/api/integrations/oauth/callback/quickbooks` is listed exactly. Any trailing slash mismatch causes a redirect_uri_mismatch error at token exchange.
- [ ] **Scope set to accounting.** The app has `com.intuit.quickbooks.accounting` as the only requested scope. No extra scopes added (additional scopes require re-authorization from tenants).
- [ ] **Production client ID and secret recorded.** Copy the production Client ID and Client Secret from the app dashboard. These are different from the sandbox credentials.
- [ ] **No active sandbox connections remain for affected tenants.** Query the integrations database: `SELECT app_id, realm_id, connection_status FROM integrations_oauth_connections WHERE provider = 'quickbooks' AND connection_status = 'connected';`. Disconnect any sandbox connections before cutover; do not carry sandbox tokens into production.

---

## Phase 2: Secrets Store Preparation (hours before cutover)

All secrets live under `/etc/7d/production/secrets/`. See `docs/SECRETS.md` for the full secrets protocol.

- [ ] **Write production client ID.** `echo -n "<prod_client_id>" | sudo tee /etc/7d/production/secrets/qbo_client_id`
- [ ] **Write production client secret.** `echo -n "<prod_client_secret>" | sudo tee /etc/7d/production/secrets/qbo_client_secret`
- [ ] **Write redirect URI.** `echo -n "https://app.7dsolutions.com/api/integrations/oauth/callback/quickbooks" | sudo tee /etc/7d/production/secrets/qbo_redirect_uri`
- [ ] **Confirm encryption key exists.** `sudo test -s /etc/7d/production/secrets/oauth_encryption_key && echo OK`. If missing, generate with `openssl rand -hex 32`. Never reuse a key that protected sandbox tokens.
- [ ] **QBO_SANDBOX is absent.** `sudo grep -r QBO_SANDBOX /etc/7d/production/secrets/ /etc/7d/production/*.env 2>/dev/null && echo "FOUND - remove it" || echo "OK - absent"`. This env var must not be present; its presence forces the sandbox base URL in the CDC poller.
- [ ] **QBO_BASE_URL is absent or production.** `sudo grep -r QBO_BASE_URL /etc/7d/production/ 2>/dev/null`. If present, value must be `https://quickbooks.api.intuit.com/v3`. Any `sandbox-quickbooks` string here means CDC polls sandbox data in production.
- [ ] **Run the automated preflight script.** See the "Automated Preflight" section below.

---

## Phase 3: Service State Checks (immediately before cutover)

- [ ] **Migrations are current.** The integrations service logs "All migrations applied" at startup. Cross-check by running migrations manually: `./scripts/cargo-slot.sh test -p integrations-rs -- migration_safety_test`.
- [ ] **Outbox queue is drained.** No pending outbox rows should exist for QBO before credentials swap: `SELECT COUNT(*) FROM integrations_outbox WHERE app_id IN (SELECT app_id FROM integrations_oauth_connections WHERE provider = 'quickbooks') AND status = 'pending';`. Zero expected; if non-zero, wait for the outbox relay to drain or manually resolve.
- [ ] **NATS is reachable.** `docker exec 7d-nats nats-server --help` exits 0. The integrations service logs confirm NATS subscription on startup.
- [ ] **No existing push attempts in 'inflight' status.** `SELECT COUNT(*) FROM integrations_sync_push_attempts WHERE status = 'inflight';`. Inflight rows from sandbox will become stale after credential rotation; resolve them before cutover.

---

## Phase 4: Cutover Execution

Execute these steps in order. Do not skip ahead.

1. **Deploy updated secrets.** Restart the integrations service to pick up the new credential files. The service uses the secrets entrypoint wrapper (`scripts/docker-secrets-entrypoint.sh`) which re-reads files on start.

2. **Verify startup logs show no panic.** The startup function `validate_qbo_env` panics if any required env var is missing or if `QBO_REDIRECT_URI` does not start with `https://`. Check container logs within 30 seconds of restart.
   ```bash
   docker compose logs integrations --since 60s | grep -E "QBO|panic|FATAL|ERROR"
   ```

3. **Verify QBO integration is active.** The service logs a startup message when QBO client ID is present and the token refresh worker spawns. Look for: `"token refresh worker started"` or similar.

4. **Confirm no sandbox URL in CDC poller.** The CDC poller logs its base URL on the first poll tick. Confirm it says `https://quickbooks.api.intuit.com` not `sandbox-quickbooks.api.intuit.com`.

5. **Trigger OAuth connect for each tenant.** A user with `integrations.oauth.connect` scope visits `GET /api/integrations/oauth/connect/quickbooks`. They are redirected to `https://appcenter.intuit.com/connect/oauth2` (not the sandbox variant). They complete authorization in their production QBO account.

6. **Verify token stored.** After callback: `GET /api/integrations/oauth/status/quickbooks` returns `{ "connection_status": "connected", "scopes": "com.intuit.quickbooks.accounting" }`. The `realm_id` must match the tenant's production QBO company ID.

7. **Verify scope is accounting only.** Check the stored scopes field. Any scope other than `com.intuit.quickbooks.accounting` indicates the Intuit app was misconfigured. Disconnect and fix the app before proceeding.

---

## Validation Gates

Each gate is a pass/fail checkpoint. Any fail triggers the rollback procedure.

| Gate | Command | Pass criterion | Fail action |
|---|---|---|---|
| G1: Service starts | `docker compose logs integrations --since 60s` | No `panic`, no `FATAL`, "token refresh worker started" present | Check env vars, see rollback |
| G2: OAuth redirects to production | Click connect link, inspect browser redirect URL | URL contains `appcenter.intuit.com` (not sandbox) | Check `QBO_AUTH_URL` env, default should be production |
| G3: Token exchange succeeds | Callback returns 201 | HTTP 201, `connection_status = connected` | Check client ID/secret match production app; see rollback |
| G4: Scope matches | `GET /oauth/status/quickbooks` | `scopes = com.intuit.quickbooks.accounting` | Disconnect, fix Intuit app scope, reconnect |
| G5: CDC base URL is production | Container logs at first CDC tick | `quickbooks.api.intuit.com` (no `sandbox-`) | Unset `QBO_SANDBOX`, check `QBO_BASE_URL` |
| G6: Authority endpoint responds | `GET /sync/authority` | HTTP 200 | Check service health, inspect logs |
| G7: Push attempt accepted | `POST /sync/push/customer` | Returns `Accepted` or `Succeeded` (not `needs_reauth`) | See rollback |
| G8: DLQ clean after 10 min | `GET /sync/dlq?failure_reason=needs_reauth` | Zero rows | Tokens invalid; disconnect + reconnect or rollback |
| G9: Token refresh cycle | Wait 60 min after connect | No `needs_reauth` rows appear; token expiry resets | Refresh worker misconfigured; check `QBO_CLIENT_ID/SECRET` |

---

## Rollback Criteria

Initiate rollback immediately if any of the following occur:

- Service panics at startup with missing env vars.
- OAuth callback returns `502 token_exchange_failed` (client ID/secret mismatch).
- Stored `scopes` field does not match `com.intuit.quickbooks.accounting`.
- Any push attempt returns `needs_reauth` within 30 minutes of connect.
- DLQ accumulates more than 5 `needs_reauth` rows within 10 minutes.
- CDC poller logs show 401 or 403 responses from Intuit API.
- `realm_id` in the stored connection does not match the tenant's production QBO company.

---

## Rollback Procedure

1. **Restore sandbox credentials in secrets store:**
   ```bash
   echo -n "<sandbox_client_id>"     | sudo tee /etc/7d/production/secrets/qbo_client_id
   echo -n "<sandbox_client_secret>" | sudo tee /etc/7d/production/secrets/qbo_client_secret
   echo -n "http://localhost:8099/api/integrations/oauth/callback/quickbooks" \
     | sudo tee /etc/7d/production/secrets/qbo_redirect_uri
   ```

2. **Restart the integrations service** to pick up sandbox credentials.

3. **Disconnect production OAuth connections** for all affected tenants:
   ```bash
   curl -X POST https://app.7dsolutions.com/api/integrations/oauth/disconnect/quickbooks \
     -H "Authorization: Bearer <service_token>"
   ```

4. **Reconnect with sandbox OAuth flow.** Navigate to `/api/integrations/oauth/connect/quickbooks` per tenant and authorize against the sandbox QBO company.

5. **Verify `GET /oauth/status/quickbooks`** shows `connected` and `realm_id` matches the sandbox company.

6. **Drain any `needs_reauth` DLQ rows** that accumulated during the failed cutover:
   ```bash
   # Inspect rows
   curl https://app.7dsolutions.com/api/integrations/sync/dlq?failure_reason=needs_reauth \
     -H "Authorization: Bearer <service_token>"
   ```
   Retry or discard these rows manually once sandbox tokens are valid.

7. **File an incident report.** Record which gate failed, what was observed, and what was corrected in `.flywheel/incidents/` with a timestamp.

---

## Automated Preflight

Run the preflight script before Phase 4 to catch misconfiguration before any service restarts:

```bash
./scripts/qbo-cutover-preflight.sh
```

The script checks all required secrets exist, validates values, and confirms no sandbox URLs are present. It exits non-zero on any failure and prints the exact misconfiguration. See `scripts/qbo-cutover-preflight.sh` for the full check list.

To run the full dry-run validation against real Postgres, real NATS, and live Intuit sandbox (the verification gate for this bead):

```bash
QBO_SANDBOX=1 ./scripts/cargo-slot.sh test -p integrations-rs \
  -- qbo_smoke_test oauth_integration sync_push_endpoint_test \
  --nocapture
```

All three test suites must pass with zero failures. The `oauth_integration` suite requires `.env.qbo-sandbox` and `.qbo-tokens.json` at the repo root; see `docs/carrier-sandbox-setup.md` for token setup.

---

## Post-Cutover Monitoring (first 24 hours)

- [ ] DLQ at `GET /sync/dlq` remains at zero `needs_reauth` rows.
- [ ] Sync job health at `GET /sync/jobs` shows `consecutive_failures = 0` for all job types.
- [ ] CDC poll job shows a `last_successful_at` timestamp updated within the last poll interval.
- [ ] Token refresh worker logs show successful refresh before each token's 60-minute expiry.
- [ ] No `sync_push_failed` NATS events with `error_code = needs_reauth` in the event stream.

---

## Scope Reference

The QBO integration requests exactly one scope:

| Scope | Purpose |
|---|---|
| `com.intuit.quickbooks.accounting` | Read/write access to QBO accounting data (customers, invoices, payments, accounts, items, vendors) |

The service does **not** request `com.intuit.quickbooks.payroll` or any other scope. Requesting additional scopes requires re-authorization from all connected tenants.

---

## Environment Variable Reference

| Variable | Required | Production value | Notes |
|---|---|---|---|
| `QBO_CLIENT_ID` | Yes (enables QBO) | Production Intuit app client ID | Service skips QBO entirely if absent |
| `QBO_CLIENT_SECRET` | Yes | Production Intuit app client secret | Validated at startup |
| `QBO_REDIRECT_URI` | Yes | `https://app.7dsolutions.com/api/integrations/oauth/callback/quickbooks` | Must start with `https://` in production |
| `OAUTH_ENCRYPTION_KEY` | Yes | 32-byte random hex | Tokens encrypted at rest via pgcrypto |
| `QBO_AUTH_URL` | No | Default: `https://appcenter.intuit.com/connect/oauth2` | Override only for testing |
| `QBO_TOKEN_URL` | No | Default: `https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer` | Override only for testing |
| `QBO_BASE_URL` | No | Absent (defaults to production) | Setting to a sandbox URL forces CDC to poll sandbox data |
| `QBO_SANDBOX` | No | Must be absent or `0` | Setting to `1` forces CDC to use `sandbox-quickbooks.api.intuit.com` |
