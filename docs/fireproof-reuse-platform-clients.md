# Fireproof ERP Reuse Investigation: Platform Clients, Notification Templates, Numbering Registry

**Author:** CopperRiver
**Bead:** bd-3ashx
**Date:** 2026-03-05

---

## Executive Summary

Fireproof ERP built typed HTTP clients for 6 platform services, plus a notification template registration pattern and a numbering entity registry. All 6 clients share an identical structure: `reqwest::Client` + exponential backoff retry + a shared `ClientError` enum. This is textbook SDK material.

**Verdict:** Extract the HTTP client base + `ClientError` as a shared crate. Adapt the notification template and numbering registry patterns into platform conventions. Individual service clients stay in the vertical (they mirror vertical-specific API surface), but the plumbing should not be duplicated.

---

## Module-by-Module Assessment

### 1. Shared HTTP Client Base + ClientError

**Source:** `identity_auth/client.rs` lines 1-160 (the `ClientError` enum and retry logic)
**LOC:** ~160
**Recommendation:** EXTRACT

Every Fireproof client (`NotificationsClient`, `NumberingClient`, `SodClient`, `PartyClient`, `TenantRegistryClient`, `IdentityAuthClient`) copy-pastes the same pattern:

- `reqwest::Client::builder()` with timeout + connect_timeout
- `base_url.trim_end_matches('/')`
- `max_retries` with exponential backoff (100ms * 2^attempt)
- Retry on 5xx and timeout/connect errors
- Fail fast on 4xx
- `ClientError` with 5 variants: `Http`, `HttpWithBody`, `Server`, `Network`, `Decode`

This is duplicated ~6 times across ~800 lines of boilerplate. A platform SDK crate (`platform-client` or `seven_d_sdk`) could provide:

```rust
pub struct PlatformHttpClient {
    http: reqwest::Client,
    base_url: String,
    max_retries: u32,
}

impl PlatformHttpClient {
    pub fn new(base_url: &str, timeout: Duration, max_retries: u32) -> Result<Self, String>;
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str, headers: HeaderMap) -> Result<T, ClientError>;
    pub async fn post_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, headers: HeaderMap, body: &B) -> Result<T, ClientError>;
    pub async fn put_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, headers: HeaderMap, body: &B) -> Result<T, ClientError>;
    pub async fn patch_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, headers: HeaderMap, body: &B) -> Result<T, ClientError>;
    pub async fn delete(&self, path: &str, headers: HeaderMap) -> Result<(), ClientError>;
}
```

Each service-specific client would then wrap `PlatformHttpClient` instead of reimplementing retry logic.

**Dependencies to bring:** `reqwest`, `serde`, `serde_json`, `tokio`, `tracing`
**Manufacturing roadmap phase:** Cross-cutting. Benefits every phase that calls a platform service (all of them).

---

### 2. NotificationsClient

**Source:** `platform/notifications_client.rs` (429 LOC)
**Recommendation:** ADAPT-PATTERN

The client itself is Fireproof-specific in its API surface (e.g., the `send()` method takes a `TemplateKey` enum specific to Fireproof). But the *shape* is generic:

- `send(auth_token, tenant_id, template_key, recipients, payload)` -> `DeliveryReceipt`
- `create_template(auth_token, tenant_id, request)` -> `TemplateResponse`
- `get_template(auth_token, tenant_id, key)` -> `TemplateResponse`
- `get_notification(auth_token, tenant_id, id)` -> `NotificationDetail`
- `get_delivery_status(auth_token, tenant_id, correlation_id)` -> `DeliveryReceipt`

**Comparison with platform API:**
- `POST /api/notifications/send` matches (platform has `SendRequest` with `template_key`, `recipients`, `payload_json`, `channel`, `correlation_id`, `causation_id`)
- `POST /api/templates` matches (platform has `CreateTemplate` with `template_key`, `channel`, `subject`, `body`, `required_vars`)
- `GET /api/templates/{key}` matches
- `GET /api/notifications/{id}` matches
- `GET /api/deliveries` matches (platform uses query params: `correlation_id`, `recipient`, `from`, `to`, `limit`, `offset`)

