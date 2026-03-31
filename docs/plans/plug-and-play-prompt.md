# 7D Solutions Platform — Module-by-Module Plug-and-Play Productization

## Context

I have a modular ERP backend platform with 30+ Rust/Axum microservices behind an Nginx gateway. Each module (AR, GL, AP, Inventory, Party, BOM, Shipping, Notifications, etc.) runs as its own Docker container with its own Postgres database and connects to a shared NATS JetStream event bus.

The platform works. 33 crates compile, tests pass, first paying customer signed (aerospace/defense).

The problem: every time a new frontend project wants to consume these modules, it takes 2-3 weeks and ~17,000 lines of boilerplate to wire everything together. Three consumer projects (Huber Power, TrashTech, Fireproof-ERP) each independently reverse-engineered the same undocumented headers, built their own API clients, discovered the same bugs, and worked around the same inconsistencies. There are 5 different pagination response formats across modules. No module serves an OpenAPI spec. No module validates its own env vars with clear errors. Required headers (X-Tenant-Id, X-Permissions, x-correlation-id) are undocumented.

## Decision

After review by Grok, ChatGPT, and two implementation agents (CopperRiver, SageDesert), we've decided:

**Go module by module.** Pick one module, make it truly plug-and-play, prove it with a real consumer, repeat. Extract shared code only after 3 modules when patterns are proven. No big upfront shared crate.

**Modules lead, projects adapt.** The modules define the standard. Consumer projects adapt to what modules offer.

## What "plug-and-play" means for each module

1. **OpenAPI spec served** — utoipa annotations, /api/openapi.json endpoint, all routes documented with required headers and response schemas
2. **Env validation on startup** — clear error messages listing exactly what's missing or wrong, fail-fast
3. **Auto-run migrations** — module self-bootstraps its database
4. **Standard response envelopes** — ONE pagination format, ONE error format across all endpoints
5. **Standard health/readiness/version endpoints** — reuse platform health crate
6. **Standard required headers documented in spec** — X-Tenant-Id, X-Permissions, x-correlation-id
7. **Self-bootstrapping** — a project can add the module without reading source code

## Module order

1. **Inventory** (first) — 0 event consumers, straightforward routes, consumed by Shipping-Receiving/Fireproof-ERP/TrashTech
2. **Party** (second) — broadly reusable, simple, hot path for every transaction
3. **BOM or AR** (third) — more integration weight, proves the pattern scales
4. After module 3: extract shared helpers into platform crate
5. Then build @7d/api (generated TS clients from module OpenAPI specs)

## Bead chain per module (sub-steps)

Each module gets 3-5 beads:

**Bead A — Contract surface:** Standard health/readiness/version endpoints. Define and enforce ONE standard error envelope and ONE standard pagination envelope. Document required headers. Add env validation with clear startup failure messages.

**Bead B — OpenAPI:** Add utoipa dependency. Annotate all route handlers. Expose /api/openapi.json with real routes, header requirements, and envelope schemas. Not placeholders.

**Bead C — Bootstrap behavior:** Auto-run migrations on startup. Verify empty-DB-to-ready path works. Document all required and optional env vars with defaults.

**Bead D — Consumer proof:** Generate a typed TypeScript client from the OpenAPI spec. Prove a consumer project (start with Shipping-Receiving for Inventory) can call it with dramatically less boilerplate. Smoke test against real dev stack.

**Bead E — Extraction review:** After each module, identify repeated code/patterns. After module 3, extract into shared platform helpers.

## Current state of Inventory module

- Location: modules/inventory/src/
- Port: 8092
- 0 event consumers (no NATS bus usage)
- Has DB pool + migrations
- Standard middleware stack (body limit, tracing, timeout, rate limit, JWT, CORS)
- No utoipa/OpenAPI today
- No env validation today (just panics on missing config)
- Uses the common 9-step startup pattern all modules share

## Technical constraints

- All Rust compilation happens natively, not in Docker (`./scripts/cargo-slot.sh` for builds)
- Tests hit real services, no mocks
- Proven modules (v1.0+) require version bumps + REVISIONS.md entries
- Files must stay under 500 LOC
- All work tracked with beads (br CLI)

## What I need from you

Review this plan and help me think through:

1. What should the ONE standard pagination envelope look like? We currently have 5 different formats across modules.
2. What should the ONE standard error envelope look like?
3. For utoipa integration on an Axum service — what's the cleanest approach? Any gotchas with our middleware stack (security crate provides JWT/rate-limiting/CORS)?
4. For env validation — should we use a crate (like `figment` or `config-rs`) or keep it simple with a custom validator?
5. What does the standard response envelope migration look like for Inventory specifically — how many handlers need to change?
6. Any risks or blind spots in this module-by-module approach?

The goal: a new project should be able to add Inventory as a dependency and have it just work — self-describing, self-validating, standard contract, zero reverse-engineering.
