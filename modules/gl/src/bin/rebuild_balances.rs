//! Rebuild balances tool
//!
//! This admin-only tool deterministically recomputes account balances from journal entries.
//! It provides audit integrity and recovery capability by treating journal entries as
//! the source of truth and rebuilding the materialized balance rollups.
//!
//! # Usage
//! ```bash
//! docker compose run --rm gl-rs ./rebuild_balances \
//!   --tenant TENANT_ID \
//!   --from 2026-01-01 \
//!   --to 2026-12-31
//! ```
//!
//! # Safety
//! - Operates on one tenant at a time (tenant safety)
//! - Uses batching to avoid long locks
//! - Runs in a transaction per period to ensure consistency
//! - Deterministic: same journal entries always produce same balances

use chrono::NaiveDate;
use sqlx::postgres::PgPoolOptions;
use std::env;
use uuid::Uuid;

/// Parse command-line arguments manually (no external crate needed)
struct Args {
    tenant_id: String,
    from_date: NaiveDate,
    to_date: NaiveDate,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let args: Vec<String> = env::args().collect();

        if args.len() != 7 {
            return Err(format!(
                "Usage: {} --tenant TENANT_ID --from YYYY-MM-DD --to YYYY-MM-DD",
                args.get(0).map(|s| s.as_str()).unwrap_or("rebuild_balances")
            ));
        }

        let mut tenant_id = None;
        let mut from_date = None;
        let mut to_date = None;

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--tenant" => {
                    if i + 1 < args.len() {
                        tenant_id = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        return Err("--tenant requires a value".to_string());
                    }
                }
                "--from" => {
                    if i + 1 < args.len() {
                        from_date = Some(
                            NaiveDate::parse_from_str(&args[i + 1], "%Y-%m-%d")
                                .map_err(|e| format!("Invalid --from date: {}", e))?
                        );
                        i += 2;
                    } else {
                        return Err("--from requires a value".to_string());
                    }
                }
                "--to" => {
                    if i + 1 < args.len() {
                        to_date = Some(
                            NaiveDate::parse_from_str(&args[i + 1], "%Y-%m-%d")
                                .map_err(|e| format!("Invalid --to date: {}", e))?
                        );
                        i += 2;
                    } else {
                        return Err("--to requires a value".to_string());
                    }
                }
                _ => return Err(format!("Unknown argument: {}", args[i])),
            }
        }

        Ok(Args {
            tenant_id: tenant_id.ok_or("--tenant is required")?,
            from_date: from_date.ok_or("--from is required")?,
            to_date: to_date.ok_or("--to is required")?,
        })
    }
}

/// Accounting period for grouping journal entries
#[derive(Debug, Clone)]
struct Period {
    id: Uuid,
    period_start: NaiveDate,
    period_end: NaiveDate,
}

/// Journal line for balance computation
#[derive(Debug, Clone)]
struct JournalLineData {
    account_ref: String,
    debit_minor: i64,
    credit_minor: i64,
}

/// Journal entry with its lines
#[derive(Debug, Clone)]
struct JournalEntryData {
    id: Uuid,
    currency: String,
    lines: Vec<JournalLineData>,
}

#[tokio::main]
async fn main() {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into())
        )
        .init();

    // Parse arguments
    let args = match Args::parse() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    tracing::info!(
        "Starting balance rebuild for tenant={}, from={}, to={}",
        args.tenant_id,
        args.from_date,
        args.to_date
    );

    // Connect to database
    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to database");

    tracing::info!("Connected to database");

    // Run migrations to ensure schema is up to date
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Find all periods that overlap with the date range
    let periods = fetch_periods(&pool, &args.tenant_id, args.from_date, args.to_date).await;

    if periods.is_empty() {
        tracing::warn!(
            "No accounting periods found for tenant={} in date range {}-{}",
            args.tenant_id,
            args.from_date,
            args.to_date
        );
        return;
    }

    tracing::info!("Found {} periods to rebuild", periods.len());

    // Rebuild balances for each period
    for period in periods {
        match rebuild_period_balances(&pool, &args.tenant_id, &period).await {
            Ok(count) => {
                tracing::info!(
                    "✓ Rebuilt {} balances for period {} ({} to {})",
                    count,
                    period.id,
                    period.period_start,
                    period.period_end
                );
            }
            Err(e) => {
                tracing::error!(
                    "✗ Failed to rebuild balances for period {}: {}",
                    period.id,
                    e
                );
                std::process::exit(1);
            }
        }
    }

    tracing::info!("✓ Balance rebuild complete");
}