**Mismatch:** Fireproof client passes `template_key` as a string but validates with a Fireproof-specific `TemplateKey` enum. The platform doesn't enforce this — the template_key is just a string. This is correct: the vertical owns its enum, the platform accepts any key.

**DTO alignment:** Fireproof's DTOs (`CreateTemplateRequest`, `TemplateResponse`, `DeliveryReceipt`) are *mostly* aligned but use slightly different field names than the platform:
- Fireproof: `subject_template` / `body_template` vs Platform: `subject` / `body`
- Fireproof: `required_vars: Vec<String>` vs Platform: `required_vars: Vec<String>` (match!)
- Fireproof: `DeliveryReceipt.notification_id` vs Platform: `SendResponse.id`

If the shared SDK is built, a `NotificationsClient` wrapper would be ~50 LOC (method signatures + DTO mapping), not the 260+ LOC of retry plumbing it currently contains.

**Manufacturing roadmap:** Phase B (production notifications for WO state changes), Phase C (inspection hold/release notifications), Phase D (ECO distribution notifications).

---

### 3. Notification Template Registration Pattern

**Source:** `platform/notification_templates.rs` (237 LOC)
**Recommendation:** ADAPT-PATTERN

This is a well-designed pattern that should become a platform convention:

1. **TemplateKey enum** — each vertical declares its notification templates as an enum
2. **required_variables()** — per-template variable validation
3. **to_seed_request()** — generates the `CreateTemplateRequest` for seeding
4. **seed_templates()** — idempotent bulk registration at startup
5. **validate_payload()** — client-side validation before HTTP call

The pattern is vertical-specific (Fireproof has `ApprovalRequested`, `HoldApplied`, etc.) but the *mechanism* is generic. A manufacturing vertical would have its own enum with keys like `work_order_released`, `inspection_failed`, `eco_distributed`.

**Recommendation:** Document this as the standard notification integration pattern in the platform SDK docs. Provide a trait or macro:

```rust
pub trait NotificationTemplateSet {
    fn all() -> &'static [Self] where Self: Sized;
    fn key(&self) -> &'static str;
    fn required_variables(&self) -> &'static [&'static str];
    fn default_subject(&self) -> &'static str;
    fn default_body(&self) -> &'static str;
    fn channel(&self) -> &'static str { "email" }
}
```

**Manufacturing roadmap:** Phase B (WO lifecycle notifications), Phase C (inspection notifications). Template registration would happen during module initialization.

---

### 4. NumberingClient

**Source:** `platform/numbering_client.rs` (246 LOC)
**Recommendation:** ADAPT-PATTERN

The client wraps two endpoints:
- `allocate(auth_token, tenant_id, entity)` -> `AllocateResponse`
- `peek(auth_token, tenant_id, entity)` -> `PeekResponse`

**Comparison with platform API:**
- Platform `POST /allocate` expects `entity`, `idempotency_key`, `gap_free` (optional)
- Fireproof client generates the `AllocateRequest` differently: it uses `pattern_name` and `gap_free` from the registry, not `entity` and `idempotency_key`

**Critical mismatch:** Fireproof's `AllocateRequest` uses `pattern_name` and `tenant_id` as fields, but the platform expects `entity` and `idempotency_key`. The platform derives `tenant_id` from JWT claims. This means the Fireproof client as-is would NOT work against the current platform API — it needs DTO alignment.

The Fireproof client also lacks `idempotency_key` support — this is a gap. The platform requires it for correctness.

With the shared SDK, this becomes ~30 LOC of wrapper over `PlatformHttpClient`.

**Manufacturing roadmap:** Phase A (BOM numbering), Phase B (WO numbering), Phase C (inspection ID numbering).

---

### 5. Numbering Registry Pattern

**Source:** `platform/numbering_registry.rs` (146 LOC)
**Recommendation:** ADAPT-PATTERN

This is a clean pattern: a `NumberedEntity` enum maps entity types to `SequenceMapping` (pattern_name + gap_free flag). Each vertical defines its own entities and their numbering config.

