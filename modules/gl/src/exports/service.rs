//! GL Export service — Guard → Mutation → Outbox
//!
//! Generates tenant-scoped exports in QuickBooks (IIF) or Xero (CSV) format.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::formats::{self, AccountRow, JournalEntryRow, JournalLineRow};
use crate::events::envelope::create_gl_envelope;
use crate::repos::outbox_repo;

/// Supported export formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    QuickBooks,
    Xero,
}

impl ExportFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::QuickBooks => "quickbooks",
            Self::Xero => "xero",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "quickbooks" | "qb" => Some(Self::QuickBooks),
            "xero" => Some(Self::Xero),
            _ => None,
        }
    }
}

/// Supported export types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportType {
    ChartOfAccounts,
    JournalEntries,
}

impl ExportType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChartOfAccounts => "chart_of_accounts",
            Self::JournalEntries => "journal_entries",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "chart_of_accounts" | "coa" => Some(Self::ChartOfAccounts),
            "journal_entries" | "journals" => Some(Self::JournalEntries),
            _ => None,
        }
    }
}

/// Request to create a GL export
#[derive(Debug)]
pub struct ExportRequest {
    pub tenant_id: String,
    pub format: ExportFormat,
    pub export_type: ExportType,
    pub idempotency_key: String,
    pub period_id: Option<Uuid>,
}

/// Result of a successful export
#[derive(Debug, serde::Serialize)]
pub struct ExportResult {
    pub export_id: Uuid,
    pub format: String,
    pub export_type: String,
    pub output: String,
    pub created_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("Invalid format: {0}")]
    InvalidFormat(String),

    #[error("Invalid export type: {0}")]
    InvalidExportType(String),

    #[error("Journal entry export requires period_id")]
    MissingPeriodId,

    #[error("Duplicate export: idempotency_key already used")]
    DuplicateIdempotencyKey,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Event payload for export events
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ExportEventPayload {
    pub export_id: String,
    pub tenant_id: String,
    pub format: String,
    pub export_type: String,
}

pub const EVENT_TYPE_EXPORT_REQUESTED: &str = "gl.export.requested";
pub const EVENT_TYPE_EXPORT_COMPLETED: &str = "gl.export.completed";

/// Execute a GL export (Guard → Mutation → Outbox)
pub async fn execute_export(
    pool: &PgPool,
    req: ExportRequest,
) -> Result<ExportResult, ExportError> {
    let export_id = Uuid::new_v4();
    let now = Utc::now();

    // --- GUARD: Check idempotency ---
    let existing: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, output FROM gl_exports WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .fetch_optional(pool)
    .await?;

    if let Some((existing_id, existing_output)) = existing {
        return Ok(ExportResult {
            export_id: existing_id,
            format: req.format.as_str().to_string(),
            export_type: req.export_type.as_str().to_string(),
            output: existing_output,
            created_at: now.to_rfc3339(),
        });
    }

    if req.export_type == ExportType::JournalEntries && req.period_id.is_none() {
        return Err(ExportError::MissingPeriodId);
    }

    // --- MUTATION: Generate export output ---
    let output = match req.export_type {
        ExportType::ChartOfAccounts => generate_coa_export(pool, &req).await?,
        ExportType::JournalEntries => generate_journal_export(pool, &req).await?,
    };

    // --- MUTATION + OUTBOX: Insert record and event atomically ---
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO gl_exports (id, tenant_id, idempotency_key, format, export_type,
                                status, output, period_id, created_at, completed_at)
        VALUES ($1, $2, $3, $4, $5, 'completed', $6, $7, $8, $8)
        "#,
    )
    .bind(export_id)
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(req.format.as_str())
    .bind(req.export_type.as_str())
    .bind(&output)
    .bind(req.period_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if is_unique_violation(&e) {
            ExportError::DuplicateIdempotencyKey
        } else {
            ExportError::Database(e)
        }
    })?;

    // --- OUTBOX: Emit export.completed event ---
    let event_payload = ExportEventPayload {
        export_id: export_id.to_string(),
        tenant_id: req.tenant_id.clone(),
        format: req.format.as_str().to_string(),
        export_type: req.export_type.as_str().to_string(),
    };

    let event_id = Uuid::new_v4();
    let correlation_id = export_id.to_string();
    let envelope = create_gl_envelope(
        event_id,
        req.tenant_id.clone(),
        EVENT_TYPE_EXPORT_COMPLETED.to_string(),
        correlation_id,
        None,
        "data_export".to_string(),
        event_payload,
    );

    let payload_json = serde_json::to_value(&envelope)
        .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

    outbox_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_EXPORT_COMPLETED,
        "gl_export",
        &export_id.to_string(),
        payload_json,
        "data_export",
    )
    .await?;

    tx.commit().await?;

    Ok(ExportResult {
        export_id,
        format: req.format.as_str().to_string(),
        export_type: req.export_type.as_str().to_string(),
        output,
        created_at: now.to_rfc3339(),
    })
}

