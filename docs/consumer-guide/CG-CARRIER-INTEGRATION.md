# Consumer Guide — Carrier Integration

> **Who reads this:** Agents and developers building vertical apps (Fireproof ERP, TrashTech, Huber) on the 7D Platform.
> **What it covers:** Per-carrier developer portal setup, required env vars, auth method, webhook URLs, token lifecycle, and troubleshooting.
> **Parent:** [PLATFORM-CONSUMER-GUIDE.md](./PLATFORM-CONSUMER-GUIDE.md)

## Contents

1. [Path Selection by Carrier](#1-path-selection-by-carrier)
2. [UPS Setup (OAuth — authorization_code)](#2-ups-setup-oauth--authorization_code)
3. [FedEx Setup (OAuth — client_credentials)](#3-fedex-setup-oauth--client_credentials)
4. [USPS Setup (API Key)](#4-usps-setup-api-key)
5. [R&L Carriers Setup (API Key)](#5-rl-carriers-setup-api-key)
6. [XPO / ODFL / Saia Setup (API Key)](#6-xpo--odfl--saia-setup-api-key)
7. [Webhook Endpoints](#7-webhook-endpoints)
8. [Token and Credential Lifecycle](#8-token-and-credential-lifecycle)
9. [Troubleshooting](#9-troubleshooting)

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-04-25 | Platform Orchestrator | Initial — seven-carrier integration guide: OAuth (UPS/FedEx) and API-key (USPS/R&L/XPO/ODFL/Saia) auth paths, webhook endpoints, token lifecycle, troubleshooting. |
| 1.1 | 2026-04-25 | BrightSparrow (bd-4dbh3) | Section 7 expanded: per-carrier webhook URLs, auth details, idempotency guarantee, multi-package master recomputation rule, ODFL 15-min poll fallback. |

---

## 1. Path Selection by Carrier

The platform supports two auth patterns across seven carriers. Choose the right section based on this table.

| Carrier | Freight type | Auth method | Developer portal |
|---------|-------------|-------------|-----------------|
| UPS | Parcel | OAuth 2.0 — `authorization_code` + refresh token | [developer.ups.com](https://developer.ups.com) |
| FedEx | Parcel | OAuth 2.0 — `client_credentials` (no browser redirect) | [developer.fedex.com](https://developer.fedex.com) |
| USPS | Parcel | API key (legacy user_id/password) | [developer.usps.com](https://developer.usps.com) |
| R&L Carriers | LTL | API key | rlcarriers.com developer portal |
| XPO Logistics | LTL | API key | [developer.xpo.com](https://developer.xpo.com) |
| Old Dominion (ODFL) | LTL | API key | odfl.com developer portal |
| Saia | LTL | API key | saia.com developer portal |

**Auth method distinction matters for your UX design:**

- **UPS** — browser redirect flow. The tenant clicks a "Connect UPS" button, is redirected to UPS, authorizes, and returns. One platform app registration serves all tenants.
- **FedEx** — no browser redirect. FedEx issues per-app credentials and bills per application, so each tenant must supply their own `client_id` and `client_secret` via a paste form. The platform mints access tokens server-side.
- **API-key carriers (USPS, R&L, XPO, ODFL, Saia)** — tenant supplies credentials via a paste form. No OAuth redirect.

---

## 2. UPS Setup (OAuth — authorization_code)

### Developer Portal Registration

1. Go to [developer.ups.com](https://developer.ups.com) and sign in with your UPS account.
2. Create a new application. UPS calls this an "app" scoped to a set of APIs.
3. Under **Redirect URIs**, add exactly:
   ```
   https://<platform-host>/api/integrations/oauth/callback/ups
   ```
   UPS performs an exact-match check. Any trailing slash, capitalization difference, or extra parameter will cause `invalid_redirect_uri` errors.
4. Select the **Shipping** scope (covers label creation, rate quotes, and tracking).
5. Note your **Client ID** and **Client Secret** from the app detail page.

### Environment Variables (platform host)

```env
UPS_CLIENT_ID=<your-ups-client-id>
UPS_CLIENT_SECRET=<your-ups-client-secret>
UPS_REDIRECT_URI=https://<platform-host>/api/integrations/oauth/callback/ups
```

These are set once on the platform host. Tenants do not manage UPS credentials directly — they authorize via the Connect UPS button.

### Tenant Flow

1. Tenant navigates to their carrier settings page.
2. They click **Connect UPS**. The platform redirects them to `https://onlinetools.ups.com/security/v1/oauth/authorize` with your `client_id` and `redirect_uri`.
3. The tenant logs into their UPS account and approves the authorization.
4. UPS redirects back to `/api/integrations/oauth/callback/ups` with an authorization `code`.
5. The platform exchanges the code for `access_token` + `refresh_token` and stores them against the tenant record.
6. The tenant is connected. They do not repeat this flow unless their refresh token expires without activity (see [Section 8](#8-token-and-credential-lifecycle)).

### Sandbox vs Production

UPS provides a sandbox at `https://wwwcie.ups.com`. Set `UPS_BASE_URL` to the sandbox URL during development. Label generation and rate calls in sandbox do not generate real shipments. Tracking events in sandbox are simulated.

---

## 3. FedEx Setup (OAuth — client_credentials)

> **Key difference from UPS:** FedEx uses the `client_credentials` grant, which is a server-to-server token exchange — there is no browser redirect, no authorization URL, and no callback endpoint needed. The tenant does not click an "Authorize" button. Instead, the tenant's admin pastes their own FedEx `client_id` and `client_secret` into a form, and the platform mints tokens on their behalf server-side.

> **Why per-tenant credentials:** FedEx bills per application and enforces rate limits per app registration. One shared platform app cannot serve multiple tenants without billing and rate-limit collisions. Each tenant must register their own FedEx developer app and supply their credentials.

### Tenant Registration at FedEx

Each tenant must:

1. Go to [developer.fedex.com](https://developer.fedex.com) and create a developer account.
2. Create a new project and add the **Ship** and **Track** APIs.
3. Under the project settings, retrieve **Client ID** and **Client Secret**.
4. In their platform carrier settings, paste those values into the FedEx credential form.

### Environment Variables (platform host)

```env
FEDEX_CLIENT_ID=<placeholder-not-used-for-auth>
FEDEX_CLIENT_SECRET=<placeholder-not-used-for-auth>
FEDEX_REDIRECT_URI=<placeholder-not-used-for-oauth-redirect>
```

`FEDEX_REDIRECT_URI` is a placeholder. FedEx `client_credentials` does not redirect — no callback URL is registered or used. These platform-level vars are unused for auth; tenant credentials are stored per-tenant in the database.

### Credential Storage

```
POST /api/integrations/carriers/fedex/credentials
Content-Type: application/json
x-tenant-id: <tenant-uuid>

{
  "client_id": "<tenant-fedex-client-id>",
  "client_secret": "<tenant-fedex-client-secret>"
}
```

The platform encrypts and stores these. On every shipping API call for that tenant, it mints a fresh `client_credentials` token from `https://apis.fedex.com/oauth/token`.

### Sandbox vs Production

FedEx provides a sandbox at `https://apis-sandbox.fedex.com`. Label responses in sandbox are valid JSON but produce non-scannable barcodes. Tracking in sandbox returns fixed simulated events.

---

## 4. USPS Setup (API Key)

USPS currently uses a legacy username/password credential pair, not a modern API key or OAuth token.

### Tenant Registration at USPS

1. Go to [developer.usps.com](https://developer.usps.com) and register for API access.
2. USPS issues a **User ID** and **Password** for the Web Tools API.
3. In their platform carrier settings, paste the User ID and Password into the USPS credential form.

### Credential Storage

```
POST /api/integrations/carriers/usps/credentials
Content-Type: application/json
x-tenant-id: <tenant-uuid>

{
  "user_id": "<usps-web-tools-user-id>",
  "password": "<usps-web-tools-password>"
}
```

The platform stores these credentials and passes them as request fields in USPS XML API calls. There is no token exchange step.

### Future OAuth Path

USPS is migrating to OAuth 2.0. Updating the platform to support USPS OAuth is tracked in a separate bead and is not in scope for this guide revision.

---

## 5. R&L Carriers Setup (API Key)

### Tenant Registration at R&L

1. Go to the R&L Carriers developer portal at rlcarriers.com.
2. Request API access. R&L may require a freight account number to issue API credentials.
3. R&L issues a single **API key**.
4. Paste the API key into the platform R&L credential form.

### Credential Storage

```
POST /api/integrations/carriers/rl/credentials
Content-Type: application/json
x-tenant-id: <tenant-uuid>

{
  "api_key": "<rl-api-key>"
}
```

---

## 6. XPO / ODFL / Saia Setup (API Key)

All three LTL carriers use the same paste-field pattern. Each has their own developer portal where the tenant registers and obtains an API key.

| Carrier | Developer portal |
|---------|-----------------|
| XPO Logistics | [developer.xpo.com](https://developer.xpo.com) |
| Old Dominion (ODFL) | odfl.com developer portal |
| Saia | saia.com developer portal |

### Credential Storage

Replace `{carrier}` with `xpo`, `odfl`, or `saia`:

```
POST /api/integrations/carriers/{carrier}/credentials
Content-Type: application/json
x-tenant-id: <tenant-uuid>

{
  "api_key": "<carrier-api-key>"
}
```

All three LTL carriers authenticate per-tenant using the stored API key on each outbound API call. There is no token exchange or refresh cycle.

---

## 7. Webhook Endpoints

All carrier webhooks are unauthenticated at the platform ingress — the carrier controls the call. Each handler performs its own payload-level auth (HMAC or shared token) before processing. A signature mismatch returns **401**; no event is recorded.

The canonical tracking status vocabulary is: `pending`, `picked_up`, `in_transit`, `out_for_delivery`, `delivered`, `exception`, `returned`, `lost`.

Every new tracking event emits a `shipping_receiving.tracking.event_received` event on the NATS bus. Consumers (notification dispatch, visibility UI) subscribe to that subject.

| Carrier | Endpoint | Auth | Push available |
|---------|----------|------|----------------|
| UPS | `POST /api/integrations/carriers/ups/webhook` | HMAC-SHA256 (`X-Ups-Webhook-Signature: v1=<hex>`) | Yes |
| FedEx | `POST /api/integrations/carriers/fedex/webhook` | HMAC-SHA256 (`X-FedEx-Signature`) + initial challenge | Yes |
| USPS | `POST /api/integrations/carriers/usps/webhook` | N/A | **No** — returns 501 |
| R&L | `POST /api/integrations/carriers/rl/webhook` | Shared token (`X-RL-Webhook-Token`) | Yes |
| XPO | `POST /api/integrations/carriers/xpo/webhook` | HMAC-SHA256 (`X-Xpo-Signature`) | Yes (premium tier) |
| ODFL | `POST /api/integrations/carriers/odfl/webhook` | N/A | **No** — returns 501 |
| Saia | `POST /api/integrations/carriers/saia/webhook` | HMAC-SHA256 (`X-Saia-Signature`) | Yes |

### UPS

Register this URL in the UPS developer portal under **Tracking Event Subscriptions**:

```
POST /api/integrations/carriers/ups/webhook
```

The platform verifies the HMAC-SHA256 signature in `X-Ups-Webhook-Signature: v1=<hex>`. The webhook secret is the `UPS_WEBHOOK_SECRET` env var on the platform host. Set this to the secret UPS shows in the portal after registration.

### FedEx

Register this URL via the FedEx webhook subscription API (not the developer portal UI):

```
POST /api/integrations/carriers/fedex/webhook
```

FedEx performs a **challenge-response** on initial registration: it sends `{ "event": { "eventType": "webhookSetup" }, "challengeToken": "..." }` and expects `{ "challengeToken": "..." }` in response. The platform handles this automatically. Subsequent event deliveries are HMAC-verified using `FEDEX_WEBHOOK_SECRET`.

### USPS

USPS's legacy Web Tools API does not support webhook push. The endpoint returns **501 Not Implemented** so carrier portal setup fails fast. USPS tracking is not currently polled — use a carrier with push support for real-time parcel visibility.

### R&L Carriers

```
POST /api/integrations/carriers/rl/webhook
```

R&L sends the shared token in `X-RL-Webhook-Token`. Set `RL_WEBHOOK_SECRET` on the platform host to the value configured in the R&L portal.

### XPO Logistics

```
POST /api/integrations/carriers/xpo/webhook
```

XPO webhook push requires the premium API tier. The platform verifies `X-Xpo-Signature` using `XPO_WEBHOOK_SECRET`.

### ODFL

ODFL does not offer webhook push. The endpoint returns **501 Not Implemented**. The platform runs a background poll task every **15 minutes** for all in-transit ODFL shipments. When polling detects a status change, the same `tracking.event_received` event is emitted.

### Saia

```
POST /api/integrations/carriers/saia/webhook
```

Saia verifies `X-Saia-Signature` (HMAC-SHA256). Set `SAIA_WEBHOOK_SECRET` on the platform host.

### Idempotency

Carrier webhook deliveries are guaranteed idempotent. The platform computes `SHA-256(raw_body)` as `raw_payload_hash` and rejects duplicate inserts via a unique constraint. Webhook retries and replay storms produce exactly one `tracking_events` row.

### Multi-package shipments

For carriers that issue a master tracking number plus per-package child tracking numbers (e.g., UPS multi-package), create child `shipments` rows with `parent_shipment_id` pointing to the master. When any child receives a webhook update, the platform recomputes the master's `carrier_status` as the **least advanced** status across all children — the master shows `in_transit` until every child is delivered, and surfaces `exception` immediately if any child is damaged or lost.

---

## 8. Token and Credential Lifecycle

### UPS

- **Access token TTL:** ~4 hours.
- **Refresh token TTL:** Long-lived. The background token refresh worker (`bd-6rlla`) rotates access tokens before they expire using the stored refresh token.
- **Reconnect required:** Only if the refresh token itself expires due to extended inactivity (tenant has not made any API calls in a very long time). In this case, the tenant must re-authorize via the Connect UPS button.
- **Rotation:** Handled automatically by the platform. No tenant action needed during normal operation.

### FedEx

- **Access token TTL:** ~1 hour.
- **Refresh:** The platform re-mints a `client_credentials` token from FedEx on each tenant's behalf using their stored `client_id` and `client_secret`. The background worker (`bd-6rlla`) handles this.
- **No refresh token:** `client_credentials` grants do not issue refresh tokens. Each mint is a fresh exchange.
- **Reconnect required:** Never from the auth perspective — as long as the tenant's `client_id` and `client_secret` remain valid. If FedEx revokes or rotates the app credentials, the tenant must paste new credentials.

### API-Key Carriers (USPS, R&L, XPO, ODFL, Saia)

- **No token lifecycle:** API keys do not expire on a short TTL. They are valid until the tenant or carrier revokes them.
- **Rotation:** Follow the carrier key rotation schedule documented in [docs/operations/secret-rotation.md](../operations/secret-rotation.md). Typically: generate new key in carrier portal, update via the credential endpoint, verify next API call succeeds, then delete the old key in the carrier portal.
- **If a key is compromised:** Revoke immediately in the carrier portal and post new credentials via the credential endpoint. The old key stops working the moment the carrier revokes it.

---

## 9. Troubleshooting

### 401 Unauthorized after a working connection

**Most common cause (OAuth carriers):** The UPS refresh token or FedEx `client_credentials` token has expired and the background worker (`bd-6rlla`) is not running or encountered an error.

Steps:
1. Check whether the background token refresh worker is running. If not, restart it.
2. For UPS: if the refresh token itself has expired (extended inactivity), the tenant must reconnect via the Connect UPS button.
3. For FedEx: verify the tenant's `client_id` and `client_secret` are still valid in their FedEx developer portal. Re-paste if they have been rotated.

**Most common cause (API-key carriers):** The carrier revoked the key, or the key was entered incorrectly.

Steps:
1. Re-paste the API key via the credential endpoint.
2. Verify the key in the carrier's developer portal.

---

### `invalid_redirect_uri` (UPS only)

The redirect URI registered in the UPS developer portal does not exactly match `UPS_REDIRECT_URI` in the platform config.

Steps:
1. Copy the exact URI from the UPS developer portal app settings.
2. Compare character-by-character with `UPS_REDIRECT_URI`. Look for: trailing slash, `http` vs `https`, extra query parameters, or subdomain difference.
3. Update the platform env var to match exactly, or update the portal registration to match the platform config. They must be identical.

---

### Wrong environment (sandbox vs production)

Symptoms: labels generate successfully but barcodes are not scannable, or tracking returns simulated events, or rates differ significantly from live quotes.

Steps:
1. Check whether the carrier base URL env var points to the sandbox endpoint.
2. For UPS: `UPS_BASE_URL` should be `https://onlinetools.ups.com` in production.
3. For FedEx: `FEDEX_BASE_URL` should be `https://apis.fedex.com` in production.
4. Verify the `client_id`/`client_secret` are from the production app registration, not the sandbox app. UPS and FedEx issue separate credentials per environment.

---

### LTL PRO number lookup returning 404

The carrier's tracking API did not find the PRO number.

Possible causes:
1. **Tracking retention window:** LTL carriers purge tracking data after a period (typically 90–180 days). Historical shipments may no longer be findable.
2. **PRO number not yet active:** Some LTL carriers take several hours after label creation before a PRO number becomes queryable.
3. **Wrong carrier:** Verify the PRO number format matches the carrier. XPO, ODFL, and Saia use different PRO number formats.
4. **Data entry error:** Re-verify the PRO number against the original label.

---

### "No shipments returned" after connecting a carrier

This is almost always a missing or incorrect `x-tenant-id` header, not a carrier auth problem. Verify the tenant UUID in the request matches the tenant for which credentials were stored.

See [CG-AUTH.md](./CG-AUTH.md) for required HTTP headers.