Fireproof entities: `WorkOrder`, `Ncr`, `CalibrationCert`, `Shipment`, `Quote`
Manufacturing would add: `WorkOrder`, `InspectionReport`, `Eco`, `Lot`, `SerialNumber` (some overlap with Fireproof)

This pattern is generic enough to be a platform convention. Each vertical registers its entities at startup. The registry ensures consistent numbering config without scattering it across the codebase.

**Recommendation:** Document as standard pattern. Consider a `NumberingRegistry` trait in the SDK:

```rust
pub trait NumberingRegistry {
    fn all() -> &'static [Self] where Self: Sized;
    fn pattern_name(&self) -> &'static str;
    fn gap_free(&self) -> bool;
}
```

**Manufacturing roadmap:** Phase 0 would define the entity list. Phase A onward uses it.

---

### 6. SodClient

**Source:** `platform/sod_client.rs` (336 LOC)
**Recommendation:** ADAPT-PATTERN

The client wraps three identity-auth SoD endpoints:
- `evaluate(auth_token, tenant_id, request)` -> `SodDecision`
- `create_policy(auth_token, tenant_id, request)` -> `CreateSodPolicyResponse`
- `list_policies(auth_token, tenant_id, action_key)` -> `Vec<SodPolicy>`

**Comparison with platform API:** The SoD endpoints exist in `platform/identity-auth/`. The DTOs (`SodEvaluateRequest`, `SodDecision`, `SodPolicy`) are well-defined and would work as shared types.

The `SodDecision.is_allowed()` helper is useful — any vertical needs to check this.

With the shared SDK, this is ~40 LOC of wrapper.

**Manufacturing roadmap:** Phase B (WO approval SoD), Phase C (inspector cannot self-accept SoD).

---

### 7. Delivery Receipt Queries

**Source:** `platform/delivery_receipts.rs` (146 LOC)
**Recommendation:** ADAPT-PATTERN

This module queries notification delivery records for audit compliance evidence. It uses the `NotificationsClient.get_json_public()` method to call `GET /api/deliveries` with filter params.

**Comparison with platform API:** The platform has `GET /api/deliveries` with `correlation_id`, `recipient`, `from`, `to`, `limit`, `offset` query params — exactly what the Fireproof `DeliveryQuery` builds.

This is needed for manufacturing audit compliance (aerospace/defense regulations require proof that notifications were sent and received). The pattern is generic.

With the shared SDK, the `DeliveryQuery` and `DeliveryRecord` types move to the notifications client section and this module becomes unnecessary as a separate file.

**Manufacturing roadmap:** Phase C (inspection notification evidence for AS9100).

---

### 8. PartyClient

**Source:** `party/client.rs` (1,074 LOC)
**Recommendation:** ADAPT-PATTERN

The largest client, covering full party CRUD:
- Companies: create, search
- Individuals: create
- Parties: list, get, update, patch (tags), search
- Contacts: create, list, update, delete, set-primary, get-primary

**Comparison with platform API:** Routes match exactly:
- `POST /api/party/companies` -> `create_company`
- `POST /api/party/individuals` -> `create_individual`
- `GET /api/party/parties` -> `list_parties`
- `GET /api/party/parties/{id}` -> `get_party`
- `PUT /api/party/parties/{id}` -> `update_party`
- `GET /api/party/parties/search` -> `search_parties`
- `POST /api/party/parties/{id}/contacts` -> `create_contact`
- etc.

The DTOs are large (~400 LOC) and mirror the platform's domain models. With a shared SDK, the retry plumbing (~500 LOC of get_json/post_json/put_json/patch_json/delete) collapses to the shared `PlatformHttpClient`. The remaining ~500 LOC of DTOs and method signatures is the genuine API surface.

**Note:** Fireproof uses `X-App-Id` header where the platform uses JWT tenant_id extraction. The party service uses `X-Correlation-Id` and `X-Actor-Id` headers — these are consistent.

**Manufacturing roadmap:** Phase A (vendor/supplier lookup for BOM sources), Phase C (calibration vendor management).

