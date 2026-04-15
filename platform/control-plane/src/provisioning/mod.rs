//! Provisioning orchestrator
//!
//! Async worker that listens for `tenant.provisioning_started` events on NATS
//! and drives the 7-step provisioning sequence. Runs inside the control-plane
//! process alongside the outbox relay.

pub mod registry;
pub mod steps;
pub mod tracking;
pub mod worker;

use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tenant_registry::{event_types, step_names};
use uuid::Uuid;

use registry::ModuleRegistry;
use steps::{StepError, StepOutcome};

/// Maximum retry attempts for retryable steps
const MAX_RETRIES: u32 = 3;

/// Backoff schedule for retries (seconds)
const RETRY_BACKOFFS: [u64; 3] = [0, 2, 8];

/// Total time budget for polling module /api/ready endpoints during activation
const ACTIVATE_READY_TIMEOUT: Duration = Duration::from_secs(90);

/// Interval for recovery polling (checks for stuck tenants)
const RECOVERY_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Hook event subjects published at provisioning milestones
mod hook_events {
    pub const DATABASES_CREATED: &str = "tenant.provisioning.databases_created";
    pub const MIGRATIONS_COMPLETE: &str = "tenant.provisioning.migrations_complete";
    pub const SEED_COMPLETE: &str = "tenant.provisioning.seed_complete";
}

/// Start the provisioning orchestrator.
///
/// Subscribes to `tenant.provisioning_started` on NATS and also polls the DB
/// for tenants stuck in `pending` status (crash recovery). Both paths converge
/// on the same idempotent `provision_tenant` function.
pub async fn start_provisioning_consumer(
    pool: PgPool,
    bus: Arc<dyn EventBus>,
    module_registry: ModuleRegistry,
) {
    tracing::info!("provisioning orchestrator started");
    module_registry.validate_migrations();

    // Spawn recovery poller alongside the NATS listener
    let recovery_pool = pool.clone();
    let recovery_registry = module_registry.clone();
    tokio::spawn(async move {
        recovery_loop(recovery_pool, recovery_registry).await;
    });

    // NATS subscription
    let subject = event_types::TENANT_PROVISIONING_STARTED;
    let stream = match bus.subscribe(subject).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to subscribe to {subject} — orchestrator disabled");
            return;
        }
    };

    tracing::info!(subject = %subject, "subscribed to provisioning events");
    consume_events(stream, pool, module_registry).await;
}

async fn consume_events(
    mut stream: futures::stream::BoxStream<'static, BusMessage>,
    pool: PgPool,
    registry: ModuleRegistry,
) {
    while let Some(msg) = stream.next().await {
        let tenant_id = match parse_tenant_id(&msg) {
            Some(id) => id,
            None => {
                tracing::warn!("ignoring event with unparseable tenant_id");
                continue;
            }
        };

        tracing::info!(tenant_id = %tenant_id, "received provisioning event");

        if let Err(e) = provision_tenant(&pool, &registry, tenant_id).await {
            tracing::error!(tenant_id = %tenant_id, error = %e, "provisioning failed");
        }
    }

    tracing::warn!("NATS subscription ended — provisioning consumer stopping");
}

/// Recovery loop: periodically checks for tenants in pending/provisioning status
/// that may have been missed (e.g., after a crash).
async fn recovery_loop(pool: PgPool, registry: ModuleRegistry) {
    loop {
        tokio::time::sleep(RECOVERY_POLL_INTERVAL).await;

        match find_stuck_tenants(&pool).await {
            Ok(tenant_ids) => {
                for tid in tenant_ids {
                    tracing::info!(tenant_id = %tid, "recovery: retrying stuck tenant");
                    if let Err(e) = provision_tenant(&pool, &registry, tid).await {
                        tracing::error!(tenant_id = %tid, error = %e, "recovery provisioning failed");
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "recovery poll query failed");
            }
        }
    }
}

/// Find tenants that are pending or provisioning (potentially stuck).
/// Only returns tenants that have been in pending for >10s (avoid racing
/// with the NATS event path).
async fn find_stuck_tenants(pool: &PgPool) -> Result<Vec<Uuid>, sqlx::Error> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT tenant_id FROM tenants \
         WHERE status IN ('pending', 'provisioning') \
         AND updated_at < NOW() - INTERVAL '10 seconds' \
         ORDER BY created_at ASC \
         LIMIT 5",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|(id,)| id).collect())
}

fn parse_tenant_id(msg: &BusMessage) -> Option<Uuid> {
    let payload: serde_json::Value = serde_json::from_slice(&msg.payload).ok()?;
    let tenant_id_str = payload.get("tenant_id")?.as_str()?;
    tenant_id_str.parse::<Uuid>().ok()
}