/// Fetch all accounting periods that overlap with the date range
async fn fetch_periods(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Vec<Period> {
    sqlx::query_as::<_, (Uuid, NaiveDate, NaiveDate)>(
        r#"
        SELECT id, period_start, period_end
        FROM accounting_periods
        WHERE tenant_id = $1
          AND period_end >= $2
          AND period_start <= $3
        ORDER BY period_start
        "#,
    )
    .bind(tenant_id)
    .bind(from_date)
    .bind(to_date)
    .fetch_all(pool)
    .await
    .expect("Failed to fetch periods")
    .into_iter()
    .map(|(id, period_start, period_end)| Period {
        id,
        period_start,
        period_end,
    })
    .collect()
}

/// Rebuild balances for a single period
///
/// This function:
/// 1. Fetches all journal entries for the period
/// 2. Deletes existing balances for the period
/// 3. Recomputes balances from journal entries using delta computation
/// 4. Inserts new balances
///
/// All operations happen in a single transaction for consistency.
async fn rebuild_period_balances(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    period: &Period,
) -> Result<usize, Box<dyn std::error::Error>> {
    tracing::info!(
        "Rebuilding balances for period {} ({} to {})",
        period.id,
        period.period_start,
        period.period_end
    );

    // Start transaction
    let mut tx = pool.begin().await?;

    // Fetch all journal entries for this period
    let entries = fetch_journal_entries_for_period(
        &mut tx,
        tenant_id,
        period.period_start,
        period.period_end,
    )
    .await?;

    tracing::info!("Fetched {} journal entries for period", entries.len());

    // Delete existing balances for this period
    let deleted = sqlx::query(
        r#"
        DELETE FROM account_balances
        WHERE tenant_id = $1 AND period_id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(period.id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if deleted > 0 {
        tracing::info!("Deleted {} existing balance rows", deleted);
    }

    // Group entries by (account_code, currency) and compute totals
    use std::collections::HashMap;

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct BalanceKey {
        account_code: String,
        currency: String,
    }

    #[derive(Debug, Clone)]
    struct BalanceAccumulator {
        debit_total: i64,
        credit_total: i64,
        last_entry_id: Uuid,
    }

    let mut balances: HashMap<BalanceKey, BalanceAccumulator> = HashMap::new();

    for entry in &entries {
        for line in &entry.lines {
            let key = BalanceKey {
                account_code: line.account_ref.clone(),
                currency: entry.currency.clone(),
            };

            let acc = balances.entry(key).or_insert(BalanceAccumulator {
                debit_total: 0,
                credit_total: 0,
                last_entry_id: entry.id,
            });

            acc.debit_total += line.debit_minor;
            acc.credit_total += line.credit_minor;
            acc.last_entry_id = entry.id; // Track the last entry that affected this balance
        }
    }

    tracing::info!("Computed {} unique balances", balances.len());

    // Insert new balances
    let mut inserted = 0;
    for (key, acc) in balances {
        let net_balance = acc.debit_total - acc.credit_total;

        sqlx::query(
            r#"
            INSERT INTO account_balances (
                tenant_id,
                period_id,
                account_code,
                currency,
                debit_total_minor,
                credit_total_minor,
                net_balance_minor,
                last_journal_entry_id,
                updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
            "#,
        )
        .bind(tenant_id)
        .bind(period.id)
        .bind(&key.account_code)
        .bind(&key.currency)
        .bind(acc.debit_total)
        .bind(acc.credit_total)
        .bind(net_balance)
        .bind(acc.last_entry_id)
        .execute(&mut *tx)
        .await?;

        inserted += 1;
    }

    tracing::info!("Inserted {} balance rows", inserted);

    // Commit transaction
    tx.commit().await?;

    Ok(inserted)
}

/// Fetch all journal entries for a period with their lines
async fn fetch_journal_entries_for_period(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Result<Vec<JournalEntryData>, Box<dyn std::error::Error>> {
    // Fetch all journal entry IDs for the period
    let entry_ids = sqlx::query_as::<_, (Uuid, String)>(
        r#"
        SELECT id, currency
        FROM journal_entries
        WHERE tenant_id = $1
          AND posted_at >= $2
          AND posted_at < $3 + INTERVAL '1 day'
        ORDER BY posted_at, id
        "#,
    )
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .fetch_all(&mut **tx)
    .await?;

    // Fetch lines for each entry
    let mut entries = Vec::new();
    for (entry_id, currency) in entry_ids {
        let lines = sqlx::query_as::<_, (String, i64, i64)>(
            r#"
            SELECT account_ref, debit_minor, credit_minor
            FROM journal_lines
            WHERE journal_entry_id = $1
            ORDER BY line_no
            "#,
        )
        .bind(entry_id)
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|(account_ref, debit_minor, credit_minor)| JournalLineData {
            account_ref,
            debit_minor,
            credit_minor,
        })
        .collect();

        entries.push(JournalEntryData {
            id: entry_id,
            currency,
            lines,
        });
    }

    Ok(entries)
}