---

### 9. Admin (TenantRegistryClient + Control Plane)

**Source:** `admin/` (1,487 LOC total)
**Recommendation:** SKIP (for now)

The admin module contains:
- `tenant_registry.rs`: Client for `GET /api/tenants` (read-only tenant browsing)
- `control_plane.rs`: Control-plane interactions (tenant provisioning, config)
- `user_management.rs`: User CRUD via identity-auth

These are admin-plane clients, not data-plane. Manufacturing doesn't need them — they're for the vertical's admin UI. The retry plumbing would still benefit from the shared SDK, but extracting the admin client logic itself adds no manufacturing value.

**Manufacturing roadmap:** Not directly needed.

---

## Cross-Cutting Finding: Platform SDK Crate

### Should there be a platform SDK crate?

**Yes.** The evidence is overwhelming:

| Metric | Current State |
|--------|---------------|
| Copy-pasted retry logic | 6 clients x ~120 LOC = ~720 LOC of identical code |
| Copy-pasted `ClientError` | Referenced from `identity_auth::client` but ideally would be a shared crate |
| DTO drift risk | Fireproof DTOs already diverge from platform API in field naming (e.g., `subject_template` vs `subject`) |
| Second vertical cost | Manufacturing would need to write all these clients from scratch |

### Proposed structure

```
crates/platform-sdk/
  src/
    lib.rs              — re-exports
    client.rs           — PlatformHttpClient + ClientError (~200 LOC)
    notifications.rs    — NotificationsClient wrapper (~80 LOC)
    numbering.rs        — NumberingClient wrapper (~50 LOC)
    party.rs            — PartyClient wrapper (~120 LOC)
    sod.rs              — SodClient wrapper (~50 LOC)
    identity.rs         — IdentityAuthClient wrapper (~60 LOC)
```

**Total estimated LOC:** ~560, replacing ~2,500 LOC of current Fireproof client code.

### When to build it

Not now. The trigger condition for extracting a shared SDK is the **second consumer** — i.e., when the manufacturing vertical (or any other) needs these clients. Until then, Fireproof's clients work fine.

However, the *patterns* (template registration, numbering registry) should be documented NOW so manufacturing beads follow the same conventions from day one.

---

## Summary Table

| Component | LOC | Recommendation | Rationale | Mfg Phase |
|-----------|-----|---------------|-----------|-----------|
| HTTP client base + ClientError | ~160 | EXTRACT | Identical across 6 clients, pure boilerplate | Cross-cutting |
| NotificationsClient | 429 | ADAPT-PATTERN | Shape is generic, DTOs need alignment | B, C, D |
| Notification template registration | 237 | ADAPT-PATTERN | Convention, not code extraction | B, C |
| NumberingClient | 246 | ADAPT-PATTERN | Needs DTO alignment (missing idempotency_key) | A, B, C |
| Numbering registry | 146 | ADAPT-PATTERN | Convention for entity-to-sequence mapping | 0, A+ |
| SodClient | 336 | ADAPT-PATTERN | Wrapper shape is generic | B, C |
| Delivery receipt queries | 146 | ADAPT-PATTERN | Needed for audit compliance | C |
| PartyClient | 1,074 | ADAPT-PATTERN | Large but clean; retry code is boilerplate | A, C |
| Admin (tenant/user mgmt) | 1,487 | SKIP | Admin-plane, not needed for manufacturing | N/A |

---

## Recommended Next Beads

1. **Document platform client conventions** (docs-only bead): Write a guide for how verticals should build typed HTTP clients for platform services, including the notification template registration pattern and numbering registry pattern. No code — just docs.

2. **Platform SDK crate** (deferred until second consumer): Extract `PlatformHttpClient` + `ClientError` + thin wrappers for each service. Trigger: when manufacturing (or another vertical) needs to call platform services.

3. **Fix Fireproof numbering client DTO mismatch** (Fireproof-side bead): Fireproof's `AllocateRequest` uses `pattern_name` instead of `entity` and lacks `idempotency_key`. This should be fixed to align with the actual platform API contract.
