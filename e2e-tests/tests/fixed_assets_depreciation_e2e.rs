//! E2E: Fixed assets depreciation run — asset created, depreciation run fires GL journal entry (bd-exxn)
//!
//! Proves the fixed asset depreciation lifecycle creates correct GL journal entries:
//!
//! 1. Create category + asset: $12,000 cost, 60-month life, in-service 2025-02-01.
//! 2. Generate depreciation schedule: 60 monthly periods, $200/month (20,000 minor units).
//! 3. Run depreciation as_of_date 2026-01-31: posts 12 periods, total $2,400 (240,000).
//! 4. Process GL entries from outbox: DR 6100 (expense), CR 1510 (accum depr), $200/period.
//! 5. Verify 12 GL journal entries created, all balanced (DR == CR per entry).
//! 6. Verify total debit = 240,000 minor units across all entries.
//! 7. Verify net book value: period-12 remaining = 960,000 (cost − accumulated).
//! 8. Idempotency: run again → 0 periods posted; GL replay → DuplicateEvent; no new entries.
//!
//! ## Services required
//! - fixed-assets postgres at localhost:5445
//! - gl postgres at localhost:5438
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- fixed_assets_depreciation_e2e --nocapture
//! ```

mod common;

use chrono::NaiveDate;
use common::wait_for_db_ready;
use fixed_assets::domain::depreciation::{
    CreateRunRequest, DepreciationService, GenerateScheduleRequest,
};
use gl_rs::consumer::fixed_assets_depreciation::{
    process_depreciation_entry_posting, DepreciationGlEntry,
};
use gl_rs::services::journal_service::JournalError;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Constants
// ============================================================================

const TENANT: &str = "e2e-fa-depr-gl";

/// Asset: $12,000 cost (1,200,000 minor units), 60-month life, $0 salvage.
/// Monthly depreciation: 1,200,000 / 60 = 20,000 minor units.
const COST_MINOR: i64 = 1_200_000;
const USEFUL_LIFE_MONTHS: i32 = 60;
const MONTHLY_DEPR: i64 = 20_000;

/// In-service 2025-02-01 → periods 1-12 end 2025-02-28 through 2026-01-31.
/// as_of_date 2026-01-31 posts exactly 12 complete periods.
const IN_SERVICE: &str = "2025-02-01";
const AS_OF_DATE: &str = "2026-01-31";

const EXPECTED_PERIODS: i32 = 12;
const EXPECTED_TOTAL: i64 = MONTHLY_DEPR * EXPECTED_PERIODS as i64; // 240,000
const EXPECTED_NBV: i64 = COST_MINOR - EXPECTED_TOTAL; // 960,000

// ============================================================================
// Database connection helpers
// ============================================================================

async fn fa_pool() -> PgPool {
    let url = std::env::var("FA_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db"
            .to_string()
    });
    wait_for_db_ready("fixed-assets", &url).await
}

async fn gl_pool() -> PgPool {
    let url = std::env::var("GL_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://gl_user:gl_pass@localhost:5438/gl_db".to_string()
    });
    wait_for_db_ready("gl", &url).await
}

// ============================================================================
// Fixed assets migration helper (idempotent via advisory lock)
// ============================================================================

const FA_MIGRATION_LOCK: i64 = 7_445_319_826_i64; // distinct from fixed_assets_end_to_end.rs

async fn ensure_fa_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(FA_MIGRATION_LOCK)
        .execute(pool)
        .await
        .expect("advisory lock failed");

    let migrations = [
        include_str!("../../modules/fixed-assets/db/migrations/20260218200001_create_asset_categories.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200002_create_assets.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200003_create_depreciation_schedules.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200004_create_depreciation_runs.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200005_create_disposals.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200006_create_outbox_and_idempotency.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200007_create_ap_capitalizations.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200008_asset_status_to_text.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200009_run_status_to_text.sql"),
    ];
    for sql in migrations {
        let _ = sqlx::raw_sql(sql).execute(pool).await;
    }

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(FA_MIGRATION_LOCK)
        .execute(pool)
        .await
        .expect("advisory unlock failed");
}

// ============================================================================
// Setup helpers — Fixed Assets
// ============================================================================