/// Drive the 7-step provisioning sequence for a tenant.
///
/// Idempotent: completed steps are skipped via resume-from-last.
/// On failure: tenant is set to `failed` and an outbox event is written.
pub async fn provision_tenant(
    pool: &PgPool,
    registry: &ModuleRegistry,
    tenant_id: Uuid,
) -> Result<(), String> {
    // Transition to provisioning (idempotent — allows pending or provisioning)
    let status: Option<(String,)> =
        sqlx::query_as("SELECT status FROM tenants WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;

    match status.as_ref().map(|s| s.0.as_str()) {
        Some("active") => {
            tracing::info!(tenant_id = %tenant_id, "already active — skipping");
            return Ok(());
        }
        Some("degraded") => {
            tracing::info!(tenant_id = %tenant_id, "tenant degraded — skipping (use retry endpoint)");
            return Ok(());
        }
        Some("failed") => {
            tracing::info!(tenant_id = %tenant_id, "tenant in failed state — skipping (use retry endpoint)");
            return Ok(());
        }
        Some("pending") => {
            sqlx::query("UPDATE tenants SET status = 'provisioning', updated_at = NOW() WHERE tenant_id = $1")
                .bind(tenant_id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
        }
        Some("provisioning") => { /* already in progress, resume */ }
        Some(other) => {
            return Err(format!("unexpected tenant status: {other}"));
        }
        None => {
            return Err(format!("tenant {tenant_id} not found"));
        }
    }

    // Seed provisioning step rows
    tracking::seed_provisioning_steps(pool, tenant_id)
        .await
        .map_err(|e| e.to_string())?;

    // Resolve bundle modules
    let module_codes = steps::resolve_bundle_modules(pool, tenant_id)
        .await
        .map_err(|e| e.to_string())?;

    if module_codes.is_empty() {
        tracing::warn!(tenant_id = %tenant_id, "no modules in bundle — skipping DB provisioning");
    }

    // Seed per-module status rows so progress is visible immediately
    worker::seed_module_statuses(pool, tenant_id, &module_codes)
        .await
        .map_err(|e| e.to_string())?;

    // Execute steps, resuming from the first non-completed step
    let step_sequence = [
        step_names::VALIDATE_TENANT_ID,
        step_names::CREATE_TENANT_DATABASES,
        step_names::RUN_SCHEMA_MIGRATIONS,
        step_names::SEED_INITIAL_DATA,
        step_names::VERIFY_DATABASE_CONNECTIVITY,
        step_names::VERIFY_SCHEMA_VERSIONS,
        step_names::ACTIVATE_TENANT,
    ];

    let resume_from = tracking::first_pending_step(pool, tenant_id)
        .await
        .map_err(|e| e.to_string())?;

    let start_step = match &resume_from {
        Some(name) => name.as_str(),
        None => {
            tracing::info!(tenant_id = %tenant_id, "all steps completed — activating");
            return Ok(());
        }
    };

    let start_idx = step_sequence
        .iter()
        .position(|s| *s == start_step)
        .unwrap_or(0);

    for &step_name in &step_sequence[start_idx..] {
        let result =
            execute_step_with_retry(pool, registry, tenant_id, step_name, &module_codes).await;

        match result {
            Ok(outcome) => {
                tracking::mark_step_completed(pool, tenant_id, step_name, &outcome.checks)
                    .await
                    .map_err(|e| e.to_string())?;

                // Publish hook events at milestones
                publish_hook_if_needed(pool, tenant_id, step_name, &module_codes).await;
            }
            Err(e) => {
                let err_msg = e.to_string();
                tracking::mark_step_failed(pool, tenant_id, step_name, &err_msg)
                    .await
                    .ok();

                // Transition tenant to failed
                fail_tenant(pool, tenant_id, step_name, &err_msg).await;
                return Err(format!("step {step_name} failed: {err_msg}"));
            }
        }
    }

    tracing::info!(tenant_id = %tenant_id, "provisioning complete — tenant active");
    Ok(())
}

async fn execute_step_with_retry(
    pool: &PgPool,
    registry: &ModuleRegistry,
    tenant_id: Uuid,
    step_name: &str,
    module_codes: &[String],
) -> Result<StepOutcome, StepError> {
    let max_attempts = if tracking::is_fatal_step(step_name) {
        1
    } else {
        MAX_RETRIES
    };

    for attempt in 0..max_attempts {
        if attempt > 0 {
            let backoff = RETRY_BACKOFFS.get(attempt as usize).copied().unwrap_or(8);
            tokio::time::sleep(Duration::from_secs(backoff)).await;
        }

        tracking::mark_step_in_progress(pool, tenant_id, step_name)
            .await
            .ok();

        match execute_step(pool, registry, tenant_id, step_name, module_codes).await {
            Ok(outcome) => return Ok(outcome),
            Err(e) if attempt + 1 < max_attempts => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    step = %step_name,
                    attempt = attempt + 1,
                    error = %e,
                    "retryable step failed, will retry"
                );
            }
            Err(e) => return Err(e),
        }
    }

    unreachable!()
}

