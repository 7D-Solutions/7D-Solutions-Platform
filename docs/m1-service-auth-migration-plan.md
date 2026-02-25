# M1 Migration Plan: Symmetric to Asymmetric Service-to-Service Auth

**Date:** 2026-02-25
**Bead:** bd-cww2 / bd-9xj6
**Status:** Assessment complete — implementation NOT started
**Severity:** MEDIUM (M1)

---

## 1. Current State

### Architecture

Service-to-service authentication uses **HMAC-SHA256 with a shared secret** (`SERVICE_AUTH_SECRET` env var). The implementation lives in `platform/security/src/service_auth.rs`.

**Token format:** `<base64url(claims_json)>.<base64url(hmac_signature)>`

**Claims structure:**
```rust
pub struct ServiceAuthClaims {
    pub service_name: String,  // e.g. "tenantctl", "ar-service"
    pub issued_at: i64,        // Unix seconds
    pub expires_at: i64,       // Unix seconds (default: +15 min)
}
```

### Risk

Any service (or compromised host) that knows `SERVICE_AUTH_SECRET` can:
- Generate tokens impersonating any other service
- There is no per-service identity verification
- No audience scoping — a token for "ar-service" is accepted by any verifier

This contrasts with the user-facing JWT system which uses **RS256 asymmetric signing** with per-key IDs, JWKS distribution, and zero-downtime rotation.

---

## 2. Callers and Callees Inventory

### Token Generators (callers)

| Component | File | Function | Purpose |
|-----------|------|----------|---------|
| `tenantctl` | `tools/tenantctl/src/verify.rs:96` | `get_service_token()` | Authenticates to module `/api/ready` and `/api/version` endpoints during tenant verification |
| `security` crate | `platform/security/src/service_auth.rs:154` | `get_service_token()` | Convenience wrapper: checks `SERVICE_TOKEN` env, falls back to `generate_service_token()` |

### Token Verifiers (callees)

| Component | File | Function | Purpose |
|-----------|------|----------|---------|
| AR module | `modules/ar/src/middleware/auth.rs:35` | `service_auth_middleware()` | Axum middleware protecting AR operational endpoints |

### Shared Infrastructure

| Component | File | Role |
|-----------|------|------|
| `security` crate | `platform/security/src/service_auth.rs` | Core: `generate_service_token()`, `verify_service_token()`, `get_service_token()` |
| `security` crate | `platform/security/src/lib.rs:34-37` | Re-exports all service_auth public types |

### Test Coverage

| File | Type |
|------|------|
| `platform/security/src/service_auth.rs` (lines 168-245) | Unit tests |
| `e2e-tests/tests/service_to_service_auth_e2e.rs` | E2E tests (token generation, verification, expiry, multi-service) |
| `modules/ar/src/middleware/auth.rs` (lines 108-176) | Integration tests for AR middleware |

---

## 3. Blast Radius Assessment

**Current blast radius is small:**

- **1 generator:** Only `tenantctl verify` actively generates service tokens
- **1 verifier:** Only the AR module uses `service_auth_middleware`
- **0 other modules** import or reference the service auth system

