# UBS scoped scan — Security findings (category 8 only)

**Repository:** 7D-Solutions Platform  
**Date:** 2026-04-15  
**Tooling:** `ubs-rust.sh` from `~/.local/share/ubs/modules/ubs-rust.sh`  
**Flags:** `--only=8` (Security Findings), `--no-cargo` (fast pass; no clippy/audit/cargo check), `--emit-findings-json` per crate  

**Note:** Top-level `ubs --category=security` does **not** filter Rust; only `resource-lifecycle` is recognized on the meta-runner. Use **`--only=8`** on the Rust module for this slice.

---

## Executive summary

Across **six** platform crates (Pass 1 + Pass 2 scope), category **8** collapses to **two** finding types:

1. **Critical — “Possible hardcoded secrets”** — high **hit counts**, almost entirely **heuristic false positives** (matches on identifiers like `token`, `password` env reads, `sign_token`, test helpers). **Manual review** still advised on any line that actually embeds literals.
2. **Info — “Plain HTTP URL(s) detected”** — appears where the scanner finds `http://` strings (often localhost/docs); samples were empty in emitted JSON; treat as **hygiene / prod TLS** reminders.

**No separate distinct critical rule types** appeared in the emitted JSON for these crates.

---

## Results by crate

| Crate | Files | Critical | Warning | Info | Primary finding titles |
|-------|------:|---------:|--------:|-----:|-------------------------|
| `platform/security` | 18 | 55 | 0 | 0 | Possible hardcoded secrets (samples: `authz_middleware.rs` bearer/sign_token paths) |
| `platform/platform-sdk` | 48 | 19 | 0 | 26 | Plain HTTP URLs (info); Possible hardcoded secrets (critical) — samples include tests + `get_service_token` |
| `platform/identity-auth` | 41 | 7 | 0 | 13 | Plain HTTP URLs; Possible hardcoded secrets — `handlers_password_reset`, `jwt.rs` |
| `platform/projections` | 13 | 1 | 0 | 0 | Possible hardcoded secrets — `admin.rs` token comparison |
| `platform/control-plane` | 28 | 5 | 0 | 14 | Plain HTTP URLs; Possible hardcoded secrets — tests `mint_token`, env `POSTGRES_PASSWORD` |
| `platform/tenant-registry` | 13 | 1 | 0 | 6 | Plain HTTP URLs; Possible hardcoded secrets — `seed.rs` `SEED_ADMIN_PASSWORD` |

**Aggregated counts:** 88 critical (all same title), 63 info (Plain HTTP).

---

## Representative samples (from emitted JSON)

### `platform/security`

- `authz_middleware.rs` — lines involving `raw_token` / `sign_token` (JWT flow, not embedded secrets).

### `platform/platform-sdk`

- `tests/jwks_auth.rs`, `tests/sdk_auth_vertical.rs` — test `sign_token` calls.
- `platform_services.rs` — `get_service_token()` (runtime secret from env, not hardcoded).

### `platform/identity-auth`

- `handlers_password_reset.rs` — `generate_raw_token()`.
- `auth/jwt.rs` — `token` / `keys` (issuer paths).

### `platform/projections`

- `admin.rs` — `token == expected` (likely admin gate; verify `expected` is not a literal).

### `platform/control-plane`

- Tests: `mint_token` / env password wiring.

### `platform/tenant-registry`

- `seed.rs` — `std::env::var("SEED_ADMIN_PASSWORD")` (env-driven seed, not a hardcoded password).

---

## Recommendations

1. **Triage** the “Possible hardcoded secrets” bucket with `-v` / line review or **suppress** documented false positives (`ubs:ignore` or rule tuning) so real literals stand out.
2. **Track Plain HTTP** info findings for **non-local** URLs in production code paths; localhost in tests is usually acceptable.
3. **CI:** Save `--emit-findings-json` (or `--summary-json`) per crate and **`--comparison=`** on the meta `ubs` runner to fail only on **new** hits (per prior audit advice).
4. **Full monorepo Rust UBS** remains a **noise trap** without baselines (2379 files / 755 critical / 96k+ warnings in an earlier full `--only=rust` run).

---

## Commands to reproduce

```bash
RUST_MOD="$HOME/.local/share/ubs/modules/ubs-rust.sh"
ROOT="/Users/james/Projects/7D-Solutions Platform"

bash "$RUST_MOD" --only=8 --no-cargo --emit-findings-json=/tmp/ubs-c8-security.json -q "$ROOT/platform/security"
bash "$RUST_MOD" --only=8 --no-cargo --emit-findings-json=/tmp/ubs-c8-platform-sdk.json -q "$ROOT/platform/platform-sdk"
bash "$RUST_MOD" --only=8 --no-cargo --emit-findings-json=/tmp/ubs-c8-identity-auth.json -q "$ROOT/platform/identity-auth"
bash "$RUST_MOD" --only=8 --no-cargo --emit-findings-json=/tmp/ubs-c8-projections.json -q "$ROOT/platform/projections"
bash "$RUST_MOD" --only=8 --no-cargo --emit-findings-json=/tmp/ubs-c8-control-plane.json -q "$ROOT/platform/control-plane"
bash "$RUST_MOD" --only=8 --no-cargo --emit-findings-json=/tmp/ubs-c8-tenant-registry.json -q "$ROOT/platform/tenant-registry"
```

---

## Mail delivery note

Agent Mail for this report was sent from **`BrownBadger`** — a session registered via `macro_start_session` for the Cursor task (avoids sending as another human-assigned agent such as GentleCliff). Local `whoami` may still show **AgentCursor**; MCP sender name is **BrownBadger**.

---

*End of report.*
