# Plug-and-play module audit — agent handoff

**Audience:** Claude / coding agents working the 7D Solutions Platform monorepo.  
**Bead (tracking):** bd-wys43  
**Related contract:** [docs/PLUG-AND-PLAY-CONTRACT.md](../../PLUG-AND-PLAY-CONTRACT.md)  
**Last audited:** 2026-04-02 (repo snapshot; re-verify after changes).

---

## What “done” means (product bar)

**Verticals must not write workaround code** because the platform is incomplete: no parallel HTTP stacks, no duplicate env wiring for peer services, no “wait until codegen works” adapters for **other platform modules**, no Rust `path` dependency from one `modules/*` crate to another **peer** module.

**Allowed:** Domain logic, migrations, external SaaS (PSP, QBO, webhooks), and **event-first** integration **when that is the documented contract**.

---

## Multi-pass methodology (how this audit was built)

| Pass | Focus |
|------|--------|
| **1 — Surface** | Grep/`Cargo.toml`/`module.toml`: `[platform.services]`, `platform-client-*`, `openapi_dump` bins, `reqwest`, `path = "../` between modules. |
| **2 — Deep** | Read adapters (`integrations/*`, `src/clients/*`), `main.rs` env vars, codegen/OpenAPI gap (`tools/client-codegen`). |
| **3 — Cross-check** | Compare to PLUG-AND-PLAY-CONTRACT; event-only vs HTTP peers. |
| **4 — Converge** | Checklist M1–M6 + global G1–G4 below. |

Agents should **re-run Pass 1** after their edits (commands in §7).

---

## Global gates (every module, where applicable)

| ID | Requirement | Snapshot (2026-04-02) |
|----|-------------|-------------------------|
| **G1** | `module.toml` contains `[platform.services]` for **each platform HTTP peer** the module calls | **0 / 26** modules |
| **G2** | Handlers use `ctx.platform_client::<T>()` for those peers (not ad hoc `PlatformClient::new` from `main`) | **0** usages under `modules/` |
| **G3** | If `src/bin/openapi_dump.rs` exists, `Cargo.toml` has `[[bin]] name = "openapi_dump"` | **13 / 26** wired; **13** have file but **missing bin** |
| **G4** | No `path = "../<other-module>"` between `modules/*` crates | **Fails:** `quality-inspection` → `workforce-competence` |

---

## Checklist keys (per-module work)

| ID | Action |
|----|--------|
| **M1** | Add `[platform.services]` entries for every **platform** HTTP dependency. |
| **M2** | Resolve clients via **`ModuleContext::platform_client`** (see `platform/platform-sdk/src/context.rs`). |
| **M3** | Add missing `[[bin]] openapi_dump` when `src/bin/openapi_dump.rs` exists. |
| **M4** | Remove peer **`path` dependencies**; use `clients/*` + HTTP. |
| **M5** | Delete/shrink peer HTTP **adapters** once OpenAPI + `tools/client-codegen` emit usable types (no `Result<(), _>` for real GET bodies). |
| **M6** | Replace **stubs** (e.g. AP → Payments) with real `platform-client-*` or a documented **event** API. |

Use **—** when a row does not apply (leaf service, events-only, or external-only HTTP).

---

## Per-module matrix

**Columns:** Peer `platform-client-*` crates (if any). **M1–M6:** blank = gap needed; **Y** = satisfied; **—** = not applicable. **openapi bin:** **Y** = registered in `Cargo.toml`; **N** = `openapi_dump.rs` exists but bin not declared (fix M3).

