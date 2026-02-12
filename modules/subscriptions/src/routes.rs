use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{
    BillRunCompletedPayload, BillRunResult, CreateInvoiceRequest, ErrorResponse,
    ExecuteBillRunRequest, FinalizeInvoiceRequest, Invoice,
};
use crate::envelope::create_subscriptions_envelope;
use crate::outbox::enqueue_event;

pub fn subscriptions_router(db: PgPool) -> Router {
    Router::new()
        .route("/api/bill-runs/execute", post(execute_bill_run))
        .with_state(db)
}

/// POST /api/bill-runs/execute - Execute billing cycle
async fn execute_bill_run(
    State(db): State<PgPool>,
    Json(req): Json<ExecuteBillRunRequest>,
) -> Result<(StatusCode, Json<BillRunResult>), (StatusCode, Json<ErrorResponse>)> {
    // Generate bill_run_id if not provided
    let bill_run_id = req.bill_run_id.unwrap_or_else(|| Uuid::new_v4().to_string());

    // Use today's date if not specified
    let execution_date = req.execution_date.unwrap_or_else(|| {
        Utc::now().date_naive()
    });

    // Check if this bill run has already been executed (idempotency)
    #[derive(sqlx::FromRow)]
    struct ExistingBillRun {
        id: Uuid,
        subscriptions_processed: i32,
        invoices_created: i32,
        failures: i32,
        created_at: DateTime<Utc>,
    }

    let existing = sqlx::query_as::<_, ExistingBillRun>(
        "SELECT id, subscriptions_processed, invoices_created, failures, created_at
         FROM bill_runs
         WHERE bill_run_id = $1"
    )
    .bind(&bill_run_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error checking existing bill run: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database_error".to_string(),
                message: "Failed to check existing bill run".to_string(),
                details: None,
            }),
        )
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
        "INSERT INTO bill_runs (bill_run_id, execution_date, status)
         VALUES ($1, $2, 'running')"
    )
    .bind(&bill_run_id)
    .bind(execution_date)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create bill run record: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database_error".to_string(),
                message: "Failed to create bill run record".to_string(),
                details: None,
            }),
        )
    })?;

    // Find subscriptions due for billing
    #[derive(sqlx::FromRow)]
    struct SubscriptionDue {
        id: Uuid,
        tenant_id: String,
        ar_customer_id: String,
        price_minor: i64,
        currency: String,
        next_bill_date: NaiveDate,
    }

    let subscriptions = sqlx::query_as::<_, SubscriptionDue>(
        "SELECT id, tenant_id, ar_customer_id, price_minor, currency, next_bill_date
         FROM subscriptions
         WHERE status = 'active'
           AND next_bill_date <= $1"
    )
    .bind(execution_date)
    .fetch_all(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch subscriptions: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database_error".to_string(),
                message: "Failed to fetch subscriptions".to_string(),
                details: None,
            }),
        )
    })?;

    let subscriptions_processed = subscriptions.len() as i32;
    let mut invoices_created = 0;
    let mut failures = 0;

    // Get AR service URL from environment
    let ar_base_url = std::env::var("AR_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8086".to_string());

    let client = reqwest::Client::new();

    // Process each subscription
    for subscription in subscriptions {
        tracing::info!(
            "Processing subscription {} for customer {}",
            subscription.id,
            subscription.ar_customer_id
        );

        // Call AR OpenAPI to create invoice
        // Parse ar_customer_id from String to i32
        let ar_customer_id: i32 = match subscription.ar_customer_id.parse() {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("Failed to parse ar_customer_id '{}': {}", subscription.ar_customer_id, e);
                failures += 1;
                continue;
            }
        };

        let create_invoice_req = CreateInvoiceRequest {
            ar_customer_id,
            amount_cents: subscription.price_minor as i32,
        };

        let create_result = client
            .post(&format!("{}/api/ar/invoices", ar_base_url))
            .json(&create_invoice_req)
            .send()
            .await;

        let invoice = match create_result {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Invoice>().await {
                        Ok(inv) => inv,
                        Err(e) => {
                            tracing::error!("Failed to parse invoice response: {}", e);
                            failures += 1;
                            continue;
                        }
                    }
                } else {
                    tracing::error!(
                        "AR API returned error status: {}",
                        response.status()
                    );
                    failures += 1;
                    continue;
                }
            }
            Err(e) => {
                tracing::error!("Failed to call AR API to create invoice: {}", e);
                failures += 1;
                continue;
            }
        };

        tracing::info!("Created invoice {} for subscription {}", invoice.id, subscription.id);

        // Call AR OpenAPI to finalize invoice
        let finalize_req = FinalizeInvoiceRequest {
            auto_advance: Some(true),
        };

        let finalize_result = client
            .post(&format!("{}/api/ar/invoices/{}/finalize", ar_base_url, invoice.id))
            .json(&finalize_req)
            .send()
            .await;

        match finalize_result {
            Ok(response) => {
                if response.status().is_success() {
                    tracing::info!("Finalized invoice {}", invoice.id);
                    invoices_created += 1;

                    // Update subscription next_bill_date
                    let new_next_bill_date = calculate_next_bill_date(
                        &subscription.next_bill_date,
                        &"monthly".to_string(), // TODO: get from subscription.schedule
                    );

                    let update_result = sqlx::query(
                        "UPDATE subscriptions
                         SET next_bill_date = $1, updated_at = NOW()
                         WHERE id = $2"
                    )
                    .bind(new_next_bill_date)
                    .bind(subscription.id)
                    .execute(&db)
                    .await;

                    if let Err(e) = update_result {
                        tracing::error!("Failed to update subscription next_bill_date: {}", e);
                    }
                } else {
                    tracing::error!(
                        "Failed to finalize invoice {}: {}",
                        invoice.id,
                        response.status()
                    );
                    failures += 1;
                }
            }
            Err(e) => {
                tracing::error!("Failed to call AR API to finalize invoice: {}", e);
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
         WHERE bill_run_id = $5"
    )
    .bind(subscriptions_processed)
    .bind(invoices_created)
    .bind(failures)
    .bind(execution_time)
    .bind(&bill_run_id)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update bill run record: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database_error".to_string(),
                message: "Failed to update bill run record".to_string(),
                details: None,
            }),
        )
    })?;

    // Emit subscriptions.billrun.completed event
    let payload = BillRunCompletedPayload {
        bill_run_id: bill_run_id.clone(),
        subscriptions_processed,
        invoices_created,
        failures,
        execution_time,
    };

    // Create envelope with platform-standard fields
    // TODO: Extract actual tenant_id when Subscriptions implements multi-tenancy
    let envelope = create_subscriptions_envelope(
        uuid::Uuid::new_v4(),
        "default".to_string(),
        None, // No correlation_id for now
        None, // No causation_id for now
        payload,
    );

    enqueue_event(&db, "billrun.completed", &envelope).await
    .map_err(|e| {
        tracing::error!("Failed to enqueue event: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "event_error".to_string(),
                message: "Failed to enqueue event".to_string(),
                details: None,
            }),
        )
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
fn calculate_next_bill_date(current_date: &NaiveDate, schedule: &String) -> NaiveDate {
    match schedule.as_str() {
        "weekly" => *current_date + chrono::Duration::weeks(1),
        "monthly" => {
            // Add one month
            let year = current_date.year();
            let month = current_date.month();
            let day = current_date.day();

            if month == 12 {
                NaiveDate::from_ymd_opt(year + 1, 1, day)
                    .unwrap_or_else(|| NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap())
            } else {
                NaiveDate::from_ymd_opt(year, month + 1, day)
                    .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap())
            }
        }
        _ => *current_date + chrono::Duration::weeks(4), // Default to 4 weeks for custom
    }
}
