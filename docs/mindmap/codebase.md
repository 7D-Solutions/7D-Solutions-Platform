# 7D Solutions Platform — Codebase Mindmap

**7D Solutions Platform is a modular Rust backend for building vertical business applications (e.g., Fireproof ERP for aerospace/defense).** Each module is an independent Axum microservice with its own PostgreSQL database, communicating via NATS JetStream events. Verticals compose modules and build their own frontends in separate repos.

---

## Platform Core (`platform/`)
Shared runtime infrastructure that all business modules depend on.

### Identity & Auth (`platform/identity-auth`)
- JWT-based authentication with Ed25519 key pairs
- RBAC, password reset, account lockout, Argon2 hashing
- Runs 2 instances behind an Nginx load balancer (`auth-lb`)
- Depends on: event-bus, health
- Port: 8080 (via LB)

### Event Bus (`platform/event-bus`)
- NATS JetStream wrapper with outbox relay and DLQ
- Foundation dependency for nearly every module
- Guard → Mutation → Outbox atomicity pattern
- EventEnvelope constitutional metadata

### Event Consumer (`platform/event-consumer`)
- Consumes events from NATS JetStream
- Depends on: event-bus

### Control Plane (`platform/control-plane`)
- Tenant provisioning and platform billing orchestration
- Depends on: tenant-registry, health
- Port: 8091

### Tenant Registry (`platform/tenant-registry`)
- Multi-tenant registry, lifecycle, plan management
- Used by control-plane and tenantctl

### Security (`platform/security`)
- AuthZ middleware, rate limiting, webhook verification
- Depends on: event-bus
- Consumed by nearly every module

### Audit (`platform/audit`)
- Append-only audit trail with field-level diffs
- Actor propagation across services

### Projections (`platform/projections`)
- Cursor-based event projection rebuild
- Blue-green swap for zero-downtime rebuilds
- Used by GL, AR, inventory, and other event-sourced modules

### Health (`platform/health`)
- Readiness/liveness probe contract (`/api/health`, `/api/ready`)
- Universal dependency for all services

### Tax Core (`platform/tax-core`)
- Tax jurisdiction resolution
- Local and zero-rate providers
- Used by: AR, AP

### Platform Contracts (`platform/platform-contracts`)
- Shared contract types and interfaces
- Depends on: event-bus
- Used by: doc-mgmt, customer-portal, numbering, shipping-receiving, workflow

### Document Management (`platform/doc-mgmt`)
- Document storage and lifecycle
- Depends on: event-bus, platform-contracts, security, health
- Port: 8095

---

## Business Modules (`modules/`)
Domain-specific services, each with own DB, API, and event integration.

### Financial Core
- **AR** (`modules/ar`) — Accounts receivable: invoices, aging, dunning, credit notes, write-offs, Tilled integration | Port 8086
- **AP** (`modules/ap`) — Accounts payable: bills, POs, payment runs, vendor management | Port 8093
- **GL** (`modules/gl`) — General ledger: journals, trial balance, rev-rec, FX, accruals | Port 8090
- **Payments** (`modules/payments`) — Payment processing, reconciliation, retry logic | Port 8088
- **Treasury** (`modules/treasury`) — Bank accounts, bank reconciliation, cash position, forecasting | Port 8094

### Financial Extended
- **Subscriptions** (`modules/subscriptions`) — Recurring billing, lifecycle state machine; calls AR | Port 8087
- **Fixed Assets** (`modules/fixed-assets`) — Asset register, depreciation, disposals | Port 8104
- **Consolidation** (`modules/consolidation`) — Multi-entity financial consolidation, eliminations | Port 8105
- **Timekeeping** (`modules/timekeeping`) — Time entries, approvals, billing, GL labor cost | Port 8097
- **Reporting** (`modules/reporting`) — Financial statements, aging reports, KPIs, forecasting | Port 8096

### Manufacturing & Supply Chain
- **Inventory** (`modules/inventory`) — Stock tracking, FIFO costing, reservations, cycle counts, lot/serial tracking | Port 8092
- **BOM** (`modules/bom`) — Bill of materials, ECO management | Port 8107
- **Production** (`modules/production`) — Work orders, routings, time tracking | Port 8108
- **Quality Inspection** (`modules/quality-inspection`) — Inspection plans, results; cross-references workforce-competence | Port 8106
- **Shipping & Receiving** (`modules/shipping-receiving`) — Inbound/outbound shipments, inventory integration | Port 8103
- **Maintenance** (`modules/maintenance`) — Work orders, preventive maintenance plans, meters | Port 8101

### Cross-Cutting Services
- **Notifications** (`modules/notifications`) — Event-driven notifications, scheduled dispatch | Port 8089
- **Party** (`modules/party`) — Party master: customers, vendors, contacts, addresses | Port 8098
- **Integrations** (`modules/integrations`) — External connectors, webhook routing, external refs | Port 8099
- **PDF Editor** (`modules/pdf-editor`) — PDF template forms, annotations, submission validation | Port 8102
- **Numbering** (`modules/numbering`) — Configurable sequence generation for documents | Port 8120
- **Workflow** (`modules/workflow`) — Approval workflows, state machines | Port 8110
- **Customer Portal** (`modules/customer-portal`) — Tenant-facing self-service; calls doc-mgmt | Port 8111
- **Workforce Competence** (`modules/workforce-competence`) — Employee skills, certifications | Port 8121

