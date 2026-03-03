//! Shared helpers for GL export tests

use sqlx::PgPool;
use uuid::Uuid;

/// Setup accounts and journal entries for export testing. Returns period_id.
pub async fn setup_export_data(pool: &PgPool, tenant_id: &str) -> Uuid {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES
            ($1, $2, '1100', 'Accounts Receivable', 'asset'::account_type, 'debit'::normal_balance, true, NOW()),
            ($3, $2, '4000', 'Revenue', 'revenue'::account_type, 'credit'::normal_balance, true, NOW()),
            ($4, $2, '5000', 'Cost of Goods Sold', 'expense'::account_type, 'debit'::normal_balance, true, NOW())
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("setup accounts");

    let period_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
         VALUES ($1, $2, '2024-03-01', '2024-03-31', false, NOW())",
    )
    .bind(period_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("setup period");

    let entry_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject,
                                     posted_at, currency, description, reference_type, reference_id, created_at)
        VALUES ($1, $2, 'ar', $3, 'gl.posting.requested',
                '2024-03-15T00:00:00Z', 'USD', 'Invoice #1001', 'AR_INVOICE', 'inv-1001', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("setup journal entry");

    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES ($1, $2, 1, '1100', 250000, 0, 'AR debit'), ($3, $2, 2, '4000', 0, 250000, 'Revenue credit')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("setup journal lines");

    period_id
}

pub async fn cleanup_export_data(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM events_outbox WHERE event_type = 'gl.export.completed'")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM gl_exports WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}
