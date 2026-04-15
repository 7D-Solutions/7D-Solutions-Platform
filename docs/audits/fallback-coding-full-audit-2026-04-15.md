# Fallback Coding & Degradation Paths — Full Audit

**Repository:** 7D-Solutions Platform  
**Date:** 2026-04-15  
**Recipient (Agent Mail):** lavenderwaterfall  
**Mode:** Static review + targeted tooling (no production traffic, no full-repo UBS convergence)

---

## How to read this document

Single shareable report for **fallback coding**: intentional degradation, env/key fallbacks, projection HTTP fallback, silent defaults, and related reliability or security footguns. Includes methodology (including **fsfs** discovery), multi-pass notes, deduplicated findings, and recommendations.

---

## Executive summary

| Severity | Count | Themes |
|----------|-------|--------|
| **High** | 0 | No confirmed exploitable vulns in this pass |
| **Medium** | 2 | Per-request projection fallback primitives (breaker/metrics); doc/example drift for JWT verifier |
| **Low** | 5 | HMAC service-token fallback; silent parse defaults; `fsfs` index path mismatch; UBS noise without baseline; module `CORS_ORIGINS` / `ENV` defaults |

**Already addressed (historical):** `tenantctl` and `projection-rebuild` previously used `JwtVerifier::from_env().or_else(from_env_with_overlap)`, which **skipped `JWT_PUBLIC_KEY_PREV`** whenever the primary PEM parsed. Both CLIs now use **`from_env_with_overlap()` only** (see git history for `[bd-c5r72]` or equivalent commit on your branch).

---

## Scope

**In scope**

- JWT / JWKS / env PEM / key rotation overlap (`JWT_PUBLIC_KEY_PREV`)
- Projection staleness → HTTP fallback (`platform/projections`, consumers such as payments)
- Service auth RSA vs HMAC (`platform/security/src/service_auth.rs`)
- Rate limiting: normal vs fallback path quotas (`platform/security/src/ratelimit.rs`, module middleware e.g. AR)
- Silent defaults: `unwrap_or` on env-derived limits/durations (`platform/platform-sdk/src/startup_helpers.rs`)
- Module config patterns surfaced by search: `PORT`, `ENV`, `CORS_ORIGINS`, upstream base URLs (`unwrap_or_else` to localhost / empty)

**Out of scope**

- Full UBS triage on entire monorepo (directory size + thousands of rule hits per crate without baseline)
- Runtime penetration or load testing

---

## Methodology

### Discovery: `fsfs` (primary for rerun)

From repo root, queries used (JSON, limit 8):

1. `JWT public key fallback JWKS rotation overlap` → key rotation runbooks, `identity-auth` JWT/JWKS, `proof_key_rotation.sh`
2. `projection HTTP fallback circuit breaker staleness` → `platform/projections/src/fallback/*`, `e2e-tests/tests/projection_fallback_circuit_e2e.rs`, payments projection route
3. `graceful degradation default unwrap_or env config silent` → degradation e2e, multiple `modules/*/src/config.rs`, notifications/consolidation mains
4. `service token HMAC RSA fallback ClaimsLayer` → `service_auth.rs`, migration docs, service-account e2e
5. `rate limit fallback path tenant IP` → `ratelimit.rs`, `modules/ar/src/middleware/ratelimit.rs`, overload e2e
6. `CORS_ORIGINS manifest fallback startup` → weaker signal; follow-up query `build_cors_layer CORS_ORIGINS startup_helpers body limit parse` → module routers + reporting/inventory CORS env parsing

**Caveat:** One `fsfs` hit referenced `modules/payments/src/routes/payments.rs`; the tree has **`modules/payments/src/http/payments.rs`** only. Treat as **index drift**; verify paths against the filesystem after `fsfs` hits.

### Multi-pass bug hunt (skill-aligned)

| Pass | Activity |
|------|----------|
| **1 — Surface** | `ubs` on `platform/security`, `platform/projections`, `modules/payments` (counts only; exit non-zero with many warnings) |
| **2 — Deep** | Read handlers, `JwtVerifier` constructors, `FallbackPolicy` / `CircuitBreaker` / `FallbackMetrics` |
| **3 — Integration** | `./scripts/cargo-slot.sh check -p projections -p payments-rs` and `test -p payments-rs --no-run` (compile tests) |
| **4 — Verify** | `ubs --fail-on-warning` on single crates **does not** converge without triage/baseline |

### Supplementary

- `rg` for exact symbols (`JwtVerifier::from_env`, `CircuitBreaker::new`)
- Prior security audit cross-reference: `docs/audits/SECURITY-AUDIT-2026-04-14-FULL.md` (CORS / wildcard class touches same `startup_helpers` area)

---

## Findings (deduplicated)

### M1 — Payments GET handler: fallback primitives not process-scoped (Medium)

**Location:** `modules/payments/src/http/payments.rs` (`get_payment`)

**Issue:** Each request constructs new `FallbackPolicy`, `FallbackMetrics::default()`, and `CircuitBreaker::new(5, 2)`. Circuit state and Prometheus registry backing metrics **do not persist** across requests, so the **circuit breaker does not protect the fleet** from a thundering herd; metrics are not attached to a shared exporter in this path.

**Why it matters:** If this handler is copied as a “production pattern,” operators get a false sense of breaker-backed safety.