/// Insert category (expense_ref=6100, accum_ref=1510) + asset (cost 1,200,000, life 60 months).
/// Returns (category_id, asset_id).
async fn create_category_and_asset(fa: &PgPool) -> (Uuid, Uuid) {
    let cat_id = Uuid::new_v4();
    let code = format!("DEPR-{}", &cat_id.to_string()[..8]);
    sqlx::query(
        r#"
        INSERT INTO fa_categories
            (id, tenant_id, code, name,
             default_method, default_useful_life_months, default_salvage_pct_bp,
             asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
             is_active, created_at, updated_at)
        VALUES ($1,$2,$3,$4,'straight_line',60,0,'1500','6100','1510',TRUE,NOW(),NOW())
        "#,
    )
    .bind(cat_id)
    .bind(TENANT)
    .bind(&code)
    .bind(format!("Category {}", code))
    .execute(fa)
    .await
    .expect("insert category");

    let asset_id = Uuid::new_v4();
    let tag = format!("FA-{}", &asset_id.to_string()[..8]);
    let in_service = NaiveDate::parse_from_str(IN_SERVICE, "%Y-%m-%d").unwrap();

    sqlx::query(
        r#"
        INSERT INTO fa_assets
            (id, tenant_id, category_id, asset_tag, name,
             status, acquisition_date, in_service_date,
             acquisition_cost_minor, currency,
             depreciation_method, useful_life_months, salvage_value_minor,
             accum_depreciation_minor, net_book_value_minor,
             created_at, updated_at)
        VALUES ($1,$2,$3,$4,$5,'active',$6,$6,$7,'usd','straight_line',$8,0,0,$7,NOW(),NOW())
        "#,
    )
    .bind(asset_id)
    .bind(TENANT)
    .bind(cat_id)
    .bind(&tag)
    .bind("Server Rack E2E")
    .bind(in_service)
    .bind(COST_MINOR)
    .bind(USEFUL_LIFE_MONTHS)
    .execute(fa)
    .await
    .expect("insert asset");

    (cat_id, asset_id)
}

// ============================================================================
// Setup helpers — GL
// ============================================================================

