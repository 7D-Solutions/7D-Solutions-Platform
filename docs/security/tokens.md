---
owner: james@7dmanufacturing.com
last_reviewed: 2026-04-15
---

# CI/CD Secret Token Registry

**Owner:** james@7dmanufacturing.com
**Last reviewed:** 2026-04-15

Every secret referenced in `.github/workflows/` is documented here: purpose, minimum required scope, rotation cadence, and environment boundary. The goal is that anyone asking "what can leak if X is compromised?" gets the answer from this file, not from a human.

**Scope policy:** Where the actual scope is not verifiable from public documentation or current configuration, the field reads `UNKNOWN — needs operator confirmation`. A documented unknown is safer than an authoritative-looking guess.

---

## CRATE_REGISTRY_TOKEN

**Purpose:** Authenticates `cargo publish` to the GitHub Packages registry (`7d-platform` scope) when the Gate 2 crate-publish CI job detects a version-intent commit.

**Minimum scope:** GitHub PAT with `write:packages` on the `7d-solutions-platform` repository only. No `repo` scope required.

**Rotation cadence:** 90 days, or immediately on any CI runner compromise.

**Environment reuse rule:** CI-only. Not used in staging or production runtime.

---

## DOCKER_PASSWORD

**Purpose:** Authenticates `docker push` to the container registry (`7dsolutions` Docker Hub namespace) in the Gate 2 image-build and release jobs.

**Minimum scope:** Docker Hub access token (not account password) scoped to `Read & Write` on repositories under the `7dsolutions` namespace only. UNKNOWN — confirm whether the stored value is an account password or a scoped access token; it must be the latter.

**Rotation cadence:** 90 days, or immediately on any registry compromise.

**Environment reuse rule:** CI-only. Production and staging pull images by digest; push access is not needed at runtime.

---

## DOCKER_USERNAME

**Purpose:** Docker Hub username passed alongside `DOCKER_PASSWORD` to authenticate registry push in CI.

**Minimum scope:** N/A — identifier, not a credential.

**Rotation cadence:** N/A (changes only on account rename).

**Environment reuse rule:** CI-only.

---

## FEDEX_ACCOUNT_NUMBER

**Purpose:** FedEx account number scoping rate and shipment API calls to the 7D account in carrier integration tests.

**Minimum scope:** N/A — account identifier, not a bearer credential. Access is gated by `FEDEX_CLIENT_ID` / `FEDEX_CLIENT_SECRET`.

**Rotation cadence:** N/A (changes only on contract change).

**Environment reuse rule:** CI-only (sandbox tests). UNKNOWN — confirm whether the sandbox and production account numbers are distinct values; they must be.

---

## FEDEX_CLIENT_ID

**Purpose:** OAuth2 client ID for the FedEx Developer Portal application; used with `FEDEX_CLIENT_SECRET` to obtain access tokens in carrier integration tests.

**Minimum scope:** FedEx Developer Portal: Rate API + Ship API, sandbox environment only. No Tracking, Ground, or Freight scopes required.

**Rotation cadence:** 90 days via FedEx Developer Portal, or immediately on exposure.

**Environment reuse rule:** CI-only (sandbox). Production FedEx credentials must be stored under separate secret names.

---

## FEDEX_CLIENT_SECRET

**Purpose:** OAuth2 client secret paired with `FEDEX_CLIENT_ID`; authenticates the carrier integration test application to FedEx.

**Minimum scope:** Same application as `FEDEX_CLIENT_ID`. Sandbox Rate + Ship only.

**Rotation cadence:** 90 days, or immediately on exposure.

**Environment reuse rule:** CI-only (sandbox). See `FEDEX_CLIENT_ID`.

---

## GITHUB_TOKEN

**Purpose:** Ephemeral GitHub Actions token used by the gitleaks secret-scanning action to access repository metadata and report scan findings on pull requests and pushes.

**Minimum scope:** `contents: read` and `pull-requests: read`. UNKNOWN — `security.yml` does not declare an explicit `permissions` block, so it inherits the repository default. Add `permissions: contents: read` and `pull-requests: read` to `security.yml` to enforce minimum scope.

**Rotation cadence:** N/A — GitHub auto-generates and revokes this token per workflow run (1-hour TTL).

**Environment reuse rule:** CI-only. Automatically scoped to the current repository.

---

## PERF_AUTH_EMAIL

**Purpose:** Email address for the dedicated k6 performance test account; used in the smoke and baseline perf workflows to obtain a session token when `PERF_AUTH_TOKEN` is absent.

**Minimum scope:** The test account must hold the minimum role required by the endpoints under test — staff reader at most. Not an admin or billing-admin account.

**Rotation cadence:** 90 days.

**Environment reuse rule:** CI-only (staging target). This account must not exist in production. UNKNOWN — confirm test account is staging-only.

---

## PERF_AUTH_PASSWORD

**Purpose:** Password for the k6 performance test account; paired with `PERF_AUTH_EMAIL` to obtain a session token.

**Minimum scope:** N/A — credential for the test account described under `PERF_AUTH_EMAIL`.

