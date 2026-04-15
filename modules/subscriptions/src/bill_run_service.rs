//! Bill run service — business logic, orchestration, and event emission.
//!
//! The handler calls into this layer. SQL goes through [`crate::db::bill_run_repo`].

use chrono::{Datelike, NaiveDate, Utc};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use sqlx::PgPool;

use crate::envelope::create_subscriptions_envelope;
use crate::gated_invoice_creation::{create_gated_invoice, InvoiceCreationError};
use crate::models::{BillRunCompletedPayload, BillRunResult};
use crate::outbox::enqueue_event;

use crate::db::bill_run_repo as repo;

/// Execute a bill run: find due subscriptions, create invoices, emit event.
///
/// Returns a cached result if this `bill_run_id` was already executed
/// (idempotent replay).
pub async fn execute_bill_run(
    db: &PgPool,
    tenant_id: &str,
    bill_run_id: &str,
    execution_date: NaiveDate,
    tracing_ctx: &TracingContext,
    ar_client: &platform_client_ar::InvoicesClient,
) -> Result<BillRunResult, ApiError> {
    // Idempotency check
    let existing = repo::fetch_existing_bill_run(db, bill_run_id, tenant_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error checking existing bill run: {}", e);
            ApiError::internal("Failed to check existing bill run")
        })?;

    if let Some(existing) = existing {
        tracing::info!(
            "Bill run {} already executed, returning cached result",
            bill_run_id
        );
        return Ok(BillRunResult {
            bill_run_id: bill_run_id.to_string(),
            subscriptions_processed: existing.subscriptions_processed,
            invoices_created: existing.invoices_created,
            failures: existing.failures,
            execution_time: existing.created_at,
        });
    }

    // Create bill run record
    repo::insert_bill_run(db, bill_run_id, tenant_id, execution_date)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create bill run record: {}", e);
            ApiError::internal("Failed to create bill run record")
        })?;

    // Find and process subscriptions
    let subscriptions = repo::fetch_subscriptions_due(db, tenant_id, execution_date)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch subscriptions: {}", e);
            ApiError::internal("Failed to fetch subscriptions")
        })?;

    let subscriptions_processed = subscriptions.len() as i32;
    let mut invoices_created = 0;
    let mut failures = 0;

    for subscription in subscriptions {
        tracing::info!(
            "Processing subscription {} for customer {}",
            subscription.id,
            subscription.ar_customer_id
        );

        let ar_customer_id: i32 = match subscription.ar_customer_id.parse() {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(
                    "Failed to parse ar_customer_id '{}': {}",
                    subscription.ar_customer_id,
                    e
                );
                failures += 1;
                continue;
            }
        };

        match create_gated_invoice(
            db,
            tenant_id,
            subscription.id,
            ar_customer_id,
            subscription.price_minor,
            subscription.next_bill_date,
            &ar_client,
        )
        .await
        {
            Ok(result) => {
                tracing::info!(
                    "Created gated invoice {} for subscription {} (cycle {})",
                    result.invoice_id,
                    subscription.id,
                    result.cycle_key
                );
                invoices_created += 1;

                let new_next_bill_date =
                    calculate_next_bill_date(&subscription.next_bill_date, &subscription.schedule);

                if let Err(e) = repo::update_subscription_next_bill_date(
                    db,
                    subscription.id,
                    new_next_bill_date,
                )
                .await
                {
                    tracing::error!("Failed to update subscription next_bill_date: {}", e);
                }
            }
            Err(InvoiceCreationError::DuplicateCycle {
                subscription_id,
                cycle_key,
            }) => {
                tracing::info!(
                    "Subscription {} already billed for cycle {} (idempotent skip)",
                    subscription_id,
                    cycle_key
                );
            }
            Err(e) => {
                tracing::error!(
                    "Failed to create invoice for subscription {}: {}",
                    subscription.id,
                    e
                );
                failures += 1;
            }
        }
    }

    // Finalize bill run record
    let execution_time = Utc::now();
    repo::complete_bill_run(
        db,
        bill_run_id,
        tenant_id,
        subscriptions_processed,
        invoices_created,
        failures,
        execution_time,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to update bill run record: {}", e);
        ApiError::internal("Failed to update bill run record")
    })?;

    // Emit billrun.completed event
    let payload = BillRunCompletedPayload {
        bill_run_id: bill_run_id.to_string(),
        subscriptions_processed,
        invoices_created,
        failures,
        execution_time,
    };

    let envelope = create_subscriptions_envelope(
        uuid::Uuid::new_v4(),
        tenant_id.to_string(),
        "billrun.completed".to_string(),
        None,
        None,
        "LIFECYCLE".to_string(),
        payload,
    )
    .with_tracing_context(tracing_ctx);

    enqueue_event(db, "billrun.completed", &envelope)
        .await
        .map_err(|e| {
            tracing::error!("Failed to enqueue event: {}", e);
            ApiError::internal("Failed to enqueue event")
        })?;

    tracing::info!(
        "Bill run {} completed: processed={}, created={}, failures={}",
        bill_run_id,
        subscriptions_processed,
        invoices_created,
        failures
    );

    Ok(BillRunResult {
        bill_run_id: bill_run_id.to_string(),
        subscriptions_processed,
        invoices_created,
        failures,
        execution_time,
    })
}

/// Calculate next bill date based on schedule.
fn calculate_next_bill_date(current_date: &NaiveDate, schedule: &str) -> NaiveDate {
    match schedule {
        "weekly" => *current_date + chrono::Duration::weeks(1),
        "monthly" => {
            let year = current_date.year();
            let month = current_date.month();
            let day = current_date.day();

            if month == 12 {
                NaiveDate::from_ymd_opt(year + 1, 1, day).unwrap_or_else(|| {
                    NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("Jan 1 is valid")
                })
            } else {
                NaiveDate::from_ymd_opt(year, month + 1, day).unwrap_or_else(|| {
                    NaiveDate::from_ymd_opt(year, month + 1, 1).expect("first of month is valid")
                })
            }
        }
        _ => *current_date + chrono::Duration::weeks(4),
    }
}
