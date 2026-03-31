//! Bill run execution handler.
//!
//! POST /api/bill-runs/execute — Execute billing cycle

use axum::{extract::State, http::StatusCode, Extension, Json};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use sqlx::PgPool;
use uuid::Uuid;

use crate::envelope::create_subscriptions_envelope;
use crate::gated_invoice_creation::{create_gated_invoice, InvoiceCreationError};
use crate::models::{BillRunCompletedPayload, BillRunResult, ExecuteBillRunRequest};
use crate::outbox::enqueue_event;

fn extract_tenant(claims: &Option<Extension<VerifiedClaims>>) -> Result<String, ApiError> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(ApiError::unauthorized("Missing or invalid authentication")),
    }
}

fn with_request_id(err: ApiError, ctx: &Option<Extension<TracingContext>>) -> ApiError {
    match ctx {
        Some(Extension(c)) => {
            if let Some(tid) = &c.trace_id {
                err.with_request_id(tid.clone())
            } else {
                err
            }
        }
        None => err,
    }
}

/// POST /api/bill-runs/execute - Execute billing cycle
#[utoipa::path(
    post, path = "/api/bill-runs/execute", tag = "Bill Runs",
    request_body = ExecuteBillRunRequest,
    responses(
        (status = 200, description = "Bill run completed (or idempotent replay)", body = BillRunResult),
        (status = 401, body = ApiError), (status = 500, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn execute_bill_run(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<ExecuteBillRunRequest>,
) -> Result<(StatusCode, Json<BillRunResult>), ApiError> {
    let raw_ctx = tracing_ctx.as_ref().map(|Extension(c)| c.clone()).unwrap_or_default();
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &tracing_ctx))?;

    let bill_run_id = req
        .bill_run_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let execution_date = req
        .execution_date
        .unwrap_or_else(|| Utc::now().date_naive());

    // Check if this bill run has already been executed (idempotency)
    #[derive(sqlx::FromRow)]
    struct ExistingBillRun {
        #[allow(dead_code)]
        id: Uuid,
        subscriptions_processed: i32,
        invoices_created: i32,
        failures: i32,
        created_at: DateTime<Utc>,
    }

    let existing = sqlx::query_as::<_, ExistingBillRun>(
        "SELECT id, subscriptions_processed, invoices_created, failures, created_at
         FROM bill_runs
         WHERE bill_run_id = $1 AND tenant_id = $2",
    )
    .bind(&bill_run_id)
    .bind(&tenant_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error checking existing bill run: {}", e);
        with_request_id(ApiError::internal("Failed to check existing bill run"), &tracing_ctx)
    })?;

    if let Some(existing) = existing {
        tracing::info!(
            "Bill run {} already executed, returning cached result",
            bill_run_id
        );
        return Ok((
            StatusCode::OK,
            Json(BillRunResult {
                bill_run_id,
                subscriptions_processed: existing.subscriptions_processed,
                invoices_created: existing.invoices_created,
                failures: existing.failures,
                execution_time: existing.created_at,
            }),
        ));
    }

    // Create bill run record
    sqlx::query(
        "INSERT INTO bill_runs (bill_run_id, tenant_id, execution_date, status)
         VALUES ($1, $2, $3, 'running')",
    )
    .bind(&bill_run_id)
    .bind(&tenant_id)
    .bind(execution_date)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create bill run record: {}", e);
        with_request_id(ApiError::internal("Failed to create bill run record"), &tracing_ctx)
    })?;

    // Find subscriptions due for billing
    #[derive(sqlx::FromRow)]
    struct SubscriptionDue {
        id: Uuid,
        tenant_id: String,
        ar_customer_id: String,
        price_minor: i64,
        #[allow(dead_code)]
        currency: String,
        #[allow(dead_code)]
        next_bill_date: NaiveDate,
        schedule: String,
    }

    let subscriptions = sqlx::query_as::<_, SubscriptionDue>(
        "SELECT id, tenant_id, ar_customer_id, price_minor, currency, next_bill_date, schedule
         FROM subscriptions
         WHERE status = 'active'
           AND tenant_id = $1
           AND next_bill_date <= $2",
    )
    .bind(&tenant_id)
    .bind(execution_date)
    .fetch_all(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch subscriptions: {}", e);
        with_request_id(ApiError::internal("Failed to fetch subscriptions"), &tracing_ctx)
    })?;

    let subscriptions_processed = subscriptions.len() as i32;
    let mut invoices_created = 0;
    let mut failures = 0;

    let ar_base_url =
        std::env::var("AR_BASE_URL").unwrap_or_else(|_| "http://localhost:8086".to_string());

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
            &db,
            &tenant_id,
            subscription.id,
            ar_customer_id,
            subscription.price_minor,
            subscription.next_bill_date,
            &ar_base_url,
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

                let new_next_bill_date = calculate_next_bill_date(
                    &subscription.next_bill_date,
                    &subscription.schedule,
                );

                let update_result = sqlx::query(
                    "UPDATE subscriptions
                     SET next_bill_date = $1, updated_at = NOW()
                     WHERE id = $2",
                )
                .bind(new_next_bill_date)
                .bind(subscription.id)
                .execute(&db)
                .await;

                if let Err(e) = update_result {
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

    // Update bill run record
    let execution_time = Utc::now();
    sqlx::query(
        "UPDATE bill_runs
         SET subscriptions_processed = $1,
             invoices_created = $2,
             failures = $3,
             status = 'completed',
             updated_at = $4
         WHERE bill_run_id = $5 AND tenant_id = $6",
    )
    .bind(subscriptions_processed)
    .bind(invoices_created)
    .bind(failures)
    .bind(execution_time)
    .bind(&bill_run_id)
    .bind(&tenant_id)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update bill run record: {}", e);
        with_request_id(ApiError::internal("Failed to update bill run record"), &tracing_ctx)
    })?;

    // Emit subscriptions.billrun.completed event
    let payload = BillRunCompletedPayload {
        bill_run_id: bill_run_id.clone(),
        subscriptions_processed,
        invoices_created,
        failures,
        execution_time,
    };

    let envelope = create_subscriptions_envelope(
        uuid::Uuid::new_v4(),
        tenant_id,
        "billrun.completed".to_string(),
        None,
        None,
        "LIFECYCLE".to_string(),
        payload,
    )
    .with_tracing_context(&raw_ctx);

    enqueue_event(&db, "billrun.completed", &envelope)
        .await
        .map_err(|e| {
            tracing::error!("Failed to enqueue event: {}", e);
            with_request_id(ApiError::internal("Failed to enqueue event"), &tracing_ctx)
        })?;

    tracing::info!(
        "Bill run {} completed: processed={}, created={}, failures={}",
        bill_run_id,
        subscriptions_processed,
        invoices_created,
        failures
    );

    Ok((
        StatusCode::OK,
        Json(BillRunResult {
            bill_run_id,
            subscriptions_processed,
            invoices_created,
            failures,
            execution_time,
        }),
    ))
}

/// Calculate next bill date based on schedule
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