**Rotation cadence:** 90 days.

**Environment reuse rule:** CI-only (staging). See `PERF_AUTH_EMAIL`.

---

## PERF_AUTH_TOKEN

**Purpose:** Optional pre-minted JWT for the k6 performance test account; bypasses the login step when present, reducing test setup latency.

**Minimum scope:** JWT issued for the test account (see `PERF_AUTH_EMAIL`). Staff reader at most.

**Rotation cadence:** Per platform JWT TTL, or 30 days if the token is long-lived.

**Environment reuse rule:** CI-only (staging target). Must not be a production-issued JWT.

---

## PROD_HOST

**Purpose:** Hostname or IP address of the production VPS; used by SSH deploy and proof-gate steps to connect to the server.

**Minimum scope:** N/A — network address, not a credential.

**Rotation cadence:** N/A (changes only on server migration).

**Environment reuse rule:** Production-only.

---

## PROD_REPO_PATH

**Purpose:** Filesystem path to the repository checkout on the production VPS; used by deploy and proof-gate scripts to locate Docker Compose files and manifests.

**Minimum scope:** N/A — path string, not a credential.

**Rotation cadence:** N/A (changes only on server reconfiguration).

**Environment reuse rule:** Production-only.

---

## PROD_SSH_PORT

**Purpose:** SSH port for the production VPS; defaults to 22 if unset.

**Minimum scope:** N/A — port number, not a credential.

**Rotation cadence:** N/A.

**Environment reuse rule:** Production-only.

---

## PROD_SSH_PRIVATE_KEY

**Purpose:** ED25519 or RSA private key for the production deploy user; written to `~/.ssh/prod_deploy` during the deploy workflow and used for all SSH operations against the production VPS.

**Minimum scope:** Authorizes `$PROD_USER` login on `$PROD_HOST`. The corresponding `authorized_keys` entry on the server must include a `command=` restriction limiting this key to deploy scripts only. UNKNOWN — confirm server-side `authorized_keys` `command=` restriction is in place.

**Rotation cadence:** 90 days. Generate a new key pair, install the public key on the server, update this secret, then remove the old `authorized_keys` entry.

**Environment reuse rule:** Production-only. Must never share a value with `STAGING_SSH_PRIVATE_KEY`.

---

## PROD_USER

**Purpose:** SSH username for the production VPS deploy user.

**Minimum scope:** N/A — username, not a credential.

**Rotation cadence:** N/A.

**Environment reuse rule:** Production-only.

---

## SCALE_TILLED_WEBHOOK_SECRET

**Purpose:** HMAC-SHA256 shared secret for signing and verifying Tilled webhook payloads during the k6 multi-tenant scale test (Phase 3 webhook burst). Intentionally separate from `TILLED_WEBHOOK_SECRET` to avoid using the real webhook secret in load tests.

**Minimum scope:** Must be the signing secret for a sandbox or test-only Tilled webhook endpoint, not the production endpoint.

**Rotation cadence:** 90 days. Regenerate in the Tilled sandbox dashboard.

**Environment reuse rule:** CI-only (staging target). Must be a different value than `TILLED_WEBHOOK_SECRET`. If they are currently the same value, that is a defect.

---

## SMOKE_STAFF_JWT

**Purpose:** Pre-minted JWT for a staff user account; used in post-deploy proof-gate smoke tests for both staging and production to authenticate API calls without a login round-trip.

**Minimum scope:** Staff reader role — the minimum role required by the smoke test endpoint set. Must not be an admin or billing-admin JWT.

**Rotation cadence:** Per platform JWT TTL, or 30 days if long-lived. Issue with the shortest TTL the proof gate can tolerate.

**Environment reuse rule:** CAUTION — used in both the `staging` and `production` proof gates under the same secret name. If GitHub environment-level secrets are not configured, the same JWT is presented to both environments. Action required: confirm environment-scoped versions exist, or replace with `SMOKE_STAFF_JWT_STAGING` and `SMOKE_STAFF_JWT_PROD` backed by separate staff accounts.

---

## STAGING_HOST

**Purpose:** Hostname or IP address of the staging VPS; used by SSH deploy, manifest diff, and proof-gate steps.

**Minimum scope:** N/A — network address, not a credential.

**Rotation cadence:** N/A (changes only on server migration).

**Environment reuse rule:** CI+staging only. Never production.

---

## STAGING_REPO_PATH

**Purpose:** Filesystem path to the repository checkout on the staging VPS.

**Minimum scope:** N/A — path string, not a credential.

**Rotation cadence:** N/A.

**Environment reuse rule:** CI+staging only. Never production.

---

## STAGING_SSH_PORT

**Purpose:** SSH port for the staging VPS; defaults to 22 if unset.

**Minimum scope:** N/A — port number, not a credential.

**Rotation cadence:** N/A.

**Environment reuse rule:** CI+staging only. Never production.

---

## STAGING_SSH_PRIVATE_KEY

**Purpose:** ED25519 or RSA private key for the staging deploy user; written to `~/.ssh/staging_deploy` during the deploy workflow and used for SSH operations against the staging VPS.