/// Ensure GL accounts 6100 (depreciation expense) and 1510 (accum depreciation) exist.
async fn ensure_gl_accounts(gl: &PgPool) {
    for (code, name, atype, nb) in [
        ("6100", "Depreciation Expense", "expense", "debit"),
        ("1510", "Accumulated Depreciation", "asset", "credit"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO accounts
                (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
            VALUES (gen_random_uuid(), $1, $2, $3, $4::account_type, $5::normal_balance, TRUE, NOW())
            ON CONFLICT (tenant_id, code) DO NOTHING
            "#,
        )
        .bind(TENANT)
        .bind(code)
        .bind(name)
        .bind(atype)
        .bind(nb)
        .execute(gl)
        .await
        .expect("ensure GL account");
    }
}

/// Ensure a broad open accounting period covering 2025-01-01 to 2026-12-31.
/// This spans all 12 depreciation period_ends (2025-02-28 through 2026-01-31).
async fn ensure_gl_period(gl: &PgPool) {
    sqlx::query(
        r#"
        INSERT INTO accounting_periods
            (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES (gen_random_uuid(), $1, '2025-01-01', '2026-12-31', FALSE, NOW())
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(TENANT)
    .execute(gl)
    .await
    .expect("ensure GL period");
}

// ============================================================================
// Cleanup helpers
// ============================================================================

async fn cleanup_fa(pool: &PgPool) {
    for q in [
        "DELETE FROM fa_depreciation_schedules WHERE tenant_id = $1",
        "DELETE FROM fa_depreciation_runs     WHERE tenant_id = $1",
        "DELETE FROM fa_events_outbox          WHERE tenant_id = $1",
        "DELETE FROM fa_assets                 WHERE tenant_id = $1",
        "DELETE FROM fa_categories             WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(TENANT).execute(pool).await.ok();
    }
}

async fn cleanup_gl(pool: &PgPool) {
    for q in [
        "DELETE FROM processed_events WHERE event_id IN \
         (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM journal_lines WHERE journal_entry_id IN \
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM journal_entries   WHERE tenant_id = $1",
        "DELETE FROM account_balances  WHERE tenant_id = $1",
        "DELETE FROM accounting_periods WHERE tenant_id = $1",
        "DELETE FROM accounts           WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(TENANT).execute(pool).await.ok();
    }
}

// ============================================================================
// Helper: extract GL entries from FA outbox event payload
// ============================================================================

/// Read the `gl_entries` field from the FA outbox event for a completed run.
/// The FA module embeds GL entry data in the `depreciation_run_completed` event
/// so the GL consumer can post balanced journal entries from the NATS payload.
async fn read_gl_entries_from_outbox(fa: &PgPool, run_id: Uuid) -> Vec<DepreciationGlEntry> {
    let (payload,): (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM fa_events_outbox \
         WHERE aggregate_id = $1 AND event_type = 'depreciation_run_completed'",
    )
    .bind(run_id.to_string())
    .fetch_one(fa)
    .await
    .expect("outbox event for depreciation run");

    serde_json::from_value(payload["gl_entries"].clone())
        .expect("deserialize gl_entries from outbox payload")
}

// ============================================================================
// Test 1: Happy path — depreciation run creates balanced GL journal entries
// ============================================================================

/// Full depreciation lifecycle with GL verification:
///
/// 1. Create asset ($12,000 cost, 60-month life, in-service 2025-02-01).
/// 2. Generate schedule → 60 periods, $200/month each.
/// 3. Run depreciation as_of_date 2026-01-31 → posts 12 periods, total $2,400.
/// 4. Read GL entries from FA outbox event (mimics real NATS consumer flow).
/// 5. Call GL consumer's `process_depreciation_entry_posting` for each entry.
/// 6. Verify 12 journal entries created, total DR 6100 = 240,000 = CR 1510.
/// 7. Verify net book value via last posted schedule period = 960,000.
#[tokio::test]
#[serial]
async fn test_depreciation_run_gl_entry_created() {
    let fa = fa_pool().await;
    let gl = gl_pool().await;

    ensure_fa_migrations(&fa).await;
    cleanup_fa(&fa).await;
    cleanup_gl(&gl).await;
    ensure_gl_accounts(&gl).await;
    ensure_gl_period(&gl).await;

    // ── 1. Create category + asset ────────────────────────────────────────────
    let (_cat_id, asset_id) = create_category_and_asset(&fa).await;
    println!("✓ Created asset {}", asset_id);

    // ── 2. Generate depreciation schedule ────────────────────────────────────
    let schedules = DepreciationService::generate_schedule(
        &fa,
        &GenerateScheduleRequest {
            tenant_id: TENANT.into(),
            asset_id,
        },
    )
    .await
    .expect("generate schedule");

    assert_eq!(
        schedules.len(),
        USEFUL_LIFE_MONTHS as usize,
        "schedule must have {} periods (full 60-month life)",
        USEFUL_LIFE_MONTHS
    );
    assert_eq!(
        schedules[0].depreciation_amount_minor,
        MONTHLY_DEPR,
        "period 1 depreciation must be {} minor units",
        MONTHLY_DEPR
    );
    println!("✓ Generated {} schedule periods, {} minor/period", schedules.len(), MONTHLY_DEPR);

    // ── 3. Run depreciation (as_of_date = 2026-01-31) ────────────────────────
    let as_of = NaiveDate::parse_from_str(AS_OF_DATE, "%Y-%m-%d").unwrap();
    let run = DepreciationService::run(
        &fa,
        &CreateRunRequest {
            tenant_id: TENANT.into(),
            as_of_date: as_of,
            currency: None,
            created_by: Some("e2e-test".into()),
        },
    )
    .await
    .expect("depreciation run");

    assert_eq!(run.status, "completed", "run must complete");
    assert_eq!(
        run.periods_posted, EXPECTED_PERIODS,
        "12 monthly periods (2025-02 through 2026-01) must be posted"
    );
    assert_eq!(
        run.total_depreciation_minor, EXPECTED_TOTAL,
        "total = 12 × 20,000 = 240,000 minor units"
    );
    assert_eq!(run.assets_processed, 1, "one asset processed");
    println!(
        "✓ Depreciation run {}: {} periods, {} minor units total",
        run.id, run.periods_posted, run.total_depreciation_minor
    );

    // ── 4. Read GL entries from outbox (mirrors NATS consumer path) ───────────
    let gl_entries = read_gl_entries_from_outbox(&fa, run.id).await;
    assert_eq!(
        gl_entries.len(),
        EXPECTED_PERIODS as usize,
        "outbox must embed {} GL entries (one per posted period)",
        EXPECTED_PERIODS
    );
    println!("✓ {} GL entries in outbox payload", gl_entries.len());

    // ── 5. GL consumer: post journal entries ──────────────────────────────────
    for entry in &gl_entries {
        process_depreciation_entry_posting(&gl, TENANT, entry)
            .await
            .unwrap_or_else(|e| panic!(
                "GL posting failed for entry {}: {:?}", entry.entry_id, e
            ));
    }
    println!("✓ All {} GL entries posted", gl_entries.len());

    // ── 6a. Verify journal entry count ────────────────────────────────────────
    let (je_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1",
    )
    .bind(TENANT)
    .fetch_one(&gl)
    .await
    .expect("count journal entries");
    assert_eq!(
        je_count,
        EXPECTED_PERIODS as i64,
        "exactly {} journal entries (one per period)",
        EXPECTED_PERIODS
    );
    println!("✓ {} journal entries in GL", je_count);

    // ── 6b. Verify total debit on 6100 (depreciation expense) ─────────────────
    // Cast to BIGINT: PostgreSQL SUM(bigint) may return NUMERIC in some contexts.
    let (total_debit_6100,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(jl.debit_minor), 0)::BIGINT \
         FROM journal_lines jl \
         JOIN journal_entries je ON je.id = jl.journal_entry_id \
         WHERE je.tenant_id = $1 AND jl.account_ref = '6100'",
    )
    .bind(TENANT)
    .fetch_one(&gl)
    .await
    .expect("sum 6100 debits");
    assert_eq!(
        total_debit_6100, EXPECTED_TOTAL,
        "total DR 6100 must equal {} minor units (12 × 20,000)",
        EXPECTED_TOTAL
    );
    println!("✓ DR 6100 total = {} minor units", total_debit_6100);

    // ── 6c. Verify total credit on 1510 (accumulated depreciation) ────────────
    let (total_credit_1510,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(jl.credit_minor), 0)::BIGINT \
         FROM journal_lines jl \
         JOIN journal_entries je ON je.id = jl.journal_entry_id \
         WHERE je.tenant_id = $1 AND jl.account_ref = '1510'",
    )
    .bind(TENANT)
    .fetch_one(&gl)
    .await
    .expect("sum 1510 credits");
    assert_eq!(
        total_credit_1510, EXPECTED_TOTAL,
        "total CR 1510 must equal {} minor units (balanced)",
        EXPECTED_TOTAL
    );
    println!("✓ CR 1510 total = {} minor units — GL balanced", total_credit_1510);

    // ── 7. Verify net book value via last posted schedule period ──────────────
    let (last_posted_nbv,): (i64,) = sqlx::query_as(
        "SELECT remaining_book_value_minor \
         FROM fa_depreciation_schedules \
         WHERE asset_id = $1 AND tenant_id = $2 AND is_posted = TRUE \
         ORDER BY period_number DESC LIMIT 1",
    )
    .bind(asset_id)
    .bind(TENANT)
    .fetch_one(&fa)
    .await
    .expect("last posted schedule period");
    assert_eq!(
        last_posted_nbv, EXPECTED_NBV,
        "after 12 months: NBV = cost {} - accumulated {} = {}",
        COST_MINOR, EXPECTED_TOTAL, EXPECTED_NBV
    );
    println!(
        "✓ Net book value after 12 months = {} minor units (cost {} − accum {})",
        last_posted_nbv, COST_MINOR, EXPECTED_TOTAL
    );

    cleanup_fa(&fa).await;
    cleanup_gl(&gl).await;
    println!("✅ test_depreciation_run_gl_entry_created PASS");
}

// ============================================================================
// Test 2: Idempotency — run again → no duplicate GL entries
// ============================================================================

/// Idempotency proof:
///
/// 1. Run depreciation → 12 periods posted, 12 GL journal entries created.
/// 2. Run depreciation again for the same as_of_date → 0 periods posted (FA guard).
/// 3. Replay GL entries from run 1 → DuplicateEvent for every entry.
/// 4. Verify GL still has exactly 12 journal entries — no duplicates.
#[tokio::test]
#[serial]
async fn test_depreciation_run_idempotent_no_duplicate_gl() {
    let fa = fa_pool().await;
    let gl = gl_pool().await;

    ensure_fa_migrations(&fa).await;
    cleanup_fa(&fa).await;
    cleanup_gl(&gl).await;
    ensure_gl_accounts(&gl).await;
    ensure_gl_period(&gl).await;

    let (_cat_id, asset_id) = create_category_and_asset(&fa).await;

    // Generate schedule
    DepreciationService::generate_schedule(
        &fa,
        &GenerateScheduleRequest {
            tenant_id: TENANT.into(),
            asset_id,
        },
    )
    .await
    .expect("generate schedule");

    let as_of = NaiveDate::parse_from_str(AS_OF_DATE, "%Y-%m-%d").unwrap();
    let run_req = CreateRunRequest {
        tenant_id: TENANT.into(),
        as_of_date: as_of,
        currency: None,
        created_by: Some("e2e-test".into()),
    };

    // ── Run 1: posts 12 periods ───────────────────────────────────────────────
    let run1 = DepreciationService::run(&fa, &run_req).await.expect("run 1");
    assert_eq!(run1.periods_posted, EXPECTED_PERIODS, "run 1 must post 12 periods");

    let gl_entries = read_gl_entries_from_outbox(&fa, run1.id).await;
    for entry in &gl_entries {
        process_depreciation_entry_posting(&gl, TENANT, entry)
            .await
            .expect("GL entry posting run 1");
    }
    println!(
        "✓ Run 1: {} periods posted, {} GL entries created",
        run1.periods_posted, gl_entries.len()
    );

    // ── Run 2: same as_of_date → FA guard prevents double-posting ────────────
    let run2 = DepreciationService::run(&fa, &run_req).await.expect("run 2");
    assert_eq!(
        run2.periods_posted, 0,
        "second run for same period must post 0 entries (all already marked is_posted=TRUE)"
    );
    assert_eq!(run2.total_depreciation_minor, 0, "zero depreciation in second run");
    println!("✓ Run 2: 0 periods posted — FA idempotency guard working");

    // ── GL replay: re-processing same entry_ids → DuplicateEvent ─────────────
    let mut duplicate_count = 0;
    for entry in &gl_entries {
        let result = process_depreciation_entry_posting(&gl, TENANT, entry).await;
        match result {
            Err(JournalError::DuplicateEvent(_)) => {
                duplicate_count += 1;
            }
            Ok(id) => panic!(
                "Expected DuplicateEvent on replay for entry {}, got Ok({})",
                entry.entry_id, id
            ),
            Err(e) => panic!(
                "Expected DuplicateEvent on replay for entry {}, got {:?}",
                entry.entry_id, e
            ),
        }
    }
    assert_eq!(
        duplicate_count,
        EXPECTED_PERIODS as usize,
        "all {} entries must return DuplicateEvent on replay (idempotent GL consumer)",
        EXPECTED_PERIODS
    );
    println!("✓ All {} GL entries returned DuplicateEvent on replay", duplicate_count);

    // ── Verify exactly 12 journal entries still — no duplicates ──────────────
    let (je_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1",
    )
    .bind(TENANT)
    .fetch_one(&gl)
    .await
    .expect("count journal entries after replay");
    assert_eq!(
        je_count,
        EXPECTED_PERIODS as i64,
        "must still have exactly {} journal entries — no duplicates on replay",
        EXPECTED_PERIODS
    );
    println!(
        "✓ GL has exactly {} journal entries — no duplicates created by replay",
        je_count
    );

    cleanup_fa(&fa).await;
    cleanup_gl(&gl).await;
    println!("✅ test_depreciation_run_idempotent_no_duplicate_gl PASS");
}