### Platform Billing
- **TTP** (`modules/ttp`) — Tenant technology platform: metering, billing, service agreements; calls control-plane | Port 8100

---

## Module Dependency Graph

### Shared Platform Dependencies (nearly all modules)
- `security` — AuthZ middleware
- `health` — Health check endpoints
- `event-bus` — NATS JetStream integration

### Additional Platform Dependencies
- Modules using `projections`: AR, GL, AP, inventory, payments, subscriptions, notifications, party, integrations, timekeeping, treasury, consolidation, fixed-assets, reporting, ttp
- Modules using `tax-core`: AR, AP
- Modules using `platform-contracts`: customer-portal, numbering, shipping-receiving, workflow, doc-mgmt

### Cross-Module Dependencies
- `quality-inspection` → `workforce-competence` (competency verification)
- `subscriptions` → `ar` (invoice generation)
- `ttp` → `control-plane` (tenant registry lookup)
- `customer-portal` → `doc-mgmt` (document access)
- `control-plane` → `tenant-registry`

---

## Infrastructure (`infra/`)

### NATS (`infra/nats`)
- NATS 2.10-alpine with JetStream enabled
- Auth token-based access
- Port: 4222 (client), 8222 (monitoring)

### PostgreSQL (`infra/postgres`)
- Postgres 16-alpine, one instance per module (30+ databases)
- TLS-enabled connections (dev self-signed, production CA-signed)
- Ports: 5433–5464 (one per service)
- Custom entrypoint for TLS setup

### Monitoring (`infra/monitoring`)
- **Prometheus** — Scrapes `/metrics` from all services, 30-day retention | Port 9091
- **Alertmanager** — Alert routing and notification | Port 9094
- **Grafana** — Dashboards for billing, operations | Port 3002
- Alert rules in `infra/monitoring/alerts/`
- Dashboards in `infra/monitoring/grafana/dashboards/`

### Nginx (`nginx/`)
- `auth-nginx.conf` — Load balancer for auth-1/auth-2
- `gateway.conf` — API gateway with edge rate limiting | Port 8000

---

## Contracts (`contracts/`)
API contract definitions (OpenAPI, event schemas) for inter-module communication.

- `contracts/api` — Shared API contracts
- `contracts/events` — Event envelope schemas with examples
- `contracts/ar`, `contracts/gl`, `contracts/payments`, etc. — Per-module contracts
- `contracts/auth`, `contracts/control-plane`, `contracts/tenant-registry` — Platform contracts

---

## Tools (`tools/`)

### CLI & Operations
- **tenantctl** — Tenant management CLI; depends on tenant-registry, audit, security
- **demo-seed** — Seed demo data for development
- **compliance-export** — Export compliance artifacts
- **projection-rebuild** — Rebuild event projections; depends on projections, security

### Testing & Validation
- **contract-tests** — Event schema and OpenAPI validation
- **simulation** — Load simulation using AR, payments, subscriptions, GL
- **stabilization-gate** — Automated stabilization checks; depends on event-bus, AR

### Performance
- `tools/perf` — Performance testing configs and libraries
- `tools/ci` — CI helper scripts

---

## Testing

### E2E Tests (`e2e-tests/`)
- 140+ end-to-end test files covering cross-module workflows
- Categories: smoke tests, stress tests, lifecycle tests, security tests
- Depends on nearly every module and platform crate
- Tests run against real databases — no mocks or stubs

### Unit/Integration Tests
- Each module has `tests/` directory for module-level tests
- Run via `./scripts/cargo-slot.sh test -p <module>`

### Stress Tests (`e2e-tests/tests/stress/`)
- DB pool exhaustion, rate limit under load, event bus flood
- Double-spend prevention, oversell protection, large payload rejection
- Multi-tenant isolation hammer

---

## Scripts (`scripts/`)

### Development
- `cargo-slot.sh` — 4-slot concurrent cargo build system
- `dev-watch.sh` — Docker Compose Watch for auto-rebuild
- `dev-native.sh` — Run service natively with cargo-watch
- `dev-cross.sh` — Cross-compile macOS → Linux for Docker
- `seed-dev.sh` / `seed-platform-admin.sh` — Development data seeding

### CI/CD (`scripts/ci/`)
- `proof-runbook-ci.sh` — CI proof runbook execution
- `check-event-conventions.sh` — Event naming convention checks
- `check-migration-versioning.sh` — Migration version validation
- `check-openapi-breaking-changes.sh` — OpenAPI breaking change detection
- `publish-platform-crates.sh` — Platform crate registry publishing