**Minimum scope:** Authorizes `$STAGING_USER` login on `$STAGING_HOST`. Server-side `authorized_keys` should include a `command=` restriction. UNKNOWN — confirm restriction is in place.

**Rotation cadence:** 90 days.

**Environment reuse rule:** CI+staging only. Must never share a value with `PROD_SSH_PRIVATE_KEY`.

---

## STAGING_USER

**Purpose:** SSH username for the staging VPS deploy user.

**Minimum scope:** N/A — username, not a credential.

**Rotation cadence:** N/A.

**Environment reuse rule:** CI+staging only. Never production.

---

## TILLED_ACCOUNT_ID

**Purpose:** Tilled merchant account ID (the `tilled-account` HTTP header value); identifies which account's data to read/write during AR sandbox tests and post-deploy proof gates.

**Minimum scope:** N/A — account identifier, not a bearer credential. Access is gated by `TILLED_SECRET_KEY` or `TILLED_WEBHOOK_SECRET`.

**Rotation cadence:** N/A (account ID does not rotate; rotate the API key instead).

**Environment reuse rule:** UNKNOWN — used in PR sandbox tests (`ar-tilled-sandbox.yml`, no GitHub environment block) and in staging/production proof gates. Confirm this is the sandbox account ID only and that production uses an environment-scoped secret with the production account ID.

---

## TILLED_SECRET_KEY

**Purpose:** Tilled sandbox API secret key (Bearer token for Tilled REST API); used in the AR Tilled sandbox test suite to create customers, payment methods, charges, refunds, disputes, and webhooks against the Tilled sandbox environment.

**Minimum scope:** Tilled sandbox API: read + write on `customers`, `payment_methods`, `charges`, `refunds`, `disputes`, `subscriptions`, `webhooks`. Sandbox environment only; no production access.

**Rotation cadence:** 90 days via Tilled dashboard, or immediately on exposure.

**Environment reuse rule:** CI-only (Tilled sandbox). Sandbox and production API keys must be different values. Any production Tilled API key must be stored under a distinct secret name.

---

## TILLED_WEBHOOK_SECRET

**Purpose:** HMAC-SHA256 shared secret for verifying incoming Tilled webhook signatures; used in post-deploy proof-gate scripts for both staging and production deploys.

**Minimum scope:** N/A — this is a shared HMAC key set in the Tilled dashboard per webhook endpoint, not a scoped API credential. Each Tilled webhook endpoint has its own secret; staging and production endpoints must have different secrets.

**Rotation cadence:** 90 days. Regenerate in the Tilled dashboard for the relevant endpoint, update the secret, redeploy.

**Environment reuse rule:** CAUTION — appears in both `staging` and `production` proof gates under the same secret name. If GitHub environment-level secrets are configured (different values per environment), this is acceptable. If not, staging and production share the same webhook secret, which conflates environments. Action required: confirm environment-scoped values are in place.

---

## UPS_ACCOUNT_NUMBER

**Purpose:** UPS account number for carrier integration tests; passed to UPS APIs to scope shipment and rate requests to the 7D account.

**Minimum scope:** N/A — account identifier, not a bearer credential. Access is gated by `UPS_CLIENT_ID` / `UPS_CLIENT_SECRET`.

**Rotation cadence:** N/A (changes only on contract change).

**Environment reuse rule:** CI-only (sandbox tests). UNKNOWN — confirm whether this is a sandbox or production UPS account number; they must be distinct.

---

## UPS_CLIENT_ID

**Purpose:** OAuth2 client ID for the UPS Developer Portal application; used with `UPS_CLIENT_SECRET` to obtain access tokens in carrier integration tests.

**Minimum scope:** UPS Developer Portal: Rate API + Shipment API, sandbox environment only.

**Rotation cadence:** 90 days via UPS Developer Portal.

**Environment reuse rule:** CI-only (sandbox). Production credentials must be stored under separate secret names.

---

## UPS_CLIENT_SECRET

**Purpose:** OAuth2 client secret paired with `UPS_CLIENT_ID`; authenticates the carrier test application to UPS.

**Minimum scope:** Same application as `UPS_CLIENT_ID`. Sandbox only.

**Rotation cadence:** 90 days, or immediately on exposure.

**Environment reuse rule:** CI-only (sandbox). See `UPS_CLIENT_ID`.

---

## USPS_USER_ID

**Purpose:** USPS Web Tools API user ID; authenticates carrier integration test calls to USPS Rate and Address APIs.

**Minimum scope:** USPS Web Tools: Rate Calculator API access only. No Shipping Label or Address Verification APIs required for current tests. UNKNOWN — confirm the API user account has only the required APIs enabled in the USPS developer portal.

**Rotation cadence:** UNKNOWN — USPS Web Tools does not publish a standard rotation process. Rotate on any suspected exposure and confirm procedure with the USPS developer account settings.

**Environment reuse rule:** CI-only. UNKNOWN — confirm whether this user ID targets sandbox or production USPS endpoints.
