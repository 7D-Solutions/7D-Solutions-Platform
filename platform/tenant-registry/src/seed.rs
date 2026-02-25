//! Tenant seed data
//!
//! Seeds required initial data into each module's database for a newly
//! provisioned tenant. Called as part of the provisioning sequence after
//! migrations have been applied.
//!
//! Seed contract:
//! - All operations are idempotent (safe to retry on partial failure)
//! - Each module function is independent — failures don't cascade
//! - No cross-module DB reads during seeding

use chrono::{Datelike, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Errors that can occur during tenant data seeding
#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("GL seed failed: {0}")]
    GlSeedFailed(sqlx::Error),

    #[error("AR seed failed: {0}")]
    ArSeedFailed(sqlx::Error),

    #[error("Subscriptions seed failed: {0}")]
    SubscriptionsSeedFailed(sqlx::Error),

    #[error("Identity seed failed: {0}")]
    IdentitySeedFailed(sqlx::Error),

    #[error("Invalid seed password: {0}")]
    InvalidSeedPassword(String),
}

pub type SeedResult<T> = Result<T, SeedError>;

// ============================================================================
// GL Module Seeding
// ============================================================================

/// Seed the GL module: create the current accounting period for the tenant.
///
/// Inserts a single accounting period covering the current calendar month.
/// Required for any financial posting operations.
/// Idempotent: skips if a period already exists overlapping this date range.
pub async fn seed_gl_module(gl_pool: &PgPool, tenant_id: Uuid) -> SeedResult<()> {
    let now = Utc::now().date_naive();
    let period_start = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .expect("valid period_start");
    // Last day of current month: first day of next month minus one day
    let period_end = if now.month() == 12 {
        NaiveDate::from_ymd_opt(now.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(now.year(), now.month() + 1, 1)
    }
    .expect("valid next month")
    .pred_opt()
    .expect("valid period_end");

    let tenant_id_str = tenant_id.to_string();

    // Use ON CONFLICT DO NOTHING — the EXCLUDE constraint prevents overlapping periods
    // for the same tenant, so concurrent inserts are safe.
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
        VALUES ($1, $2, $3, false)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(&tenant_id_str)
    .bind(period_start)
    .bind(period_end)
    .execute(gl_pool)
    .await
    .map_err(SeedError::GlSeedFailed)?;

    Ok(())
}

// ============================================================================
// AR Module Seeding
// ============================================================================

/// Seed the AR module: create dunning config (AR settings) for the tenant.
///
/// Inserts default dunning configuration: 3-day grace period, retry on
/// days 3, 7, and 14. Required for invoice lifecycle management.
/// Idempotent: ON CONFLICT DO NOTHING on unique (app_id).
pub async fn seed_ar_module(ar_pool: &PgPool, tenant_id: Uuid) -> SeedResult<()> {
    let app_id = tenant_id.to_string();
    let retry_schedule = serde_json::json!([3, 7, 14]);

    sqlx::query(
        r#"
        INSERT INTO ar_dunning_config (app_id, grace_period_days, retry_schedule_days, max_retry_attempts, updated_at)
        VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (app_id) DO NOTHING
        "#,
    )
    .bind(&app_id)
    .bind(3_i32)
    .bind(&retry_schedule)
    .bind(3_i32)
    .execute(ar_pool)
    .await
    .map_err(SeedError::ArSeedFailed)?;

    Ok(())
}

// ============================================================================
// Subscriptions Module Seeding
// ============================================================================

/// Seed the subscriptions module: create a default monthly plan for the tenant.
///
/// Inserts a "Standard Monthly" plan at $99.00/month in USD.
/// Required so that subscriptions can be created immediately after provisioning.
/// Idempotent: skips if a plan with this name already exists for the tenant.
pub async fn seed_subscriptions_module(
    subscriptions_pool: &PgPool,
    tenant_id: Uuid,
) -> SeedResult<()> {
    let tenant_id_str = tenant_id.to_string();

    // Check for existing default plan before inserting
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM subscription_plans WHERE tenant_id = $1 AND name = $2 LIMIT 1",
    )
    .bind(&tenant_id_str)
    .bind("Standard Monthly")
    .fetch_optional(subscriptions_pool)
    .await
    .map_err(SeedError::SubscriptionsSeedFailed)?;

    if existing.is_none() {
        sqlx::query(
            r#"
            INSERT INTO subscription_plans (tenant_id, name, description, schedule, price_minor, currency)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(&tenant_id_str)
        .bind("Standard Monthly")
        .bind("Default monthly subscription plan")
        .bind("monthly")
        .bind(9900_i64) // $99.00 in minor units (cents)
        .bind("usd")
        .execute(subscriptions_pool)
        .await
        .map_err(SeedError::SubscriptionsSeedFailed)?;
    }

    Ok(())
}

// ============================================================================
// Identity Module Seeding
// ============================================================================

/// Passwords that are never allowed as seed admin passwords.
const FORBIDDEN_SEED_PASSWORDS: &[&str] = &[
    "changeme123",
    "password",
    "password123",
    "admin",
    "admin123",
    "123456",
    "12345678",
    "secret",
    "letmein",
    "qwerty",
    "test",
    "1234",
];

/// Validate a candidate seed admin password against security requirements.
///
/// Returns `Ok(())` if the password is acceptable, or `Err(InvalidSeedPassword)`
/// if it is empty or matches a known-bad default. Extracted for unit-testability.
fn validate_seed_password(password: &str) -> SeedResult<()> {
    if password.is_empty() {
        return Err(SeedError::InvalidSeedPassword(
            "SEED_ADMIN_PASSWORD must not be empty".to_string(),
        ));
    }
    if FORBIDDEN_SEED_PASSWORDS.contains(&password) {
        return Err(SeedError::InvalidSeedPassword(format!(
            "'{}' is a known-bad default — set SEED_ADMIN_PASSWORD to a secure value",
            password
        )));
    }
    Ok(())
}

/// Seed the identity module: create a default admin user for the tenant.
///
/// Creates an admin credential record using PostgreSQL's pgcrypto crypt()
/// with bcrypt salt. The admin email is `admin@<tenant_id>.local`.
///
/// **Requires `SEED_ADMIN_PASSWORD` env var.** Seeding is refused if the
/// variable is unset, empty, or matches a known-bad default.
///
/// Idempotent: ON CONFLICT DO NOTHING on (tenant_id, email).
pub async fn seed_identity_module(
    identity_pool: &PgPool,
    tenant_id: Uuid,
) -> SeedResult<()> {
    let admin_password = std::env::var("SEED_ADMIN_PASSWORD").map_err(|_| {
        SeedError::InvalidSeedPassword(
            "SEED_ADMIN_PASSWORD env var is required but not set".to_string(),
        )
    })?;

    validate_seed_password(&admin_password)?;

    let admin_user_id = Uuid::new_v4();
    let admin_email = format!("admin@{}.local", tenant_id);

    // Use PostgreSQL's pgcrypto crypt() with bcrypt for password hashing
    sqlx::query(
        r#"
        INSERT INTO credentials (tenant_id, user_id, email, password_hash, is_active)
        VALUES ($1, $2, $3, crypt($4, gen_salt('bf')), true)
        ON CONFLICT (tenant_id, email) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .bind(admin_user_id)
    .bind(&admin_email)
    .bind(&admin_password)
    .execute(identity_pool)
    .await
    .map_err(SeedError::IdentitySeedFailed)?;

    Ok(())
}

// ============================================================================
// Aggregate Seed Function
// ============================================================================

/// Seed all modules for a newly provisioned tenant.
///
/// Runs GL, AR, subscriptions, and identity seed functions in sequence.
/// Each is independent and idempotent. Returns the first error encountered.
/// On retry, already-seeded modules are safely skipped.
pub async fn seed_all_modules(
    gl_pool: &PgPool,
    ar_pool: &PgPool,
    subscriptions_pool: &PgPool,
    identity_pool: &PgPool,
    tenant_id: Uuid,
) -> SeedResult<()> {
    seed_gl_module(gl_pool, tenant_id).await?;
    seed_ar_module(ar_pool, tenant_id).await?;
    seed_subscriptions_module(subscriptions_pool, tenant_id).await?;
    seed_identity_module(identity_pool, tenant_id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_dates_are_valid_for_mid_year() {
        let now = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let period_start = NaiveDate::from_ymd_opt(now.year(), now.month(), 1).unwrap();
        let period_end = NaiveDate::from_ymd_opt(now.year(), now.month() + 1, 1)
            .unwrap()
            .pred_opt()
            .unwrap();
        assert_eq!(period_start, NaiveDate::from_ymd_opt(2026, 6, 1).unwrap());
        assert_eq!(period_end, NaiveDate::from_ymd_opt(2026, 6, 30).unwrap());
        assert!(period_end > period_start);
    }

    #[test]
    fn period_dates_wrap_correctly_for_december() {
        let now = NaiveDate::from_ymd_opt(2026, 12, 10).unwrap();
        let period_start = NaiveDate::from_ymd_opt(now.year(), now.month(), 1).unwrap();
        let period_end = NaiveDate::from_ymd_opt(now.year() + 1, 1, 1)
            .unwrap()
            .pred_opt()
            .unwrap();
        assert_eq!(period_start, NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert_eq!(period_end, NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());
        assert!(period_end > period_start);
    }

    #[test]
    fn seed_error_messages_are_descriptive() {
        let err = SeedError::GlSeedFailed(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("GL seed failed"));

        let err = SeedError::ArSeedFailed(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("AR seed failed"));

        let err = SeedError::SubscriptionsSeedFailed(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("Subscriptions seed failed"));
    }

    // -------------------------------------------------------------------------
    // Seed password enforcement — tests call validate_seed_password directly
    // to avoid process-global env-var races between concurrent async tests.
    // -------------------------------------------------------------------------

    #[test]
    fn seed_identity_rejects_missing_password() {
        let err = validate_seed_password("")
            .expect_err("must fail on empty (missing) password");
        assert!(
            err.to_string().contains("must not be empty"),
            "error should describe empty password, got: {err}"
        );
    }

    #[test]
    fn seed_identity_rejects_forbidden_password() {
        let err = validate_seed_password("changeme123")
            .expect_err("must fail with known-bad password");
        assert!(
            err.to_string().contains("known-bad"),
            "error should mention known-bad, got: {err}"
        );
    }

    #[test]
    fn seed_identity_rejects_empty_password() {
        let err = validate_seed_password("")
            .expect_err("must fail with empty password");
        assert!(
            err.to_string().contains("must not be empty"),
            "error should describe empty password, got: {err}"
        );
    }

    #[test]
    fn seed_identity_accepts_strong_password() {
        validate_seed_password("Xk9#mP2$vQ8nRj5@")
            .expect("strong password must be accepted");
    }
}