| Module | Clients (peers) | M1 | M2 | openapi bin | M4 | M5 / notes | M6 |
|--------|-----------------|----|----|-------------|----|------------|-----|
| ap | — | | | **N** | Y | — | **payments** stub |
| ar | party | | | Y | Y | `integrations/party_client.rs`, `PARTY_MASTER_URL` | — |
| bom | numbering | | | **N** | Y | `NUMBERING_URL` in main, `domain/numbering_client.rs` | — |
| consolidation | gl, ar, ap | | | **N** | Y | `GL_BASE_URL`, `integrations/{gl,ar,ap}` | — |
| customer-portal | doc-mgmt | | | **N** | Y | `http/docs.rs`, `reqwest` | — |
| fixed-assets | — | — | — | Y | Y | Events from AP (OK if contract canonical) | — |
| gl | — | — | — | Y | Y | — | — |
| integrations | — | — | — | Y | Y | External SaaS (expected) | — |
| inventory | — | — | — | Y | Y | — | — |
| maintenance | — | — | — | Y | Y | — | — |
| notifications | — | — | — | Y | Y | `reqwest` for outbound delivery (OK) | — |
| numbering | — | — | — | **N** | Y | — | — |
| party | — | — | — | Y | Y | — | — |
| payments | — | — | — | **N** | Y | `reqwest` for PSP (OK) | — |
| pdf-editor | — | — | — | **N** | Y | — | — |
| production | — | — | — | **N** | Y | — | — |
| quality-inspection | — | — | — | **N** | **fix M4** | `WORKFORCE_COMPETENCE_DATABASE_URL` | — |
| reporting | — | — | — | Y | Y | — | — |
| shipping-receiving | inventory | | | **N** | Y | `INVENTORY_URL`, `integrations/inventory_client.rs` | — |
| smoke-test | — | — | — | Y | Y | Reference vertical | — |
| subscriptions | ar | | | **N** | Y | `bill_run_service`, `PlatformClient::new`, `reqwest` | — |
| timekeeping | — (events→gl/ar) | — | — | **N** | Y | Event-first OK; add bin | — |
| treasury | — | — | — | Y | Y | Confirm deploy has service binary (crate layout unusual) | — |
| ttp | ar, tenant-registry | | | Y | Y | `src/clients/*`, env in `main` | — |
| workflow | — | — | — | Y | Y | — | — |
| workforce-competence | — | — | — | **N** | Y | Library dep from QI — **M4** | — |

---

## Suggested agent priority order

1. **M4 / quality-inspection:** Remove `workforce-competence-rs` path dep; define HTTP/OpenAPI client or alternate contract.  
2. **M3:** Add `[[bin]] openapi_dump` for the **13** modules where the file exists but bin is missing (quick win for codegen/CI).  
3. **M1 + M2 + M5 (peer HTTP):** consolidation, ttp, shipping-receiving, bom, subscriptions, customer-portal, ar — in dependency order if needed.  
4. **M6:** AP disbursement → `platform-client-payments` (or explicit event spec).  
5. **OpenAPI/codegen:** Eliminate GET/list methods that codegen maps to `()` (see `tools/client-codegen/src/spec.rs` `find_success_response`) so **M5** adapters can be deleted.

---

## Key implementation pointers

| Topic | Location |
|-------|----------|
| Manifest services | `platform/platform-sdk/src/manifest/platform_services.rs` |
| Builder wires `PlatformServices` | `platform/platform-sdk/src/builder.rs` |
| `ctx.platform_client` | `platform/platform-sdk/src/context.rs` |
| Proof test (Party) | `platform/platform-sdk/tests/vertical_proof.rs` |
| Client generator | `tools/client-codegen/` |
| Consumer wiring | `ModuleBuilder::consumer` / `tenant_consumer` in `platform/platform-sdk/src/builder.rs` |

---

## Re-verify commands (agents)

```bash
# No [platform.services] in module manifests yet — should become non-empty as work lands
rg "platform\.services" modules/**/module.toml

# platform_client usage in vertical modules
rg "platform_client" modules/**/*.rs

# openapi_dump: file without bin declaration (manual triage)
for m in modules/*/; do
  f="${m}src/bin/openapi_dump.rs"
  c="${m}Cargo.toml"
  if [[ -f "$f" ]] && ! rg -q 'name = "openapi_dump"' "$c" 2>/dev/null; then
    echo "MISSING BIN: $m"
  fi
done

# Peer path dependencies (should be empty outside allowlist)
rg 'path = "\.\./[^.]' modules/*/Cargo.toml
```

---

## Agent rules reminder

- Claim a bead before edits; commit messages must include `[bd-…]` per [AGENTS.md](../../AGENTS.md).  
- Use `./scripts/cargo-slot.sh` instead of raw `cargo`.  
- Do not fix unrelated modules in the same bead; use the priority order above or new beads.

---

## Revision history

| Date | Change |
|------|--------|
| 2026-04-02 | Initial handoff from multi-pass plug-and-play audit (bd-wys43). |
