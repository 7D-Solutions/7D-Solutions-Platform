use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};
use uuid::Uuid;

/// Journal entry with lines (for reading from DB)
#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub id: Uuid,
    pub tenant_id: String,
    pub source_module: String,
    pub source_event_id: Uuid,
    pub source_subject: String,
    pub posted_at: DateTime<Utc>,
    pub currency: String,
    pub description: Option<String>,
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    pub reverses_entry_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>, // Phase 16: Audit traceability
    pub created_at: DateTime<Utc>,
}

/// Journal line (for reading from DB)
#[derive(Debug, Clone)]
pub struct JournalLine {
    pub id: Uuid,
    pub journal_entry_id: Uuid,
    pub line_no: i32,
    pub account_ref: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub memo: Option<String>,
}

/// Fetch a journal entry by ID with its lines
pub async fn fetch_entry_with_lines(
    pool: &PgPool,
    entry_id: Uuid,
) -> Result<Option<(JournalEntry, Vec<JournalLine>)>, sqlx::Error> {
    // Fetch entry header
    let entry = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Uuid,
            String,
            DateTime<Utc>,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<Uuid>,
            Option<Uuid>,
            DateTime<Utc>,
        ),
    >(
        r#"
        SELECT id, tenant_id, source_module, source_event_id, source_subject,
               posted_at, currency, description, reference_type, reference_id,
               reverses_entry_id, correlation_id, created_at
        FROM journal_entries
        WHERE id = $1
        "#,
    )
    .bind(entry_id)
    .fetch_optional(pool)
    .await?;

    let Some(entry_row) = entry else {
        return Ok(None);
    };

    let entry = JournalEntry {
        id: entry_row.0,
        tenant_id: entry_row.1,
        source_module: entry_row.2,
        source_event_id: entry_row.3,
        source_subject: entry_row.4,
        posted_at: entry_row.5,
        currency: entry_row.6,
        description: entry_row.7,
        reference_type: entry_row.8,
        reference_id: entry_row.9,
        reverses_entry_id: entry_row.10,
        correlation_id: entry_row.11,
        created_at: entry_row.12,
    };

    // Fetch lines
    let lines = sqlx::query_as::<_, (Uuid, Uuid, i32, String, i64, i64, Option<String>)>(
        r#"
        SELECT id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo
        FROM journal_lines
        WHERE journal_entry_id = $1
        ORDER BY line_no
        "#,
    )
    .bind(entry_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| JournalLine {
        id: row.0,
        journal_entry_id: row.1,
        line_no: row.2,
        account_ref: row.3,
        debit_minor: row.4,
        credit_minor: row.5,
        memo: row.6,
    })
    .collect();

    Ok(Some((entry, lines)))
}

/// Insert a journal entry header with optional reverses_entry_id
pub async fn insert_entry_with_reversal(
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
    reverses_entry_id: Option<Uuid>,
    correlation_id: Option<Uuid>, // Phase 16: Audit traceability
) -> Result<Uuid, sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO journal_entries
            (id, tenant_id, source_module, source_event_id, source_subject,
             posted_at, currency, description, reference_type, reference_id, reverses_entry_id, correlation_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
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
    .bind(reverses_entry_id)
    .bind(correlation_id)
    .execute(&mut **tx)
    .await?;

    Ok(entry_id)
}

/// Insert a journal entry header and return the generated entry_id
/// (Wrapper without reverses_entry_id)
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
    correlation_id: Option<Uuid>, // Phase 16: Audit traceability
) -> Result<Uuid, sqlx::Error> {
    insert_entry_with_reversal(
        tx,
        entry_id,
        tenant_id,
        source_module,
        source_event_id,
        source_subject,
        posted_at,
        currency,
        description,
        reference_type,
        reference_id,
        None,
        correlation_id,
    )
    .await
}

/// Bulk insert journal lines for a journal entry
pub async fn bulk_insert_lines<T>(
    tx: &mut Transaction<'_, Postgres>,
    journal_entry_id: Uuid,
    lines: T,
) -> Result<(), sqlx::Error>
where
    T: AsRef<[JournalLineInsert]>,
{
    let lines = lines.as_ref();
    if lines.is_empty() {
        return Ok(());
    }

    let mut query_builder = QueryBuilder::<Postgres>::new(
        "INSERT INTO journal_lines \
         (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo) ",
    );
    query_builder.push_values(lines, |mut builder, line| {
        builder
            .push_bind(line.id)
            .push_bind(journal_entry_id)
            .push_bind(line.line_no)
            .push_bind(&line.account_ref)
            .push_bind(line.debit_minor)
            .push_bind(line.credit_minor)
            .push_bind(&line.memo);
    });

    query_builder.build().execute(&mut **tx).await?;

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
