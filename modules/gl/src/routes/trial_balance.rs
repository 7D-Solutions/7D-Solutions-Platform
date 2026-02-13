//! Trial Balance API Routes
//!
//! Provides HTTP endpoints for querying trial balance reports.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    repos::account_repo::{AccountType, NormalBalance},
    services::trial_balance_service,
};

/// Query parameters for trial balance endpoint
#[derive(Debug, Deserialize)]
pub struct TrialBalanceQuery {
    /// Tenant identifier
    pub tenant_id: String,
    /// Accounting period ID
    pub period_id: Uuid,
    /// Optional currency filter (e.g., "USD", "EUR")
    pub currency: Option<String>,
}

/// Trial balance row response
#[derive(Debug, Serialize)]
pub struct TrialBalanceRowResponse {
    pub account_code: String,
    pub account_name: String,
    pub account_type: String,
    pub normal_balance: String,
    pub currency: String,
    pub debit_total_minor: i64,
    pub credit_total_minor: i64,
    pub net_balance_minor: i64,
}

/// Trial balance totals
#[derive(Debug, Serialize)]
pub struct TrialBalanceTotals {
    pub total_debits: i64,
    pub total_credits: i64,
    pub is_balanced: bool,
}

/// Trial balance response
#[derive(Debug, Serialize)]
pub struct TrialBalanceResponse {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub currency: Option<String>,
    pub rows: Vec<TrialBalanceRowResponse>,
    pub totals: TrialBalanceTotals,
}

/// Trial balance error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Handler for GET /api/trial-balance
///
/// Returns trial balance for a tenant and period with optional currency filter.
pub async fn get_trial_balance(
    State(pool): State<Arc<PgPool>>,
    Query(params): Query<TrialBalanceQuery>,
) -> Result<Json<TrialBalanceResponse>, TrialBalanceErrorResponse> {
    // Query trial balance
    let rows = trial_balance_service::get_trial_balance(
        &pool,
        &params.tenant_id,
        params.period_id,
        params.currency.as_deref(),
    )
    .await
    .map_err(|e| TrialBalanceErrorResponse {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Failed to fetch trial balance: {}", e),
    })?;

    // Transform to response DTOs and calculate totals
    let mut total_debits = 0i64;
    let mut total_credits = 0i64;

    let response_rows: Vec<TrialBalanceRowResponse> = rows
        .into_iter()
        .map(|row| {
            total_debits += row.debit_total_minor;
            total_credits += row.credit_total_minor;

            TrialBalanceRowResponse {
                account_code: row.account_code,
                account_name: row.account_name,
                account_type: format_account_type(&row.account_type),
                normal_balance: format_normal_balance(&row.normal_balance),
                currency: row.currency,
                debit_total_minor: row.debit_total_minor,
                credit_total_minor: row.credit_total_minor,
                net_balance_minor: row.net_balance_minor,
            }
        })
        .collect();

    // Check if trial balance is balanced
    let is_balanced = total_debits == total_credits;

    Ok(Json(TrialBalanceResponse {
        tenant_id: params.tenant_id,
        period_id: params.period_id,
        currency: params.currency,
        rows: response_rows,
        totals: TrialBalanceTotals {
            total_debits,
            total_credits,
            is_balanced,
        },
    }))
}

/// Format AccountType enum for JSON response
fn format_account_type(account_type: &AccountType) -> String {
    match account_type {
        AccountType::Asset => "asset".to_string(),
        AccountType::Liability => "liability".to_string(),
        AccountType::Equity => "equity".to_string(),
        AccountType::Revenue => "revenue".to_string(),
        AccountType::Expense => "expense".to_string(),
    }
}

/// Format NormalBalance enum for JSON response
fn format_normal_balance(normal_balance: &NormalBalance) -> String {
    match normal_balance {
        NormalBalance::Debit => "debit".to_string(),
        NormalBalance::Credit => "credit".to_string(),
    }
}

/// Error response wrapper for proper HTTP error handling
#[derive(Debug)]
pub struct TrialBalanceErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for TrialBalanceErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_account_type() {
        assert_eq!(format_account_type(&AccountType::Asset), "asset");
        assert_eq!(format_account_type(&AccountType::Revenue), "revenue");
    }

    #[test]
    fn test_format_normal_balance() {
        assert_eq!(format_normal_balance(&NormalBalance::Debit), "debit");
        assert_eq!(format_normal_balance(&NormalBalance::Credit), "credit");
    }
}
