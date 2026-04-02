# Provisioning Orchestrator Design

## Problem

When a tenant is created via `POST /api/control/tenants`, the control plane:
1. Inserts a `tenants` row with `status = pending`
2. Assigns a default bundle (`cp_tenant_bundle`)
3. Seeds entitlements (`cp_entitlements`)
4. Writes a `tenant.provisioning_started` event to the provisioning outbox

The outbox relay publishes that event to NATS. **But nobody listens.** The 7-step provisioning lifecycle defined in `lifecycle.rs` is never executed. Per-module databases are never created, migrations never run, and the tenant is never activated. The only way to provision today is the `tenantctl` CLI, which is manual and has no status tracking or failure recovery.

## Architecture

### Where it runs

The provisioning orchestrator runs **inside the control-plane process** as a NATS consumer, not as a separate service. Reasons:

- The control plane already connects to NATS for outbox publishing
- It already has the tenant-registry database pool
- Adding a consumer avoids a new deployment unit, new database connection, and new Docker service
- The 7-step sequence touches only the tenant-registry DB and module Postgres instances — all reachable from the control plane

The consumer subscribes to `tenant.provisioning_started` using a durable NATS JetStream consumer (so events survive restarts and are delivered exactly once).

### Consumer wiring

```
main.rs startup
  ├── pool connects to tenant-registry DB
  ├── NATS outbox relay spawns (existing)
  └── provisioning consumer spawns (NEW)
        └── subscribes to "tenant.provisioning_started" (durable: "provisioning-orchestrator")
```

On receiving an event, the orchestrator:
1. Deserializes the payload → `tenant_id`, `product_code`, `plan_code`
2. Looks up the tenant's assigned bundle via `cp_tenant_bundle` → `cp_bundle_modules`
3. Runs the 7-step provisioning sequence for each module in the bundle
4. On success: transitions tenant to `active`, writes `tenant.provisioned` to outbox
5. On failure: transitions tenant to `failed`, writes `tenant.provisioning_failed` to outbox

### Why not a separate worker?

A separate binary would need its own database pool, its own NATS connection, its own Docker service, its own health check, and its own deployment config. The provisioning workload is low-frequency (tenants are created infrequently) and short-lived (provisioning takes seconds, not hours). A separate service adds operational overhead with no scaling benefit.

If throughput becomes a concern later, the durable consumer can be scaled by adding consumer group members — but this is unlikely for a first customer deployment.

## Step Sequence

The 7 steps are already defined in `tenant_registry::lifecycle::standard_provisioning_sequence()`. The orchestrator executes them in order:

| Step | Name | What It Does | Failure Is |
|------|------|-------------|------------|
| 1 | `validate_tenant_id` | Confirm tenant row exists and is in `pending` status | Fatal — abort |
| 2 | `create_tenant_databases` | Create per-module PostgreSQL databases for all modules in the bundle | Retryable |
| 3 | `run_schema_migrations` | Apply SQLx migrations to each module database | Retryable |
| 4 | `seed_initial_data` | Seed required initial data (chart of accounts, default settings) | Retryable |
| 5 | `verify_database_connectivity` | Ping each module database | Retryable |
| 6 | `verify_schema_versions` | Record applied migration versions in `tenants.module_schema_versions` | Retryable |
| 7 | `activate_tenant` | Set tenant status to `active` | Fatal — rollback |

### Step execution model

```
for step in steps:
    UPDATE provisioning_steps SET status='in_progress', started_at=now()
    
    result = execute_step(step)
    
    if result.ok:
        UPDATE provisioning_steps SET status='completed', completed_at=now(),
               verification_result=result.checks
    else:
        UPDATE provisioning_steps SET status='failed', error_message=result.error
        → enter failure handling (see below)
```

Each step writes its status to the existing `provisioning_steps` table. This table already has the right schema: `step_name`, `step_order`, `status` (pending/in_progress/completed/failed), `started_at`, `completed_at`, `error_message`, `verification_result`.

### Resume-from-last

On retry (after a failure is corrected or after a crash), the orchestrator does not re-run completed steps. It queries `provisioning_steps` for the tenant and resumes from the first non-completed step. This makes the entire sequence **idempotent**: re-delivering the `tenant.provisioning_started` event is safe because completed steps are skipped.

## Per-Module Database Provisioning (Steps 2–3)

### Which modules to provision

The bundle determines which modules a tenant gets. The orchestrator reads:

```sql
SELECT bm.module_code, bm.module_version
FROM cp_tenant_bundle tb
JOIN cp_bundle_modules bm ON bm.bundle_id = tb.bundle_id
WHERE tb.tenant_id = $1
```

