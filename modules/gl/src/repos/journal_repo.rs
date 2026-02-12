use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Insert a journal entry header and return the generated entry_id
pub async fn insert_entry(
    tx: &mut Transaction<'_, Postgres>,
    entry_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    source_event_id: Uuid,
    source_subject: &str,
    posted_at: DateTime<Utc>,
    currency: &str,
    description: Option<&str>,
    reference_type: Option<&str>,
    reference_id: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO journal_entries
            (id, tenant_id, source_module, source_event_id, source_subject,
             posted_at, currency, description, reference_type, reference_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(source_module)
    .bind(source_event_id)
    .bind(source_subject)
    .bind(posted_at)
    .bind(currency)
    .bind(description)
    .bind(reference_type)
    .bind(reference_id)
    .execute(&mut **tx)
    .await?;

    Ok(entry_id)
}

/// Bulk insert journal lines for a journal entry
pub async fn bulk_insert_lines(
    tx: &mut Transaction<'_, Postgres>,
    journal_entry_id: Uuid,
    lines: Vec<JournalLineInsert>,
) -> Result<(), sqlx::Error> {
    for line in lines {
        sqlx::query(
            r#"
            INSERT INTO journal_lines
                (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#
        )
        .bind(line.id)
        .bind(journal_entry_id)
        .bind(line.line_no)
        .bind(&line.account_ref)
        .bind(line.debit_minor)
        .bind(line.credit_minor)
        .bind(&line.memo)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

/// Struct for inserting a journal line
#[derive(Debug, Clone)]
pub struct JournalLineInsert {
    pub id: Uuid,
    pub line_no: i32,
    pub account_ref: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub memo: Option<String>,
}