async fn execute_step(
    pool: &PgPool,
    registry: &ModuleRegistry,
    tenant_id: Uuid,
    step_name: &str,
    module_codes: &[String],
) -> Result<StepOutcome, StepError> {
    match step_name {
        step_names::VALIDATE_TENANT_ID => steps::validate_tenant(pool, tenant_id).await,

        // Steps 2-5 are handled together by the bundle worker, which provisions each
        // module independently and tracks per-module status in cp_tenant_module_status.
        // CREATE_TENANT_DATABASES drives all per-module work; steps 3-5 are idempotently
        // "done" after the worker completes.
        step_names::CREATE_TENANT_DATABASES => {
            let summary = worker::provision_all_modules(pool, registry, tenant_id, module_codes)
                .await
                .map_err(StepError::Database)?;

            if summary.failed_count > 0 {
                let failed: Vec<&str> = summary
                    .results
                    .iter()
                    .filter(|r| !r.success)
                    .map(|r| r.module_code.as_str())
                    .collect();
                return Err(StepError::Migration(format!(
                    "{} module(s) failed provisioning: {}",
                    summary.failed_count,
                    failed.join(", ")
                )));
            }

            let ready: Vec<&str> = summary
                .results
                .iter()
                .map(|r| r.module_code.as_str())
                .collect();
            Ok(StepOutcome {
                checks: serde_json::json!({"modules_provisioned": ready}),
            })
        }

        // Handled by the bundle worker above; mark complete if reached during resume.
        step_names::RUN_SCHEMA_MIGRATIONS
        | step_names::SEED_INITIAL_DATA
        | step_names::VERIFY_DATABASE_CONNECTIVITY => Ok(StepOutcome {
            checks: serde_json::json!({
                "skipped": true,
                "reason": "completed by bundle worker in create_tenant_databases"
            }),
        }),

        step_names::VERIFY_SCHEMA_VERSIONS => {
            steps::verify_schema_versions(pool, tenant_id, module_codes, registry).await
        }
        step_names::ACTIVATE_TENANT => {
            let http_client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default();
            steps::activate_tenant(
                pool,
                tenant_id,
                module_codes,
                registry,
                &http_client,
                ACTIVATE_READY_TIMEOUT,
            )
            .await
        }
        unknown => Err(StepError::InvalidState(format!("unknown step: {unknown}"))),
    }
}

async fn publish_hook_if_needed(
    pool: &PgPool,
    tenant_id: Uuid,
    step_name: &str,
    module_codes: &[String],
) {
    let (event_type, payload) = match step_name {
        step_names::CREATE_TENANT_DATABASES => (
            hook_events::DATABASES_CREATED,
            json!({"tenant_id": tenant_id.to_string(), "modules": module_codes}),
        ),
        step_names::RUN_SCHEMA_MIGRATIONS => (
            hook_events::MIGRATIONS_COMPLETE,
            json!({"tenant_id": tenant_id.to_string(), "modules": module_codes}),
        ),
        step_names::SEED_INITIAL_DATA => (
            hook_events::SEED_COMPLETE,
            json!({"tenant_id": tenant_id.to_string(), "modules": module_codes}),
        ),
        _ => return,
    };

    if let Err(e) = tracking::write_hook_event(pool, tenant_id, event_type, payload).await {
        tracing::warn!(
            tenant_id = %tenant_id,
            event = %event_type,
            error = %e,
            "failed to write hook event (non-fatal)"
        );
    }
}

async fn fail_tenant(pool: &PgPool, tenant_id: Uuid, failed_step: &str, error: &str) {
    let now = chrono::Utc::now();

    if let Err(e) =
        sqlx::query("UPDATE tenants SET status = 'failed', updated_at = $1 WHERE tenant_id = $2")
            .bind(now)
            .bind(tenant_id)
            .execute(pool)
            .await
    {
        tracing::error!(error = %e, "failed to transition tenant to failed status");
    }

    let payload = json!({
        "tenant_id": tenant_id.to_string(),
        "failed_step": failed_step,
        "error": error,
    });

    if let Err(e) = tracking::write_hook_event(
        pool,
        tenant_id,
        event_types::TENANT_PROVISIONING_FAILED,
        payload,
    )
    .await
    {
        tracing::error!(error = %e, "failed to write provisioning_failed event");
    }
}

/// Load the module registry from the tenant-registry database.
/// Reads all unique module codes from `cp_bundle_modules` and configures
/// each from environment variables.
pub async fn load_registry_from_db(pool: &PgPool) -> Result<ModuleRegistry, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT module_code FROM cp_bundle_modules")
        .fetch_all(pool)
        .await?;

    let codes: Vec<String> = rows.into_iter().map(|(code,)| code).collect();
    tracing::info!(modules = ?codes, "loaded module codes from cp_bundle_modules");

    Ok(ModuleRegistry::from_env(&codes))
}