This returns rows like `("ar", "latest")`, `("gl", "latest")`, `("payments", "latest")`.

### Module registry

The orchestrator needs to know, for each `module_code`, the database connection details (host, port, user, password) and the migrations path. This information currently lives as hardcoded constants in `tenantctl/src/provision.rs` (`MODULE_DEFS`).

For the orchestrator, this configuration should be loaded from environment variables using a standard pattern:

```
{MODULE}_POSTGRES_HOST, {MODULE}_POSTGRES_PORT,
{MODULE}_POSTGRES_USER, {MODULE}_POSTGRES_PASSWORD
```

These are already present in `docker-compose.services.yml` for each module. The migrations path is derivable from `module.toml` (`database.migrations` field) or from the convention `./modules/{module_code}/db/migrations`.

A `ModuleRegistry` struct loads these at control-plane startup and provides a lookup by `module_code`.

### Database naming

Following the existing pattern in `tenantctl`:

```
tenant_{sanitized_uuid}_{module_code}_db
```

Where `sanitized_uuid` replaces hyphens with underscores. Example: `tenant_550e8400_e29b_41d4_a716_446655440000_ar_db`

### Database creation

Step 2 connects to each module's Postgres instance (to the `postgres` admin database) and runs:

```sql
CREATE DATABASE "tenant_{sanitized_uuid}_{module_code}_db"
```

This is the exact pattern in `tenantctl::provision::provision_tenant_module`. The orchestrator reuses this logic.

If the database already exists (idempotency check via `pg_database`), the step records success without recreating it.

### Migration execution

Step 3 connects to each tenant-specific database and runs SQLx migrations from the module's migrations directory:

```rust
let migrator = sqlx::migrate::Migrator::new(Path::new(&migrations_path)).await?;
migrator.run(&tenant_pool).await?;
```

This is the existing `tenantctl` pattern. The orchestrator reuses it directly.

## Status Tracking

### Tenant status

The `tenants.status` column tracks the overall lifecycle. The orchestrator transitions it through the state machine defined in `lifecycle.rs`:

```
pending → provisioning → active
                       → failed
```

The `is_valid_provisioning_transition` function enforces valid transitions.

### Step-level status

The `provisioning_steps` table tracks each step independently:

```
pending → in_progress → completed
                      → failed
```

Steps are seeded when provisioning begins (all 7 rows inserted with `status = pending`). This gives visibility into exactly where provisioning stands at any moment.

### Progress query

To check provisioning progress for a tenant:

```sql
SELECT step_name, step_order, status, started_at, completed_at, error_message
FROM provisioning_steps
WHERE tenant_id = $1
ORDER BY step_order
```

This powers the existing `GET /api/control/tenants/{tenant_id}/summary` endpoint with step-level detail.

## Failure Handling

### Retry policy

When a retryable step fails, the orchestrator applies exponential backoff:

| Attempt | Delay |
|---------|-------|
| 1 | Immediate |
| 2 | 2 seconds |
| 3 | 8 seconds |

After 3 attempts, the step is marked `failed`, the tenant is transitioned to `failed`, and a `tenant.provisioning_failed` event is written to the outbox. An operator can then:

