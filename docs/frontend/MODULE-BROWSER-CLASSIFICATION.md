# Module Browser Classification

> **Who reads this:** bd-7btgv (Fail-closed CORS via module manifest). Platform Orchestrator, security reviewers.
> **What it covers:** Which of the 27 platform modules receive HTTP calls directly from a browser (SPA/mobile) vs. only from other backend services.
> **Source of truth:** Derived from codebase inspection — see Investigation Notes section for evidence chain.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-04-15 | Platform Orchestrator | Created. First classification of all 27 modules. Produced for bd-xamh3 to unblock bd-7btgv. |

---

## Classification Table

| Module | Browser-facing? | Frontends that hit it | Recommended origins |
|---|---|---|---|
| ap | NO | None confirmed | `origins = []` |
| ar | NO | None confirmed (TrashTech calls via tt-api server-side) | `origins = []` |
| bom | NO | None confirmed | `origins = []` |
| consolidation | NO | None confirmed | `origins = []` |
| customer-portal | YES | Generic customer portal SPA for verticals. No current single-org SPA found, but module is explicitly designed for direct browser access: independent RS256 JWT, login/refresh/logout endpoints, TypeScript client `@7d/customer-portal-client` ("Customer self-service"). TrashTech uses its own tt-api instead. Other verticals (Huber Power, future operators) deploy SPAs that call this module directly. | Dev: `http://localhost:5173`; Production: operator-supplied TBD (each vertical's portal domain, e.g. `https://portal.acmewaste.com`) |
| fixed-assets | NO | None confirmed | `origins = []` |
| gl | NO | None confirmed | `origins = []` |
| integrations | NO | None confirmed | `origins = []` |
| inventory | NO | None confirmed | `origins = []` |
| maintenance | NO | None confirmed | `origins = []` |
| notifications | NO | TCP UI BFF (removed in bd-cop0). In-app inbox endpoints exist (`/api/inbox`) but no current browser frontend calls them directly — the TCP UI proxied through its BFF. | `origins = []` |
| numbering | NO | None confirmed | `origins = []` |
| party | NO | None confirmed | `origins = []` |
| payments | NO | TCP UI hosted pay portal (`/pay/[session_id]`) was removed in bd-cop0. `CORS_ORIGINS` env var and production wildcard guard exist in `config.rs` (dead field — not applied in routes), indicating the module was designed for browser use. No current browser frontend calls it. | `origins = []` |
| pdf-editor | YES | External React app at `/Users/james/Projects/PDF-Creation/frontend/` (and any vertical that embeds the pdf-editor). `modules/pdf-editor/frontend/src/api/client.ts` is the browser-side API client; `INTEGRATION.md` explicitly requires `CORS_ORIGINS=http://localhost:5173` on the backend. Default URL in client.ts (`localhost:8102`) is a stale artifact — actual module port is 8121. | Dev: `http://localhost:5173`; Production: operator-supplied TBD (the embedding React app's deployed origin) |
| production | NO | None confirmed | `origins = []` |
| quality-inspection | NO | None confirmed | `origins = []` |
| reporting | NO | None confirmed | `origins = []` |
| shipping-receiving | NO | None confirmed | `origins = []` |
| smoke-test | NO | Dev tooling only — proves SDK plug-and-play. Never browser-facing. | `origins = []` |
| subscriptions | NO | None confirmed | `origins = []` |
| timekeeping | NO | None confirmed | `origins = []` |
| treasury | NO | None confirmed | `origins = []` |
| ttp | NO | None confirmed | `origins = []` |
| vertical-proof | NO | Dev tooling only — proves a vertical can call 5 platform modules via SDK. Never browser-facing. | `origins = []` |
| workflow | NO | None confirmed | `origins = []` |
| workforce-competence | NO | None confirmed | `origins = []` |

**Summary:** 2 of 27 modules are browser-facing. 25 are internal-only.

---

## Investigation Notes

### What was searched

Investigation followed `rules/search.md` — fsfs semantic search first, raw grep only to confirm.

**fsfs queries run:**
- `frontend fetch call to module API`
- `Next.js API route calls backend module`
- `fetch axios http client BASE_URL module`
- `customer portal web UI`
- `mobile app consumes platform API`
- `browser client consumes notifications service`
- Per-module probes: `frontend browser client calls $m module` for all 27 modules

**Source files read:**
- `apps/ranchorbit-pilot/` — all source files (mock data only; login calls identity-auth, not platform modules)
- `apps/sandbox/` — stub app, no API calls
- `apps/tenant-control-plane-ui/` — source removed in bd-cop0; reconstructed from git history
- `modules/pdf-editor/frontend/src/api/client.ts` — browser API client confirmed
- `modules/pdf-editor/frontend/INTEGRATION.md` — explicit CORS requirement confirmed
- `modules/customer-portal/src/http/mod.rs` — login/refresh/logout routes confirmed
- `modules/customer-portal/src/main.rs` — independent RS256 JWT confirmed
- `modules/customer-portal/module.toml` — "Customer-facing portal" description confirmed
- `modules/payments/src/config.rs` — `CORS_ORIGINS` env var (dead field, not applied in routes)
- `clients/README.md` — TypeScript SDK for vertical frontend developers
- `/Users/james/Projects/TrashTech/apps/customer-portal/vite.config.ts` — proxies to tt-api (port 8105), not platform customer-portal (port 8111)
- `/Users/james/Projects/TrashTech/apps/trashtech-pro/lib/api/client.ts` — BFF pattern, no direct module port refs
- `/Users/james/Projects/Fireproof/frontend/src/infrastructure/utils/env.ts` — calls own backend via relative URL, no platform module ports

### TCP UI (removed — bd-cop0)

The TCP UI (`apps/tenant-control-plane-ui/`) was removed from the platform repo in commit `2ff80ef4`. It used a **BFF pattern** — `lib/constants.ts` comment: "Backend service base URLs (used ONLY in BFF routes — never in browser code)". The browser called the TCP UI's Next.js server; the Next.js server called platform modules. No CORS was ever needed for the TCP UI's server-side calls. Modules that the TCP UI's BFF proxied: `notifications`, `ar`, `ttp`, `payments`, `identity-auth`, `tenant-registry`, `audit`.

### TypeScript client SDK

`clients/README.md` describes TypeScript clients for all 27 modules as being "for vertical developers building frontends against the 7D platform." The existence of a TypeScript client does **not** make a module browser-facing — the clients can equally be used in server-side Node.js or Next.js BFF routes. Only confirmed current browser usage (or explicit design for browser-direct access, as with customer-portal) qualifies a module as YES.

### TrashTech customer portal

The TrashTech customer portal SPA (`apps/customer-portal/` in the TrashTech project) calls **tt-api** at port 8105 — TrashTech's own Rust backend — not the platform's customer-portal module (port 8111). TrashTech built their own customer portal backend. The platform's customer-portal module is for verticals that want to consume a generic customer portal rather than building their own.

### Payments CORS stub

`modules/payments/src/config.rs` parses `CORS_ORIGINS` env var with a production wildcard guard, indicating the module was designed with browser intent. However, `cors_origins` is not included in `AppState` and no `CorsLayer` is applied in `main.rs` using this value — it is dead code as of today. The only known browser consumer (TCP UI hosted pay portal) was removed in bd-cop0. Classified as NO until a replacement browser consumer is confirmed.

---

## Open Questions

| # | Question | Status |
|---|----------|--------|
| 1 | Will a replacement hosted pay portal (calling payments directly from the browser) be built? If yes, payments needs to move to YES. | Unresolved — flag for bd-7btgv author |
| 2 | Does Huber Power or any other signed vertical currently deploy a SPA that calls platform modules (other than customer-portal) directly from the browser? | Unresolved — no evidence found in this repo |