### Production (`scripts/production/`)
- `deploy_stack.sh` / `rollback_stack.sh` — Image-pinned deployment and rollback
- `backup_all_dbs.sh` / `backup_prune.sh` / `backup_ship.sh` — Backup pipeline
- `restore_drill.sh` / `health_audit.sh` — DR restore and verification
- `secrets_init.sh` / `secrets_rotate.sh` / `secrets_check.sh` — Secret lifecycle
- `provision_vps.sh` / `ssh_bootstrap.sh` — VPS provisioning
- `smoke.sh` / `isolation_check.sh` — Post-deploy validation
- `log_bundle.sh` — Diagnostic log capture

### Staging (`scripts/staging/`)
- `build_images.sh` / `push_images.sh` — Image build and registry push
- `deploy_stack.sh` / `rollback_stack.sh` — Staging deployment
- `smoke.sh` / `isolation_check.sh` — Staging validation
- `payment_loop.sh` — E2E payment verification

### Versioning (`scripts/versioning/`)
- `detect_version_intent.sh` — Detect version-intent changes in commits
- `lint_revisions.sh` — Validate REVISIONS.md for proven modules
- `promote_module.sh` — Module promotion through Gates 1, 2, 3
- `pre-commit-version-check.sh` — Gate 1 pre-commit hook

### DR Drills (`scripts/drills/`)
- `dlq_replay_drill.sh` — Dead letter queue replay verification
- `jetstream_restore_drill.sh` — JetStream backup/restore drill

### Agent Coordination
- `agent-mail-helper.sh` — Inter-agent mail system
- `reserve-files.sh` — Multi-agent file reservation
- `plan-to-agents.sh` — Spawn agents from bead queue
- `session-start-hook.sh` / `session-stop-hook.sh` — Agent lifecycle hooks
- `fix-tracking-files.sh` — Stale tracking file cleanup

---

## Deployment

### Docker Compose Stacks
- `docker-compose.data.yml` — Databases + NATS (start first)
- `docker-compose.services.yml` — All application services
- `docker-compose.monitoring.yml` — Prometheus + Alertmanager + Grafana
- `docker-compose.cross.yml` — Cross-compiled binary overlay
- `docker-compose.production.yml` — Immutable image tag pins
- `docker-compose.production-data.yml` — Production secrets + TLS overlay

### Deploy Configs
- `deploy/production/` — Production manifests
- `deploy/staging/` — Staging manifests
- `scripts/release/manifest_to_env.sh` — Manifest → env conversion

---

## Documentation (`docs/`)

### Architecture (`docs/architecture/`)
- Module standards, layering rules, CI guardrails
- ADRs in `docs/architecture/decisions/`

### Operations (`docs/ops/`, `docs/runbooks/`)
- Operational runbooks for DR, deployment, backup

### Governance (`docs/governance/`)
- Domain ownership, mutation classes, retention policies

### Plans (`docs/plans/`)
- Manufacturing roadmap, phase plans, `.drawio` flowcharts

### Other
- `docs/VERSIONING.md` — SemVer module versioning standard (three gates)
- `docs/contracts/` — Contract documentation
- `docs/consumer-guide/` — Module consumer integration guide
- `docs/hardening/` — Security hardening notes
- `docs/releases/` — Release notes
- `docs/frontend/` — Frontend standards (for vertical repos)

---

## Agent Coordination System

### Beads (`.beads/`)
- Work tracking system — all changes require an active bead
- Pre-edit hooks enforce bead requirement
- Bead lifecycle: draft → open → in_progress → closed

### Flywheel (`.flywheel/`)
- Auto-retro system (every 5 bead closes)
- Retro artifacts in `.flywheel/retro/`
- Counter state in `.flywheel/retro-counter.json`

### Agent Mail
- MCP-based inter-agent messaging
- Broadcast to swarm groups (`@all`, `@active`, `@coordinators`)
- Orchestrator: BrightHill | Agents: CopperRiver, PurpleCliff, MaroonHarbor, SageDesert, DarkOwl

### Search Tools (`.frankensearch/`)
- `fsfs` — Semantic + keyword codebase search
- `cass` — Prior session solution search
- Lexical index, vector index, and explain cache

---

## Build System

### Cargo Workspace
- 33 workspace members (11 platform + 25 modules + 7 tools)
- Rust 2021 edition, resolver v2
- Aligned deps: Axum 0.8, sqlx 0.8, Tower 0.5
- Workspace lints: `unsafe_code = "deny"`

### Build Slots
- 4 independent target directories (`target-slot-1` through `target-slot-4`)
- `scripts/cargo-slot.sh` routes builds to available slots
- Enables parallel compilation by multiple agents

### CI (`.github/workflows/`)
- GitHub Actions workflows
- File size lint (500 LOC max)
- Version bump enforcement for proven modules
- Event convention checks, migration versioning, OpenAPI diff

---

## Proofs (`proofs/`)
- Timestamped proof runbook outputs (20+ runs)
- Each contains: cross-phase validation, NATS health, service checks, test results
- Final proof: `20260304T145459Z` — 33/33 crates green, platform contracts pass