The service auth system is **lightly adopted** — most inter-service communication appears to not yet require authentication, or uses the user JWT passthrough pattern (forwarding the user's JWT to downstream services).

---

## 4. Proposed Migration Path

### Option A: Extend Existing JWT Infrastructure (Recommended)

Leverage the existing RS256 JWT system in `identity-auth` to issue **service-scoped JWTs**:

1. **Service accounts already supported:** `identity-auth/src/auth/jwt.rs` already supports `actor_type: "service"` and `app_id` fields. The `sign_access_token()` method accepts an `actor_type` parameter.

2. **Verifier already exists:** `platform/security/src/claims.rs` (`JwtVerifier`) already verifies RS256 JWTs and produces `VerifiedClaims` with `actor_type: ActorType::Service`. The `ClaimsMiddleware` in `authz_middleware.rs` is already deployed across all modules.

3. **JWKS already distributed:** `identity-auth` serves a `/.well-known/jwks.json` endpoint. Services already consume the public key via `JWT_PUBLIC_KEY` env var.

4. **Key rotation already solved:** `JwtVerifier::from_env_with_overlap()` and `JwtKeys::with_prev_key()` handle zero-downtime rotation.

**What would change:**
- Each service gets a **service account** in identity-auth (a row in the users table with `actor_type = "service"`)
- Each service holds its own credentials (or a long-lived refresh token) to obtain short-lived RS256 JWTs
- `service_auth_middleware` is replaced by the standard `ClaimsMiddleware` + `RequirePermissionsLayer` (already used for user auth)
- Per-service permissions become possible (e.g., `tenantctl` can call `/api/ready` but not `/api/invoices`)

### Option B: Separate Service-Specific RS256 Keypairs

Each service gets its own RSA keypair:
- Service signs tokens with its private key
- Verifier validates against a registry of service public keys

**Pros:** Strongest isolation — compromise of one service doesn't affect others
**Cons:** Key management overhead, need a new key registry, doesn't reuse JWT infra

### Recommendation

**Option A** is strongly recommended because:
- 90% of the infrastructure already exists
- No new key management system needed
- Unifies user and service auth into a single verification path
- The `VerifiedClaims` → `ActorType::Service` path is already exercised by E2E tests (`e2e-tests/tests/service_account_auth_e2e.rs`)

---

## 5. Migration Steps (Option A)

### Phase 1: Prepare (no breaking changes)
1. Ensure each service that needs to make authenticated calls has a service account in identity-auth
2. Add a `SERVICE_REFRESH_TOKEN` env var (or similar) for each service to obtain JWTs
3. Create a helper in the `security` crate: `get_service_jwt()` that exchanges credentials for a short-lived RS256 JWT

### Phase 2: Dual-mode verification (backwards compatible)
1. Update `service_auth_middleware` to accept **both** HMAC tokens (old) and RS256 JWTs (new)
2. Deploy the updated middleware
3. Migrate `tenantctl` to use RS256 JWTs via `get_service_jwt()`
4. Monitor: ensure no HMAC tokens are being generated

### Phase 3: Remove HMAC (breaking change)
1. Remove HMAC verification from middleware
2. Delete `platform/security/src/service_auth.rs`
3. Remove `SERVICE_AUTH_SECRET` from all environments
4. Update re-exports in `platform/security/src/lib.rs`
5. Update E2E tests in `e2e-tests/tests/service_to_service_auth_e2e.rs`

### Phase 4: Cleanup
1. Replace AR's custom `service_auth_middleware` with standard `ClaimsMiddleware`
2. Remove `modules/ar/src/middleware/auth.rs` (or repurpose for AR-specific logic)

---

## 6. Effort Estimate

| Phase | Files Changed | Effort |
|-------|--------------|--------|
| Phase 1: Prepare | 2-3 new/modified files in `security` crate + identity-auth seed | Small |
| Phase 2: Dual-mode | 3-4 files (middleware + tenantctl) | Small |
| Phase 3: Remove HMAC | 4-5 files (delete service_auth.rs, update lib.rs, tests) | Small |
| Phase 4: Cleanup | 2-3 files (AR middleware) | Small |

**Total: ~4 beads, low complexity each.** The blast radius is contained because only 2 components actively use the HMAC system today.

---

## 7. Environment Variables Affected

| Variable | Current Use | Migration Action |
|----------|------------|-----------------|
| `SERVICE_AUTH_SECRET` | HMAC signing key (shared across all services) | Remove in Phase 3 |
| `SERVICE_TOKEN` | Optional pre-generated token cache | Remove in Phase 3 |
| `SERVICE_NAME` | Fallback service name for token generation | Keep (useful for logging) |
| `SERVICE_REFRESH_TOKEN` (new) | Per-service credential for obtaining RS256 JWTs | Add in Phase 1 |

---

## 8. Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| Service account credentials leaked | Use short-lived refresh tokens; rotate regularly |
| Migration breaks tenantctl verification | Phase 2 dual-mode ensures backwards compatibility |
| Other services start using HMAC before migration | Document deprecation; block new HMAC adoption |
| identity-auth becomes SPOF for service auth | Service JWTs are self-contained; only token issuance requires identity-auth |
