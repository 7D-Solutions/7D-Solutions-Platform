mod common;

use chrono::Utc;
use gl_rs::repos::journal_repo::{self, JournalLineInsert};
use serial_test::serial;
use sqlx::{Postgres, Transaction};
use std::time::Instant;
use uuid::Uuid;

async fn insert_entry(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    entry_id: Uuid,
) -> Result<(), sqlx::Error> {
    journal_repo::insert_entry(
        tx,
        entry_id,
        tenant_id,
        "bench",
        Uuid::new_v4(),
        "gl.events.posting.requested",
        Utc::now(),
        "USD",
        Some("journal batch insert benchmark"),
        None,
        None,
        None,
    )
    .await?;
    Ok(())
}

async fn legacy_bulk_insert_lines(
    tx: &mut Transaction<'_, Postgres>,
    journal_entry_id: Uuid,
    lines: &[JournalLineInsert],
) -> Result<(), sqlx::Error> {
    for line in lines {
        sqlx::query(
            r#"
            INSERT INTO journal_lines
                (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
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

fn build_lines(line_count: usize) -> Vec<JournalLineInsert> {
    let mut lines = Vec::with_capacity(line_count);

    for idx in 0..line_count {
        lines.push(JournalLineInsert {
            id: Uuid::new_v4(),
            line_no: (idx + 1) as i32,
            account_ref: if idx % 2 == 0 {
                "1100".to_string()
            } else {
                "4000".to_string()
            },
            debit_minor: if idx % 2 == 0 { 1_000 } else { 0 },
            credit_minor: if idx % 2 == 0 { 0 } else { 1_000 },
            memo: Some(format!("benchmark line {}", idx)),
        });
    }

    lines
}

#[tokio::test]
#[serial]
#[ignore]
async fn benchmark_bulk_insert_lines_against_legacy_loop() {
    let pool = common::get_test_pool().await;
    let tenant_id = "perf_tenant_journal_batch";
    common::cleanup_test_tenant(&pool, tenant_id).await;

    let lines = build_lines(2_000);
    let legacy_entry_id = Uuid::new_v4();
    let optimized_entry_id = Uuid::new_v4();

    let mut legacy_tx = pool.begin().await.expect("legacy transaction");
    insert_entry(&mut legacy_tx, tenant_id, legacy_entry_id)
        .await
        .expect("legacy entry");
    let legacy_start = Instant::now();
    legacy_bulk_insert_lines(&mut legacy_tx, legacy_entry_id, &lines)
        .await
        .expect("legacy line inserts");
    let legacy_elapsed = legacy_start.elapsed();
    legacy_tx.commit().await.expect("legacy commit");

    let optimized_lines = build_lines(2_000);
    let mut optimized_tx = pool.begin().await.expect("optimized transaction");
    insert_entry(&mut optimized_tx, tenant_id, optimized_entry_id)
        .await
        .expect("optimized entry");
    let optimized_start = Instant::now();
    journal_repo::bulk_insert_lines(&mut optimized_tx, optimized_entry_id, &optimized_lines)
        .await
        .expect("optimized line inserts");
    let optimized_elapsed = optimized_start.elapsed();
    optimized_tx.commit().await.expect("optimized commit");

    let legacy_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(legacy_entry_id)
    .fetch_one(&pool)
    .await
    .expect("legacy count");
    let optimized_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(optimized_entry_id)
    .fetch_one(&pool)
    .await
    .expect("optimized count");

    assert_eq!(legacy_count, 2_000);
    assert_eq!(optimized_count, 2_000);

    println!(
        "legacy={:?} optimized={:?} speedup={:.2}x",
        legacy_elapsed,
        optimized_elapsed,
        legacy_elapsed.as_secs_f64() / optimized_elapsed.as_secs_f64()
    );

    common::cleanup_test_tenant(&pool, tenant_id).await;
    pool.close().await;
}
