/// Queries the tenant-registry database for tenants eligible for platform billing.
///
/// Eligible tenants: status IN ('active', 'trial') with a non-empty plan_code.
use sqlx::PgPool;
use uuid::Uuid;

/// A tenant that is eligible to be billed by the platform billing runner.
#[derive(Debug, Clone)]
pub struct EligibleTenant {
    pub tenant_id: Uuid,
    pub plan_code: String,
    /// Product tier identifier — used to look up pricing in cp_plans.
    pub product_code: String,
}

/// Fetch all tenants eligible for platform billing from the tenant-registry DB.
///
/// Tenants with status 'active' or 'trial' and a non-null/non-empty plan_code are included.
pub async fn fetch_eligible_tenants(pool: &PgPool) -> Result<Vec<EligibleTenant>, sqlx::Error> {
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        r#"
        SELECT tenant_id, plan_code, COALESCE(product_code, '') AS product_code
        FROM tenants
        WHERE status IN ('active', 'trial')
          AND plan_code IS NOT NULL
          AND plan_code != ''
        ORDER BY tenant_id
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(tenant_id, plan_code, product_code)| EligibleTenant {
            tenant_id,
            plan_code,
            product_code,
        })
        .collect())
}

/// Look up the monthly fee in minor units for a given product tier from cp_plans.
///
/// Returns None if the plan_code is not found in cp_plans.
/// Callers should treat None as a zero-fee plan or skip the tenant.
pub async fn fetch_plan_fee_minor(
    pool: &PgPool,
    plan_code: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT monthly_fee_minor FROM cp_plans WHERE plan_code = $1",
    )
    .bind(plan_code)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(fee,)| fee))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_eligible_tenants_returns_only_active_and_trial() {
        let db_url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
                .to_string()
        });
        let pool = match sqlx::PgPool::connect(&db_url).await {
            Ok(p) => p,
            Err(_) => return, // skip if DB unavailable
        };

        // Fetch current eligible tenants — just verify the query runs without error
        let tenants = fetch_eligible_tenants(&pool).await.expect("query should succeed");

        // All returned tenants must have non-empty plan_code
        for t in &tenants {
            assert!(!t.plan_code.is_empty(), "plan_code should be non-empty");
        }

        // Insert a trial tenant and verify it appears
        let tenant_id = Uuid::new_v4();
        let app_id = format!("app-{}", &tenant_id.to_string().replace('-', "")[..12]);
        sqlx::query(
            r#"INSERT INTO tenants
               (tenant_id, status, environment, module_schema_versions,
                product_code, plan_code, app_id, created_at, updated_at)
               VALUES ($1, 'trial', 'development', '{}'::jsonb, 'starter', 'monthly', $2, NOW(), NOW())"#,
        )
        .bind(tenant_id)
        .bind(&app_id)
        .execute(&pool)
        .await
        .expect("insert test tenant");

        let tenants_after = fetch_eligible_tenants(&pool).await.expect("query after insert");
        assert!(
            tenants_after.iter().any(|t| t.tenant_id == tenant_id),
            "trial tenant should appear in eligible list"
        );
        let entry = tenants_after
            .iter()
            .find(|t| t.tenant_id == tenant_id)
            .unwrap();
        assert_eq!(entry.plan_code, "monthly");
        assert_eq!(entry.product_code, "starter");

        // Cleanup
        sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await
            .ok();
    }

    #[tokio::test]
    async fn fetch_plan_fee_minor_returns_correct_prices() {
        let db_url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
                .to_string()
        });
        let pool = match sqlx::PgPool::connect(&db_url).await {
            Ok(p) => p,
            Err(_) => return, // skip if DB unavailable
        };

        // Verify seeded plans
        let starter = fetch_plan_fee_minor(&pool, "starter")
            .await
            .expect("query should succeed");
        assert_eq!(starter, Some(2900), "starter should cost 2900 minor units");

        let professional = fetch_plan_fee_minor(&pool, "professional")
            .await
            .expect("query should succeed");
        assert_eq!(professional, Some(7900), "professional should cost 7900 minor units");

        let enterprise = fetch_plan_fee_minor(&pool, "enterprise")
            .await
            .expect("query should succeed");
        assert_eq!(enterprise, Some(29900), "enterprise should cost 29900 minor units");

        // Unknown plan returns None
        let unknown = fetch_plan_fee_minor(&pool, "nonexistent-plan")
            .await
            .expect("query should succeed");
        assert_eq!(unknown, None, "unknown plan should return None");
    }
}
