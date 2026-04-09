# Carrier Sandbox Credential Setup

How to provision sandbox API credentials for USPS, UPS, and FedEx so the carrier integration tests can run.

---

## Overview

Carrier integration tests live in `modules/shipping-receiving` and are gated with `#[ignore]`. They run in CI via `.github/workflows/carrier-integration.yml` and require real sandbox credentials stored as GitHub repository secrets.

Credentials are **never hardcoded**. They flow through the system as:

```
GitHub Secrets
  → CI env vars
    → psql SET app.* GUC settings
      → integrations_connector_configs table (test-carrier-sandbox tenant)
        → internal credentials endpoint (/api/integrations/internal/carrier-credentials/{type})
          → shipping-receiving credential facade
            → carrier adapter test
```

---

## Carrier Credential Requirements

### USPS Web Tools

| Secret Name  | Description                                      |
|--------------|--------------------------------------------------|
| `USPS_USER_ID` | Web Tools API username from the USPS developer portal |

**Where to get it:** Register at https://registration.shippingapis.com — approval is automatic for sandbox access.

### UPS OAuth2

| Secret Name          | Description                            |
|----------------------|----------------------------------------|
| `UPS_CLIENT_ID`      | OAuth2 client ID from the UPS Developer Kit |
| `UPS_CLIENT_SECRET`  | OAuth2 client secret                   |
| `UPS_ACCOUNT_NUMBER` | UPS account number (6-character)       |

**Where to get it:** Sign in to https://developer.ups.com — create an app under the sandbox environment to receive `client_id` and `client_secret`. The account number is on your UPS profile.

### FedEx REST API

| Secret Name            | Description                                          |
|------------------------|------------------------------------------------------|
| `FEDEX_CLIENT_ID`      | API key from the FedEx Developer Portal (test project) |
| `FEDEX_CLIENT_SECRET`  | Secret key for the test project                      |
| `FEDEX_ACCOUNT_NUMBER` | FedEx account number                                 |

**Where to get it:** Register at https://developer.fedex.com — create a project under the Test environment to receive `client_id` and `client_secret`. The account number is on your FedEx profile.

---

## Adding Secrets to GitHub

In the repository, go to **Settings → Secrets and variables → Actions → New repository secret** and add each of the seven secrets listed above.

Names must match exactly (case-sensitive):
- `USPS_USER_ID`
- `UPS_CLIENT_ID`
- `UPS_CLIENT_SECRET`
- `UPS_ACCOUNT_NUMBER`
- `FEDEX_CLIENT_ID`
- `FEDEX_CLIENT_SECRET`
- `FEDEX_ACCOUNT_NUMBER`

---

## Running Locally

To seed credentials and run the integration tests on a local dev database:

```bash
# 1. Start the integrations DB
docker compose -f docker-compose.infrastructure.yml up -d integrations-postgres

# 2. Apply schema migrations
for f in modules/integrations/db/migrations/[0-9]*.sql; do
  PGPASSWORD=integrations_pass psql \
    -h localhost -p 5449 \
    -U integrations_user -d integrations_db \
    -f "$f"
done

# 3. Seed carrier credentials (set your env vars first)
export USPS_USER_ID="your-usps-user-id"
export UPS_CLIENT_ID="your-ups-client-id"
export UPS_CLIENT_SECRET="your-ups-client-secret"
export UPS_ACCOUNT_NUMBER="your-ups-account-number"
export FEDEX_CLIENT_ID="your-fedex-client-id"
export FEDEX_CLIENT_SECRET="your-fedex-client-secret"
export FEDEX_ACCOUNT_NUMBER="your-fedex-account-number"

PGPASSWORD=integrations_pass psql \
  -h localhost -p 5449 \
  -U integrations_user -d integrations_db \
  -c "SET app.usps_user_id = '${USPS_USER_ID}';" \
  -c "SET app.ups_client_id = '${UPS_CLIENT_ID}';" \
  -c "SET app.ups_client_secret = '${UPS_CLIENT_SECRET}';" \
  -c "SET app.ups_account_number = '${UPS_ACCOUNT_NUMBER}';" \
  -c "SET app.fedex_client_id = '${FEDEX_CLIENT_ID}';" \
  -c "SET app.fedex_client_secret = '${FEDEX_CLIENT_SECRET}';" \
  -c "SET app.fedex_account_number = '${FEDEX_ACCOUNT_NUMBER}';" \
  -f modules/integrations/db/migrations/20260409000013_seed_carrier_sandbox_credentials.sql

# 4. Run the carrier integration tests
./scripts/cargo-slot.sh test -p shipping-receiving-rs -- carrier_provider --include-ignored --test-threads=1 --nocapture
```

---

## How the Seed Migration Works

`20260409000013_seed_carrier_sandbox_credentials.sql` uses a PL/pgSQL `DO` block. It reads credentials from PostgreSQL session-level GUC settings (`app.*` namespace) set by the caller. If any credential is absent or empty, that carrier's INSERT is skipped with a `NOTICE` message — the migration never fails due to missing credentials.

Records are inserted into `integrations_connector_configs` with:
- `app_id = 'test-carrier-sandbox'`
- `connector_type` matching the carrier code used by the dispatch consumer (`usps`, `ups`, `fedex`)
- `ON CONFLICT DO UPDATE` so re-seeding is idempotent

---

## Verifying the Setup

After seeding, confirm each carrier's record is retrievable via the internal endpoint:

```bash
# Start the integrations service
docker compose -f docker-compose.services.yml up -d integrations

# Query each carrier
curl -s http://localhost:8099/api/integrations/internal/carrier-credentials/usps \
  -H "X-App-Id: test-carrier-sandbox" | jq .

curl -s http://localhost:8099/api/integrations/internal/carrier-credentials/ups \
  -H "X-App-Id: test-carrier-sandbox" | jq .

curl -s http://localhost:8099/api/integrations/internal/carrier-credentials/fedex \
  -H "X-App-Id: test-carrier-sandbox" | jq .
```

Each should return a `200` with the credential JSON (no credential values are logged or displayed in test output).
