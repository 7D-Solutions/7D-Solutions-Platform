# Tenant Context Propagation Audit — 2026-04-11

**Bead:** bd-recec  
**Author:** MaroonHarbor  
**Date:** 2026-04-11  
**Scope:** All service-to-service calls in `platform/` and `modules/` that could carry a nil or wrong tenant UUID.

---

## Executive Summary

The fix in bd-s56d3 (`inject_headers` per-request JWT minting) is working on the happy path. Every `PlatformClient` call now attempts to mint a fresh RSA JWT carrying the caller's real `tenant_id` before falling back to the startup-time bearer token. No cross-module call path was found that bypasses `inject_headers` or hardcodes nil UUIDs in the `VerifiedClaims` it passes to the SDK.

**Primary risk vector (mitigated by bd-s56d3):**  
When `JWT_PRIVATE_KEY_PEM` is absent from the environment, `mint_service_jwt_with_context` fails and every cross-service call falls back to the nil-UUID bearer token minted at startup. This is the footgun the fix addresses.

**Current status:** Safe when `JWT_PRIVATE_KEY_PEM` is set in all environments. The canary test at `e2e-tests/tests/tenant_context_canary_e2e.rs` enforces this on every PR.

---

## Audit Method

Four grep patterns across the workspace:

```
1. get_service_token()       — startup-time nil-UUID token callers
2. bearer_token              — static bearer token references
3. PlatformClient::new       — direct construction (bypasses manifest wiring)
4. reqwest::Client           — raw HTTP without inject_headers
```

---

## Findings

### Pattern 1 — `get_service_token()` callers

| File | Line | Finding | Status |
|------|------|---------|--------|
| `tools/tenantctl/src/verify.rs` | 97 | CLI admin tool uses startup token for connectivity verification | **SAFE** — admin tool, not a cross-tenant data operation |
| `platform/platform-sdk/src/platform_services.rs` | 75 | Startup token minted at service boot; stored as fallback in PlatformClient | **SAFE** — `inject_headers` prefers per-request JWT. Comment added. |
| `platform/security/src/service_auth.rs` | 223 | Definition of `get_service_token`; embeds nil UUIDs when JWT_PRIVATE_KEY_PEM is set | **SAFE** — intended behavior; canary documents nil-UUID danger |
| `e2e-tests/tests/service_to_service_auth_e2e.rs` | 193 | Test verifying startup token behavior | **N/A** — test code |

**Assessment:** `get_service_token()` is intentionally nil-UUID by design (no request context at startup). The fix in bd-s56d3 means `inject_headers` never reaches the fallback bearer token as long as `JWT_PRIVATE_KEY_PEM` is set.

---

### Pattern 2 — `bearer_token` in production code

| File | Line | Finding | Status |
|------|------|---------|--------|
| `platform/platform-sdk/src/http_client.rs` | 180 | `bearer_token` used as fallback after per-request JWT minting attempt | **SAFE** — fallback only; per-request JWT tried first via `mint_service_jwt_with_context` |
| `modules/bom/src/domain/numbering_client.rs` | 86–88 | Http mode extracts inbound user JWT and forwards to Numbering; `claims` passed to `typed.allocate()` | **SAFE** — inbound user JWT forwarded; inject_headers mints per-request JWT from those claims |
| `modules/bom/src/domain/numbering_client.rs` | 119–123 | Same pattern for `confirm_eco_number` | **SAFE** — same reasoning |
| `modules/shipping-receiving/src/integrations/inventory_client.rs` | 123, 191 | Http mode sets static bearer + passes `PlatformClient::service_claims(tenant_id)` | **SAFE** — inject_headers tries per-request JWT from service_claims; static bearer is redundant fallback |
| `modules/quality-inspection/src/http/inspection_routes.rs` | 26 | `wc_with_bearer` forwards inbound request JWT to WC service | **SAFE** — correctly propagating authenticated caller JWT; inject_headers still mints fresh JWT from claims |
| `modules/quality-inspection/tests/*` | — | Test helpers using `with_bearer_token` | **N/A** — test code only |

---

### Pattern 3 — `PlatformClient::new` in production code

