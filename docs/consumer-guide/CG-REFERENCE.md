# Consumer Guide — Reference: Env Vars, Dependencies, Local Dev & Tests

> **Who reads this:** Agents building vertical apps on the 7D Platform.
> **What it covers:** Environment variables, Cargo.toml path dependencies, local development setup, reference E2E test files, and the full source file index.
> **Parent:** [PLATFORM-CONSUMER-GUIDE.md](./PLATFORM-CONSUMER-GUIDE.md)

## Contents

1. [Environment Variables](#environment-variables) — all required env vars, JWT_PUBLIC_KEY note, test RSA key generation
2. [Cargo.toml Path Dependencies](#cargotoml-path-dependencies) — event-bus, security, and common dependency versions
3. [Local Development](#local-development) — docker compose, verifying services, running your vertical app alongside platform
4. [Reference E2E Tests](#reference-e2e-tests) — copy-from index by test file + what it demonstrates
5. [Source File Index](#source-file-index) — find the source file for any platform topic

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-CONSUMER-GUIDE.md. Environment variables, Cargo.toml deps, local dev instructions, reference E2E test index, source file index. |
| 2.0 | 2026-03-04 | MaroonHarbor | Added complete service port table (21 services), extension module base URLs, updated NATS_URL with auth token, added source file index entries for maintenance/notifications/SoD. |

---

## Environment Variables

Required env vars for any module that uses platform crates. Set these in your `docker-compose.yml` and test scripts.

```bash
# Your module's Postgres connection
DATABASE_URL=postgres://postgres:postgres@your-app-postgres:5432/your_app_db

# NATS event bus (JetStream enabled)
# Note: when NATS auth is enabled, use token format:
# NATS_URL=nats://platform:${NATS_AUTH_TOKEN}@7d-nats:4222
NATS_URL=nats://7d-nats:4222

# JWT public key for RS256 verification (from identity-auth)
# In Docker: read from volume or env. In tests: set to test key.
JWT_PUBLIC_KEY="-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----"

# Your service's HTTP bind address
LISTEN_ADDR=0.0.0.0
LISTEN_PORT=8101   # pick an unused port

# Your tenant identity (set by orchestrator during provisioning)
TENANT_ID=550e8400-e29b-41d4-a716-446655440000
APP_ID=<your-app-id>

# Platform module base URLs (use container names in Docker, localhost in dev)
PARTY_BASE_URL=http://7d-party:8098
AR_BASE_URL=http://7d-ar:8086
AUTH_BASE_URL=http://7d-auth-lb:8080
MAINTENANCE_BASE_URL=http://7d-maintenance:8101
NOTIFICATIONS_BASE_URL=http://7d-notifications:8089
INTEGRATIONS_BASE_URL=http://7d-integrations:8099
GL_BASE_URL=http://7d-gl:8090
PAYMENTS_BASE_URL=http://7d-payments:8088
SUBSCRIPTIONS_BASE_URL=http://7d-subscriptions:8087
INVENTORY_BASE_URL=http://7d-inventory:8092

# Log level
RUST_LOG=info
```

**`JwtVerifier::from_env_with_overlap()` reads `JWT_PUBLIC_KEY` (and `JWT_PUBLIC_KEY_PREV` during key rotation).** If `JWT_PUBLIC_KEY` is absent, it returns `None` — **this does NOT bypass auth**. When the verifier is `None`, `optional_claims_mw` extracts no claims, and `RequirePermissionsLayer` returns `401 Unauthorized` on every mutation route. You cannot call mutation endpoints without a valid JWT, even locally.

**Always set `JWT_PUBLIC_KEY` — including in local development.** Use a test RSA key pair:
```bash
# Generate a test RSA key pair (one-time local setup)
openssl genrsa -out /tmp/jwt-test.pem 2048
openssl rsa -in /tmp/jwt-test.pem -pubout -out /tmp/jwt-test-pub.pem

# Set env var (inline PEM, newlines as \n)
export JWT_PUBLIC_KEY="$(cat /tmp/jwt-test-pub.pem)"
```

In E2E tests against real platform, read the public key from the identity-auth JWKS endpoint (`GET /.well-known/jwks.json`) or from the platform ops team.

### Complete Service Port Table

All platform services and their ports. In Docker Compose, use the container name. On localhost, use the mapped port.

| Container Name | Port | Module |
|---------------|------|--------|
| `7d-auth-lb` | 8080 | Identity & Auth (nginx load balancer) |
| `7d-gateway` | 8000 | API Gateway (nginx) |
| `7d-control-plane` | 8091 | Tenant Registry / Control Plane |
| `7d-ar` | 8086 | Accounts Receivable |
| `7d-subscriptions` | 8087 | Subscriptions |
| `7d-payments` | 8088 | Payments |
| `7d-notifications` | 8089 | Notifications |
| `7d-gl` | 8090 | General Ledger |
| `7d-inventory` | 8092 | Inventory |
| `7d-ap` | 8093 | Accounts Payable |
| `7d-treasury` | 8094 | Treasury |
| `7d-timekeeping` | 8097 | Timekeeping |
| `7d-party` | 8098 | Party Master |
| `7d-integrations` | 8099 | Integrations |
| `7d-ttp` | 8100 | Tenant Provisioning |
| `7d-maintenance` | 8101 | Maintenance |
| `7d-pdf-editor` | 8102 | PDF Editor / Document Mgmt |
| `7d-shipping-receiving` | 8103 | Shipping & Receiving |
| `7d-fixed-assets` | 8104 | Fixed Assets |
| `7d-consolidation` | 8105 | Consolidation |

**Auth instances:** `7d-auth-1` and `7d-auth-2` run behind `7d-auth-lb`. Connect to the load balancer, not directly.

---

## Cargo.toml Path Dependencies

When your vertical app needs platform crates, add these to your `Cargo.toml`:

```toml
[dependencies]
# Platform crates (path dependencies — adjust relative path based on your module location)
# If your module is at modules/your-app/:
event-bus = { path = "../../platform/event-bus" }
security = { path = "../../platform/security" }

# Common dependencies matching platform versions
axum = "0.8"
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres", "uuid", "chrono", "json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["serde", "v4"] }
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
tower = "0.5"
http = "1"
```

Source: `platform/security/Cargo.toml` confirms `event-bus = { path = "../event-bus" }` pattern.

---

## Local Development

To run platform services locally for integration tests:

```bash
# From the 7D Solutions Platform repo root
docker compose up -d

# Wait for services to be healthy
docker compose ps   # all should show "healthy" or "running"

# Verify key services are ready
curl http://localhost:8080/api/ready  # identity-auth
curl http://localhost:8086/api/ready  # AR
curl http://localhost:8098/api/ready  # Party Master

# Run a specific E2E test
AUDIT_DATABASE_URL=postgres://postgres:postgres@localhost:5432/audit_db \
PROJECTIONS_DATABASE_URL=postgres://postgres:postgres@localhost:5432/projections_db \
TENANT_REGISTRY_DATABASE_URL=postgres://postgres:postgres@localhost:5432/tenant_registry_db \
./scripts/cargo-slot.sh test -p e2e-tests -- party_master_e2e --nocapture
```

Platform services bind to localhost in development:
- identity-auth: localhost:8080
- AR: localhost:8086
- Party Master: localhost:8098

In Docker Compose networking (service-to-service), use container names: `7d-auth-lb`, `7d-ar`, `7d-party`.

### Running Your Vertical App Locally

Your vertical app (TrashTech) runs alongside platform services — not inside the platform compose. Run it separately with `cargo run` pointing at platform services on localhost:

```bash
# 1. Start platform services (from 7D Platform repo)
cd /path/to/7D-Solutions-Platform && docker compose up -d

# 2. In your TrashTech repo, run your service with env vars pointing to platform
DATABASE_URL=postgres://postgres:postgres@localhost:5433/your_app_db \
NATS_URL=nats://localhost:4222 \
JWT_PUBLIC_KEY="$(cat /tmp/jwt-test-pub.pem)" \
AUTH_BASE_URL=http://localhost:8080 \
AR_BASE_URL=http://localhost:8086 \
PARTY_BASE_URL=http://localhost:8098 \
./scripts/cargo-slot.sh run -p tt-server

# 3. Run your E2E tests (same env vars, separate terminal)
DATABASE_URL=postgres://postgres:postgres@localhost:5433/your_app_db \
./scripts/cargo-slot.sh test -p e2e-tests -- your_app --nocapture
```

Your service uses a **separate Postgres instance** (different port or database name) from the platform services.

---

## Reference E2E Tests

Copy patterns from these files in `e2e-tests/tests/`:

| Test file | What it shows |
|-----------|---------------|
| `party_master_e2e.rs` | Full Party CRUD: create company, get, update, deactivate, search |
| `party_ar_link.rs` | Create party → create AR customer with party_id → verify |
| `ap_vendor_party_link_e2e.rs` | Create party → create AP vendor with party_id |
| `cross_module_invoice_payment_e2e.rs` | Invoice → payment → status update full cycle |
| `cross_module_subscription_invoice_e2e.rs` | Subscription → auto-invoice generation |
| `integrations_integration.rs` | Inbound webhook → external ref mapping |
| `subscriptions_lifecycle.rs` | Subscription state machine transitions |
| `provisioning_full_lifecycle_e2e.rs` | Tenant provisioning end-to-end |
| `rbac_enforcement.rs` | RBAC permission enforcement patterns |
| `treasury_forecast_e2e.rs` | Cash forecast from AR/AP data |

---

## Source File Index

For re-verification or deeper reading:

| Topic | Source file |
|-------|-------------|
| Auth endpoints | `platform/identity-auth/src/routes/auth.rs` |
| JWT claims structure | `platform/identity-auth/src/auth/jwt.rs` → `AccessClaims` |
| JWKS endpoint | `platform/identity-auth/src/main.rs` (mounted at `/.well-known/jwks.json`) |
| JWT verification | `platform/security/src/claims.rs` → `JwtVerifier`, `VerifiedClaims` |
| Auth middleware | `platform/security/src/authz_middleware.rs` → `ClaimsLayer`, `RequirePermissionsLayer` |
| Permission constants | `platform/security/src/permissions.rs` |
| Security crate Cargo.toml | `platform/security/Cargo.toml` |
| EventEnvelope (canonical) | `platform/event-bus/src/envelope/mod.rs` |
| EventEnvelope builder | `platform/event-bus/src/envelope/builder.rs` |
| EventEnvelope validation | `platform/event-bus/src/envelope/validation.rs` |
| MerchantContext enum | `platform/event-bus/src/envelope/mod.rs` |
| TracingContext | `platform/event-bus/src/envelope/tracing_context.rs` |
| Party Master endpoints | `modules/party/src/http/party.rs` |
| Party Master router | `modules/party/src/http/mod.rs` |
| Party Master models | `modules/party/src/domain/party/models.rs` |
| AR customer model | `modules/ar/src/models/customer.rs` |
| AR invoice model | `modules/ar/src/models/invoice.rs` |
| AR router (all routes) | `modules/ar/src/routes/mod.rs` |
| AR customer endpoints | `modules/ar/src/routes/customers.rs` |
| AR envelope helper | `modules/ar/src/events/envelope.rs` |
| AR outbox functions | `modules/ar/src/events/outbox.rs` |
| AR publisher | `modules/ar/src/events/publisher.rs` |
| AR outbox migration | `modules/ar/db/migrations/20260211000001_create_events_outbox.sql` |
| AR outbox metadata migration | `modules/ar/db/migrations/20260216000001_add_envelope_metadata_to_outbox.sql` |
| NATS subjects (AR) | `modules/ar/src/events/publisher.rs` line 51-57 |
| NATS subjects (auth) | `platform/identity-auth/src/auth/handlers.rs` |
| SoD policy CRUD | `platform/identity-auth/src/db/sod.rs` |
| SoD HTTP handlers | `platform/identity-auth/src/auth/handlers.rs` (lines 201-340) |
| SoD event subjects | `auth.sod.policy.upserted`, `auth.sod.policy.deleted`, `auth.sod.decision.recorded` |
| Party contacts endpoints | `modules/party/src/http/contacts.rs` |
| Party contact model | `modules/party/src/domain/contact.rs` |
| Party contact events | `modules/party/src/events/contact.rs` |
| Party addresses endpoints | `modules/party/src/http/addresses.rs` |
| Party router (all routes) | `modules/party/src/http/mod.rs` |
| Maintenance router (all routes) | `modules/maintenance/src/main.rs` (lines 109-248) |
| Maintenance asset model | `modules/maintenance/src/domain/assets.rs` |
| Maintenance event subjects | `modules/maintenance/src/events/subjects.rs` |
| Maintenance work orders | `modules/maintenance/src/http/work_orders.rs` |
| Maintenance calibration | `modules/maintenance/src/http/calibration_events.rs` |
| Maintenance downtime | `modules/maintenance/src/http/downtime.rs` |
| Notifications router (all routes) | `modules/notifications/src/main.rs` (lines 156-208) |
| Notifications templates | `modules/notifications/src/http/templates.rs` |
| Notifications sends | `modules/notifications/src/http/sends.rs` |
| Notifications inbox | `modules/notifications/src/http/inbox.rs` |
| Notifications DLQ | `modules/notifications/src/http/dlq.rs` |
| Notifications template model | `modules/notifications/src/template_store/models.rs` |
| Notifications send model | `modules/notifications/src/sends/models.rs` |
| Tenant status endpoint | `platform/tenant-registry/src/routes.rs` |
| Tenant lifecycle states | `platform/tenant-registry/src/lifecycle.rs` |

---

> See `docs/PLATFORM-CONSUMER-GUIDE.md` for the master index and critical concepts.