// ---------------------------------------------------------------------------
// Data fetching + format rendering
// ---------------------------------------------------------------------------

async fn generate_coa_export(
    pool: &PgPool,
    req: &ExportRequest,
) -> Result<String, ExportError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT code, name, type::text
        FROM accounts
        WHERE tenant_id = $1 AND is_active = true
        ORDER BY code
        "#,
    )
    .bind(&req.tenant_id)
    .fetch_all(pool)
    .await?;

    let accounts: Vec<AccountRow> = rows
        .into_iter()
        .map(|(code, name, typ)| AccountRow {
            code,
            name,
            account_type: parse_account_type(&typ),
        })
        .collect();

    Ok(match req.format {
        ExportFormat::QuickBooks => formats::quickbooks_chart_of_accounts(&accounts),
        ExportFormat::Xero => formats::xero_chart_of_accounts(&accounts),
    })
}

async fn generate_journal_export(
    pool: &PgPool,
    req: &ExportRequest,
) -> Result<String, ExportError> {
    let period_id = req.period_id.ok_or(ExportError::MissingPeriodId)?;

    // Fetch journal entries in this period for this tenant
    let headers: Vec<(Uuid, String, Option<String>, String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT je.id, je.posted_at::text, je.description, je.currency,
               je.reference_id
        FROM journal_entries je
        INNER JOIN accounting_periods ap
            ON ap.tenant_id = je.tenant_id
            AND je.posted_at >= ap.period_start::timestamptz
            AND je.posted_at < (ap.period_end + interval '1 day')::timestamptz
        WHERE je.tenant_id = $1 AND ap.id = $2
        ORDER BY je.posted_at, je.created_at
        "#,
    )
    .bind(&req.tenant_id)
    .bind(period_id)
    .fetch_all(pool)
    .await?;

    let mut entries = Vec::new();
    for (entry_id, posted_at, description, _currency, reference_id) in &headers {
        let lines: Vec<(String, String, i64, i64, Option<String>)> = sqlx::query_as(
            r#"
            SELECT jl.account_ref, a.name, jl.debit_minor, jl.credit_minor, jl.memo
            FROM journal_lines jl
            INNER JOIN accounts a ON a.tenant_id = $2 AND a.code = jl.account_ref
            WHERE jl.journal_entry_id = $1
            ORDER BY jl.line_no
            "#,
        )
        .bind(entry_id)
        .bind(&req.tenant_id)
        .fetch_all(pool)
        .await?;

        entries.push(JournalEntryRow {
            posted_at: format_date_for_export(posted_at, req.format),
            description: description.clone().unwrap_or_default(),
            reference_id: reference_id.clone().unwrap_or_default(),
            lines: lines
                .into_iter()
                .map(|(code, name, dr, cr, memo)| JournalLineRow {
                    account_code: code,
                    account_name: name,
                    debit_minor: dr,
                    credit_minor: cr,
                    memo,
                })
                .collect(),
        });
    }

    Ok(match req.format {
        ExportFormat::QuickBooks => formats::quickbooks_journal_entries(&entries),
        ExportFormat::Xero => formats::xero_journal_entries(&entries),
    })
}

fn format_date_for_export(pg_timestamp: &str, format: ExportFormat) -> String {
    // pg_timestamp comes as "2024-02-15 00:00:00+00" or similar
    let date_part = pg_timestamp.split_whitespace().next().unwrap_or(pg_timestamp);
    match format {
        ExportFormat::QuickBooks => {
            // QuickBooks expects MM/DD/YYYY
            if let Some((y, rest)) = date_part.split_once('-') {
                if let Some((m, d)) = rest.split_once('-') {
                    return format!("{}/{}/{}", m, d, y);
                }
            }
            date_part.to_string()
        }
        ExportFormat::Xero => {
            // Xero expects DD/MM/YYYY
            if let Some((y, rest)) = date_part.split_once('-') {
                if let Some((m, d)) = rest.split_once('-') {
                    return format!("{}/{}/{}", d, m, y);
                }
            }
            date_part.to_string()
        }
    }
}

fn parse_account_type(s: &str) -> crate::repos::account_repo::AccountType {
    match s.to_lowercase().as_str() {
        "asset" => crate::repos::account_repo::AccountType::Asset,
        "liability" => crate::repos::account_repo::AccountType::Liability,
        "equity" => crate::repos::account_repo::AccountType::Equity,
        "revenue" => crate::repos::account_repo::AccountType::Revenue,
        "expense" => crate::repos::account_repo::AccountType::Expense,
        _ => crate::repos::account_repo::AccountType::Asset,
    }
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(ref db_err) = e {
        return db_err.code().as_deref() == Some("23505");
    }
    false
}