| File | Line | Finding | Status |
|------|------|---------|--------|
| `modules/bom/src/domain/inventory_client.rs` | 73 | Http mode `PlatformClient::new(base_url)` without bearer; passes inbound `claims` to `fetch_via_http` | **SAFE** — inject_headers mints per-request JWT from inbound claims; no unauthenticated gap if JWT_PRIVATE_KEY_PEM is set |
| `modules/ttp/src/clients/ar.rs` | 69 | `ArClient::new(base_url)` wraps `PlatformClient::new`; no static bearer | **SAFE** — inject_headers mints per-request JWT from VerifiedClaims passed at call time |
| `modules/ttp/src/clients/tenant_registry.rs` | 45 | SDK-wired via `PlatformService::from_platform_client` | **SAFE** — SDK wiring, claims passed at each call |
| `platform/identity-auth/src/clients/tenant_registry.rs` | 109 | SDK-wired | **SAFE** |
| `modules/consolidation/src/integrations/{ap,ar,gl}/client.rs` | ~31 | SDK-wired via `PlatformClient::new` with base URL from config | **SAFE** — inject_headers called with VerifiedClaims from each request |
| `modules/ar/src/integrations/party_client.rs` | 107 | `PlatformClient::new("http://127.0.0.1:19999")` | **N/A** — test fixture (hardcoded localhost port) |

---

### Pattern 4 — Raw `reqwest::Client` in production code

All raw `reqwest::Client` usages are for **external third-party APIs** (Stripe payments, UPS/FedEx/USPS carrier APIs, QuickBooks Online, eBay/Amazon marketplace APIs, SMTP/email delivery). None carry platform tenant context through `inject_headers`.

| Module | Usage | Status |
|--------|-------|--------|
| `modules/payments/src/processor.rs` | Stripe payment processor API | **SAFE** — external API, not cross-tenant platform call |
| `modules/payments/src/http/checkout_sessions/session_logic.rs` | Stripe checkout | **SAFE** |
| `modules/shipping-receiving/src/domain/carrier_providers/` | UPS, FedEx, USPS APIs | **SAFE** — external carrier APIs |
| `modules/integrations/src/` | QuickBooks Online, OAuth refresh | **SAFE** — external SaaS integrations |
| `modules/ar/src/tilled/` | Tilled payment gateway | **SAFE** — external payment processor |
| `modules/notifications/src/scheduled/sender.rs` | Email delivery (SMTP/sendgrid) | **SAFE** — external email API |
| `platform/tenant-registry/src/` | Health checks to module `/api/ready` endpoints | **SAFE** — health checks, no tenant data |
| `platform/auth-kit/src/` | JWKS key fetch | **SAFE** — public key fetch, no tenant data |

---

## Risk Summary

| Risk | Severity | Status |
|------|----------|--------|
| Nil-UUID bearer token from `get_service_token()` used as JWT claim | CRITICAL | **MITIGATED** — inject_headers prefers per-request JWT |
| `JWT_PRIVATE_KEY_PEM` absent in environment | HIGH | **OPERATIONAL CONTROL** — must be set in prod/staging; canary documents danger |
| Static bearer set in `platform_services.rs` startup token | MEDIUM | **MITIGATED** — inject_headers fallback only, per-request JWT tried first |
| Http-mode clients with no bearer + no JWT key | MEDIUM | **MITIGATED by deploy requirement**: JWT_PRIVATE_KEY_PEM required |

---

## Required Deployment Control

`JWT_PRIVATE_KEY_PEM` **MUST** be set in all production and staging environments. Without it, `mint_service_jwt_with_context` fails silently (logged as warn) and cross-service calls use either:
- The nil-UUID static bearer token (if service started with `get_service_token()`)
- No Authorization header at all

Both are wrong for tenant-isolated multi-tenant operations.

**Verification:** The canary test `tenant_context_canary_e2e` runs on every PR and fails fast if inject_headers produces nil or wrong-tenant UUIDs.

---

## Files Reviewed

```
platform/platform-sdk/src/http_client.rs
platform/platform-sdk/src/platform_services.rs
platform/platform-sdk/src/startup.rs
platform/security/src/service_auth.rs
platform/security/src/lib.rs
modules/bom/src/domain/numbering_client.rs
modules/bom/src/domain/inventory_client.rs
modules/shipping-receiving/src/integrations/inventory_client.rs
modules/quality-inspection/src/http/inspection_routes.rs
modules/ar/src/integrations/party_client.rs
modules/ttp/src/clients/ar.rs
modules/ttp/src/clients/tenant_registry.rs
modules/consolidation/src/integrations/ap/client.rs
modules/consolidation/src/integrations/ar/client.rs
modules/consolidation/src/integrations/gl/client.rs
platform/identity-auth/src/clients/tenant_registry.rs
modules/payments/src/processor.rs
modules/notifications/src/scheduled/sender.rs
modules/integrations/src/domain/qbo/client.rs
tools/tenantctl/src/verify.rs
```