1. Fix the underlying issue (e.g., restore a module's Postgres instance)
2. Re-trigger provisioning by publishing a new `tenant.provisioning_started` event (or calling a future `POST /api/control/tenants/{tenant_id}/retry` endpoint)

The resume-from-last behavior ensures only the failed step and subsequent steps are re-attempted.

### Fatal vs retryable

- **Fatal** (steps 1, 7): Validation failure or activation failure means a logic error — retrying won't help. The tenant goes directly to `failed` status.
- **Retryable** (steps 2–6): Infrastructure failures (database unreachable, migration timeout) are transient. The orchestrator retries with backoff.

### Partial failure and consistency

The key invariant: **a tenant is never activated unless all steps succeed.**

If step 4 (seed initial data) fails after step 3 (migrations) succeeds:
- Module databases exist and have schemas applied (steps 2–3 completed)
- But the tenant stays in `provisioning` status (not `active`)
- No traffic reaches the tenant because module routes check `tenants.status = active`
- The `provisioning_steps` table shows exactly which step failed and why

There is **no rollback of completed steps**. Rationale:
- Creating databases and running migrations are safe operations — an unused database doesn't cause harm
- Dropping databases to "undo" provisioning risks data loss if the tenant was partially seeded
- The correct recovery path is forward: fix the issue and resume, not reverse

This matches the provisioning model in `tenantctl`, which also moves forward without undo.

### Crash recovery

If the control-plane crashes mid-provisioning:
1. The NATS durable consumer has not acknowledged the message (ack happens after step 7)
2. On restart, NATS redelivers the `tenant.provisioning_started` event
3. The orchestrator sees completed steps in `provisioning_steps` and resumes from where it stopped

This gives exactly-once execution semantics at the step level without distributed transactions.

## SDK Hook for Vertical Participation

### Problem

Platform modules (AR, GL, payments) have well-known database structures and seed data. But verticals (like TrashTech) may need custom provisioning steps — creating vertical-specific database tables, seeding vertical-specific data, or registering the tenant in vertical-specific systems.

### Design: Event-driven hooks

The provisioning orchestrator publishes progress events at defined hook points. Verticals subscribe to these events and run their own provisioning steps:

```
Hook point 1: tenant.provisioning.databases_created
  → Published after step 2 completes
  → Verticals create their own databases

Hook point 2: tenant.provisioning.migrations_complete
  → Published after step 3 completes
  → Verticals run their own migrations

Hook point 3: tenant.provisioning.seed_complete
  → Published after step 4 completes
  → Verticals seed their own initial data

Hook point 4: tenant.provisioned
  → Published after step 7 (existing event)
  → Verticals activate their own tenant-specific features
```

### How verticals participate

A vertical subscribes to the hook events it cares about:

```rust
// In the vertical's main.rs (e.g., TrashTech)
ModuleBuilder::from_manifest("module.toml")
    .consumer("tenant.provisioning.migrations_complete", |ctx, env| async move {
        let tenant_id = env.payload["tenant_id"].as_str().unwrap();
        // Create TrashTech-specific tables, seed route templates, etc.
        Ok(())
    })
    .routes(|ctx| { /* ... */ })
    .run()
    .await
```

### Why events, not a registration API

An alternative design would have verticals register provisioning steps with the control plane via an API call. The orchestrator would then execute those steps as part of its sequence.

This is rejected because:
- It creates coupling — the control plane must know how to call into verticals
- It requires the vertical to be running during provisioning (the orchestrator calls the vertical's endpoint)
- It breaks the platform's event-driven architecture where modules communicate via NATS, not direct HTTP calls

The event-driven approach lets verticals:
- Run their own provisioning independently of the platform sequence
- Retry their own steps without involving the platform orchestrator
- Be offline during platform provisioning and catch up later via durable NATS consumers

### Vertical provisioning status

Verticals that need coordinated status tracking should publish their own events (e.g., `trashtech.tenant.provisioned`) that the platform can optionally aggregate. This is out of scope for this design but follows naturally from the event-driven pattern.

## Module Registry Configuration

### Current state

`tenantctl/src/provision.rs` hardcodes 5 modules: AR, payments, subscriptions, GL, notifications. This is sufficient for the CLI tool but doesn't scale to 25 modules.

### Design

The orchestrator reads `module.toml` files from all modules at startup to build a registry. Each `module.toml` already declares:
- `module.name` — the module code (e.g., `"ar"`)
- `database.migrations` — path to migrations directory
- `server.port` — HTTP port for health checks

The remaining piece — database connection details — comes from environment variables following the existing `docker-compose.services.yml` pattern:

```
{MODULE_UPPER}_POSTGRES_USER, {MODULE_UPPER}_POSTGRES_PASSWORD,
{MODULE_UPPER}_POSTGRES_HOST (default: 7d-{module}-postgres),
{MODULE_UPPER}_POSTGRES_PORT (default: 5432)
```

### Struct

```rust
pub struct ModuleRegistry {
    modules: HashMap<String, ModuleProvisioningConfig>,
}

pub struct ModuleProvisioningConfig {
    pub module_code: String,
    pub postgres_host: String,
    pub postgres_port: u16,
    pub postgres_user: String,
    pub postgres_password: String,
    pub migrations_path: PathBuf,
    pub http_port: u16,
}
```

The registry validates at startup that all modules referenced by `cp_bundle_modules` have corresponding configuration. If a bundle references a module that isn't configured, the control plane logs a warning but continues (the provisioning step will fail with a clear error for that specific tenant).

## Outbox Events

The orchestrator writes these events to the existing `provisioning_outbox` table:

| Event Type | When | Payload |
|-----------|------|---------|
| `tenant.provisioning_started` | Existing — written by `create_tenant` handler | `{tenant_id, product_code, plan_code, app_id}` |
| `tenant.provisioning.databases_created` | After step 2 | `{tenant_id, modules: ["ar", "gl", ...]}` |
| `tenant.provisioning.migrations_complete` | After step 3 | `{tenant_id, module_versions: {"ar": "20260301...", ...}}` |
| `tenant.provisioning.seed_complete` | After step 4 | `{tenant_id, seed_data: ["chart_of_accounts", ...]}` |
| `tenant.provisioned` | After step 7 | `{tenant_id, duration_ms, module_versions}` |
| `tenant.provisioning_failed` | On unrecoverable failure | `{tenant_id, failed_step, error, attempts}` |

The existing outbox relay publishes these to NATS automatically — no relay changes needed.

## API Surface

### New endpoint: Retry provisioning

```
POST /api/control/tenants/{tenant_id}/retry
```

Re-publishes a `tenant.provisioning_started` event for a tenant in `failed` status. Resets the tenant status to `pending`. The orchestrator picks up the event and resumes from the first non-completed step.

Guard: returns 409 if tenant is not in `failed` status.

### New endpoint: Provisioning status

```
GET /api/control/tenants/{tenant_id}/provisioning
```

Returns the provisioning step detail:

```json
{
  "tenant_id": "...",
  "status": "provisioning",
  "steps": [
    {"step": "validate_tenant_id", "order": 1, "status": "completed", "completed_at": "..."},
    {"step": "create_tenant_databases", "order": 2, "status": "completed", "completed_at": "..."},
    {"step": "run_schema_migrations", "order": 3, "status": "in_progress", "started_at": "..."},
    {"step": "seed_initial_data", "order": 4, "status": "pending"},
    {"step": "verify_database_connectivity", "order": 5, "status": "pending"},
    {"step": "verify_schema_versions", "order": 6, "status": "pending"},
    {"step": "activate_tenant", "order": 7, "status": "pending"}
  ]
}
```

This reads directly from the `provisioning_steps` table.

## Schema References

### Existing tables used by the orchestrator

**`tenants`** (migration `20260216000001`):
- `tenant_id UUID PRIMARY KEY`
- `status VARCHAR(20)` — pending, provisioning, active, failed, suspended, deleted, trial, past_due
- `module_schema_versions JSONB` — per-module migration versions

**`provisioning_steps`** (migration `20260216000001`):
- `step_id UUID PRIMARY KEY`
- `tenant_id UUID REFERENCES tenants`
- `step_name VARCHAR(100)`
- `step_order INTEGER`
- `status VARCHAR(20)` — pending, in_progress, completed, failed
- `started_at, completed_at TIMESTAMPTZ`
- `error_message TEXT`
- `verification_result JSONB`
- `UNIQUE (tenant_id, step_name)`

**`provisioning_outbox`** (migration `20260217000001`):
- `id UUID PRIMARY KEY`
- `tenant_id UUID REFERENCES tenants`
- `event_type VARCHAR(100)`
- `payload JSONB`
- `created_at, published_at TIMESTAMPTZ`

**`cp_bundles`** (migration `20260220000002`):
- `bundle_id UUID PRIMARY KEY`
- `product_code TEXT`
- `bundle_name TEXT`
- `is_default BOOLEAN`

**`cp_bundle_modules`** (migration `20260220000002`):
- `(bundle_id, module_code) PRIMARY KEY`
- `module_version TEXT DEFAULT 'latest'`

**`cp_tenant_bundle`** (migration `20260220000002`):
- `tenant_id UUID PRIMARY KEY REFERENCES tenants`
- `bundle_id UUID REFERENCES cp_bundles`
- `status TEXT` — active, in_transition

### No new tables required

All required tables already exist. The orchestrator uses them as designed.

## Implementation Scope

The implementation bead (`bd-5a957`) should deliver:

1. **`ModuleRegistry`** — loads module configs from env vars + `module.toml` files
2. **`ProvisioningOrchestrator`** — NATS consumer that drives the 7-step sequence
3. **Consumer wiring** — spawn the orchestrator in `main.rs` alongside the outbox relay
4. **Retry endpoint** — `POST /api/control/tenants/{tenant_id}/retry`
5. **Status endpoint** — `GET /api/control/tenants/{tenant_id}/provisioning`
6. **Hook events** — publish intermediate progress events at hook points

Files to create/modify:
- `platform/control-plane/src/provisioning/mod.rs` — orchestrator module
- `platform/control-plane/src/provisioning/registry.rs` — module registry
- `platform/control-plane/src/provisioning/steps.rs` — step execution logic
- `platform/control-plane/src/handlers/retry_provisioning.rs` — retry endpoint
- `platform/control-plane/src/handlers/provisioning_status.rs` — status endpoint
- `platform/control-plane/src/routes.rs` — add new routes
- `platform/control-plane/src/main.rs` — spawn consumer