**Recommendation:** Hold `Arc<CircuitBreaker>` (keyed by projection or tenant if needed) and shared `FallbackMetrics` / registry in `AppState`; or mark the route explicitly as demo-only in OpenAPI/docs.

---

### M2 — Doc example: `JwtVerifier::from_env()` without overlap (Medium)

**Location:** `platform/security/src/authz_middleware.rs` (ignored doc example ~line 202)

**Issue:** Example shows `JwtVerifier::from_env().map(Arc::new)` while services use **`from_env_with_overlap()`** for rotation.

**Why it matters:** Copy-paste into a new binary could **drop** `JWT_PUBLIC_KEY_PREV` support during overlap.

**Recommendation:** Change the example to `from_env_with_overlap().map(Arc::new)`.

---

### L1 — Service token: HMAC fallback when RSA missing (Low)

**Location:** `platform/security/src/service_auth.rs` (`get_service_token`)

**Issue:** If `JWT_PRIVATE_KEY_PEM` is unset, code falls back to **legacy HMAC** (“not compatible with ClaimsLayer” per comments).

**Recommendation:** In strict environments, fail closed or log at **error** once if HMAC path is used outside tests.

---

### L2 — Body limit / duration parsing: silent defaults (Low)

**Location:** `platform/platform-sdk/src/startup_helpers.rs` (`parse_body_limit`, `parse_duration_str`)

**Issue:** Malformed env strings fall back to fixed defaults (e.g. 2 MiB, 30s).

**Recommendation:** Accept for dev ergonomics; for prod, consider logging **warn** on parse failure or validating at manifest parse time.

---

### L3 — `fsfs` index path mismatch for payments (Low / tooling)

**Issue:** Search ranked `modules/payments/src/routes/payments.rs` which does not exist; actual file is `src/http/payments.rs`.

**Recommendation:** Refresh frankensearch / `fsfs` index for the repo.

---

### L4 — UBS without baseline (Low / process)

**Issue:** Per-crate UBS reports hundreds of warnings + many “critical” counts without triage; unsuitable as a clean gate until baselined or scoped (`--category`, staged files).

**Recommendation:** Add a baseline JSON per crate or CI job that only fails on **new** findings vs baseline.

---

### L5 — Module configs: empty CORS / development defaults (Low)

**Locations:** e.g. `modules/reporting/src/config.rs`, `modules/inventory/src/config.rs` (from `fsfs` + pattern knowledge)

**Issue:** Common pattern `CORS_ORIGINS` → split list, empty string meaning “allow any” in some modules (align with module docs and `MONOREPO-STANDARD`).

**Recommendation:** Periodic alignment audit with `platform-sdk` CORS fail-closed rules; ensure `ENV=production` paths cannot accidentally widen CORS.

---

## Positive observations

- **Services** consistently use `JwtVerifier::from_env_with_overlap()` in `platform-sdk` startup, control-plane, doc-mgmt, customer-portal.
- **Projection fallback** library (`platform/projections/src/fallback/`) implements staleness check, **time budget**, circuit breaker, and metrics hooks with tests in-crate and **e2e** (`projection_fallback_circuit_e2e.rs`).
- **Rate limiter** distinguishes normal vs fallback path with tighter fallback quotas (`platform/security/src/ratelimit.rs`).

---

## Verification commands (optional)

```bash
cd "/Users/james/Projects/7D-Solutions Platform"

# Compile critical crates
./scripts/cargo-slot.sh check -p projections -p payments-rs

# Compile payments tests only
./scripts/cargo-slot.sh test -p payments-rs --no-run

# fsfs discovery examples
fsfs search "projection HTTP fallback circuit breaker staleness" --format json --limit 8
fsfs search "JWT public key fallback JWKS rotation overlap" --format json --limit 8

# Scoped UBS (expect non-zero until triaged)
UBS_MAX_DIR_SIZE_MB=0 ubs platform/projections --only=rust --format=json
```

---

## File index (high-signal)

| Path | Role |
|------|------|
| `platform/security/src/claims.rs` | `from_env`, `from_env_with_overlap`, `from_jwks_url` + `fallback_to_env` |
| `platform/platform-sdk/src/startup.rs` | JWKS vs static verifier wiring |
| `platform/projections/src/fallback/*` | Policy, breaker, metrics |
| `modules/payments/src/http/payments.rs` | HTTP handler using fallback types |
| `e2e-tests/tests/projection_fallback_circuit_e2e.rs` | Staleness + breaker behavior |
| `platform/security/src/service_auth.rs` | RSA mint vs HMAC fallback |
| `platform/security/src/ratelimit.rs` | Fallback path limits |
| `modules/ar/src/middleware/ratelimit.rs` | Consumer of fallback limit checks |
| `tools/tenantctl/src/main.rs`, `tools/projection-rebuild/src/main.rs` | CLI JWT verify |
| `docs/runbooks/key_rotation.md`, `scripts/proof_key_rotation.sh` | Operational rotation |

---

## Recommendations summary

1. **Harden or demote** the payments `get_payment` fallback demo (shared `Arc` breaker + metrics, or explicit “demo” labeling).
2. **Update** `authz_middleware` doc example to `from_env_with_overlap()`.
3. **Refresh** `fsfs` index after large moves (payments path).
4. **Baseline UBS** or scope by category before using `--fail-on-warning` in CI.
5. **Cross-check** module-level CORS defaults against `SECURITY-AUDIT-2026-04-14-FULL.md` CORS guidance.

---

*End of report.*
