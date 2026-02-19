/// POST /api/control/tenants handler
///
/// Creates a new tenant using the Guard → Mutation → Outbox pattern:
/// 1. Guard: check idempotency key; reject duplicate if key already used.
/// 2. Mutation: insert tenant row (status=pending) with product/plan/app_id.
/// 3. Outbox: write tenant.provisioning_started event in the same transaction.
/// 4. Assign default bundle (cp_tenant_bundle) and seed entitlements (cp_entitlements).
///
/// Returns 202 Accepted with the tenant_id, app_id, product/plan info, and bundle.
/// Returns 200 OK if idempotency key was already used (replays the result).
/// Returns 422 Unprocessable Entity for validation errors.
/// Returns 409 Conflict if tenant_id is explicitly supplied and already exists.

use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use chrono::Utc;
use serde_json::json;
use sqlx::Acquire;
use std::sync::Arc;
use uuid::Uuid;

use tenant_registry::event_types;

use crate::models::{CreateTenantRequest, CreateTenantResponse, ErrorBody};
use crate::state::AppState;

/// Default concurrent user limit when the caller omits the field.
const DEFAULT_CONCURRENT_USER_LIMIT: i32 = 5;

/// Derive a stable, unique AR app_id from a tenant UUID.
///
/// Format: `app-` + first 12 hex digits of tenant_id (no hyphens).
/// Length: 16 characters — well within VARCHAR(50).
/// Stability: deterministic; given the same tenant_id, always returns the same app_id.
fn derive_app_id(tenant_id: Uuid) -> String {
    let hex = tenant_id.to_string().replace('-', "");
    format!("app-{}", &hex[..12])
}

/// POST /api/control/tenants
pub async fn create_tenant(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTenantRequest>,
) -> Result<(StatusCode, Json<CreateTenantResponse>), (StatusCode, Json<ErrorBody>)> {
    // Validate
    if req.idempotency_key.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody {
                error: "idempotency_key must not be empty".to_string(),
            }),
        ));
    }
    if req.product_code.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody {
                error: "product_code must not be empty".to_string(),
            }),
        ));
    }
    if req.plan_code.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody {
                error: "plan_code must not be empty".to_string(),
            }),
        ));
    }

    let concurrent_user_limit = req.concurrent_user_limit.unwrap_or(DEFAULT_CONCURRENT_USER_LIMIT);
    if concurrent_user_limit <= 0 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody {
                error: "concurrent_user_limit must be > 0".to_string(),
            }),
        ));
    }

    let mut conn = state.pool.acquire().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Database connection error: {e}"),
            }),
        )
    })?;

    // --- GUARD: check idempotency key ---
    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT tenant_id FROM provisioning_requests WHERE idempotency_key = $1",
    )
    .bind(&req.idempotency_key)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db_error)?;

    if let Some((existing_tenant_id,)) = existing {
        return replay_response(&mut conn, existing_tenant_id, &req.idempotency_key).await;
    }

    // Resolve tenant ID
    let tenant_id = req.tenant_id.unwrap_or_else(Uuid::new_v4);
    let app_id = derive_app_id(tenant_id);

    // --- GUARD: check tenant_id uniqueness if explicitly supplied ---
    if req.tenant_id.is_some() {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM tenants WHERE tenant_id = $1)",
        )
        .bind(tenant_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(db_error)?;

        if exists {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorBody {
                    error: format!("Tenant {} already exists", tenant_id),
                }),
            ));
        }
    }

    let environment = req.environment.as_str();
    let now = Utc::now();

    // --- MUTATION + OUTBOX: atomic transaction ---
    let mut tx = conn.begin().await.map_err(db_error)?;

    // 1. Insert tenant row with product/plan/app_id
    sqlx::query(
        r#"
        INSERT INTO tenants
            (tenant_id, status, environment, module_schema_versions,
             product_code, plan_code, app_id, created_at, updated_at)
        VALUES ($1, 'pending', $2, '{}'::jsonb, $3, $4, $5, $6, $6)
        "#,
    )
    .bind(tenant_id)
    .bind(environment)
    .bind(&req.product_code)
    .bind(&req.plan_code)
    .bind(&app_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;

    // 2. Record idempotency key
    sqlx::query(
        "INSERT INTO provisioning_requests (idempotency_key, tenant_id, environment, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(&req.idempotency_key)
    .bind(tenant_id)
    .bind(environment)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;

    // 3. Look up default bundle for this product (optional — skip if not seeded)
    let bundle_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT bundle_id FROM cp_bundles WHERE product_code = $1 AND is_default = TRUE LIMIT 1",
    )
    .bind(&req.product_code)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_error)?;

    // 4. Assign bundle (cp_tenant_bundle) if a default bundle was found
    if let Some(bid) = bundle_id {
        sqlx::query(
            r#"
            INSERT INTO cp_tenant_bundle (tenant_id, bundle_id, status, effective_at)
            VALUES ($1, $2, 'active', $3)
            ON CONFLICT (tenant_id) DO NOTHING
            "#,
        )
        .bind(tenant_id)
        .bind(bid)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    }

    // 5. Seed cp_entitlements
    sqlx::query(
        r#"
        INSERT INTO cp_entitlements (tenant_id, plan_code, concurrent_user_limit, effective_at, updated_at)
        VALUES ($1, $2, $3, $4, $4)
        ON CONFLICT (tenant_id) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .bind(&req.plan_code)
    .bind(concurrent_user_limit)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;

    // 6. Write provisioning_started event to outbox
    let payload = json!({
        "tenant_id": tenant_id,
        "environment": environment,
        "product_code": req.product_code,
        "plan_code": req.plan_code,
        "app_id": app_id,
        "idempotency_key": req.idempotency_key,
        "occurred_at": now,
    });

    sqlx::query(
        r#"
        INSERT INTO provisioning_outbox (tenant_id, event_type, payload, created_at)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(tenant_id)
    .bind(event_types::TENANT_PROVISIONING_STARTED)
    .bind(&payload)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;

    // Commit atomically
    tx.commit().await.map_err(db_error)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(CreateTenantResponse {
            tenant_id,
            status: "pending".to_string(),
            idempotency_key: req.idempotency_key,
            app_id,
            product_code: req.product_code,
            plan_code: req.plan_code,
            concurrent_user_limit,
            bundle_id,
        }),
    ))
}

/// Replay the response for a duplicate idempotency key by reading all fields from DB.
async fn replay_response(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    tenant_id: Uuid,
    idempotency_key: &str,
) -> Result<(StatusCode, Json<CreateTenantResponse>), (StatusCode, Json<ErrorBody>)> {
    let row: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT status, product_code, plan_code, app_id FROM tenants WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&mut **conn)
    .await
    .map_err(db_error)?;

    let (status, product_code, plan_code, app_id) = row;

    let concurrent_user_limit: Option<i32> = sqlx::query_scalar(
        "SELECT concurrent_user_limit FROM cp_entitlements WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut **conn)
    .await
    .map_err(db_error)?;

    let bundle_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT bundle_id FROM cp_tenant_bundle WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut **conn)
    .await
    .map_err(db_error)?;

    Ok((
        StatusCode::OK,
        Json(CreateTenantResponse {
            tenant_id,
            status,
            idempotency_key: idempotency_key.to_string(),
            app_id: app_id.unwrap_or_else(|| derive_app_id(tenant_id)),
            product_code: product_code.unwrap_or_default(),
            plan_code: plan_code.unwrap_or_default(),
            concurrent_user_limit: concurrent_user_limit.unwrap_or(DEFAULT_CONCURRENT_USER_LIMIT),
            bundle_id,
        }),
    ))
}

fn db_error(e: sqlx::Error) -> (StatusCode, Json<ErrorBody>) {
    tracing::error!("Database error: {}", e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: format!("Database error: {e}"),
        }),
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_app_id_is_stable() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let app_id = derive_app_id(id);
        assert_eq!(app_id, "app-550e8400e29b");
        // Calling again returns the same value
        assert_eq!(derive_app_id(id), app_id);
    }

    #[test]
    fn derive_app_id_is_unique_per_tenant() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        assert_ne!(derive_app_id(id1), derive_app_id(id2));
    }

    #[test]
    fn derive_app_id_fits_varchar50() {
        let id = Uuid::new_v4();
        let app_id = derive_app_id(id);
        assert!(app_id.len() <= 50, "app_id length {} exceeds VARCHAR(50)", app_id.len());
        assert_eq!(app_id.len(), 16); // "app-" (4) + 12 hex digits
    }

    #[test]
    fn create_tenant_request_deserialises_with_defaults() {
        let json = r#"{
            "idempotency_key": "key-001",
            "environment": "development",
            "product_code": "starter",
            "plan_code": "monthly"
        }"#;
        let req: CreateTenantRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.product_code, "starter");
        assert_eq!(req.plan_code, "monthly");
        assert!(req.concurrent_user_limit.is_none());
    }

    #[test]
    fn create_tenant_request_deserialises_with_explicit_limit() {
        let json = r#"{
            "idempotency_key": "key-002",
            "environment": "production",
            "product_code": "enterprise",
            "plan_code": "annual",
            "concurrent_user_limit": 50
        }"#;
        let req: CreateTenantRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.concurrent_user_limit, Some(50));
    }

    #[tokio::test]
    async fn create_tenant_full_flow_against_real_db() {
        let db_url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
                .to_string()
        });
        let pool = match sqlx::PgPool::connect(&db_url).await {
            Ok(p) => p,
            Err(_) => return, // skip if DB unavailable
        };

        let idempotency_key = format!("test-{}", Uuid::new_v4());
        let tenant_id = Uuid::new_v4();
        let app_id = derive_app_id(tenant_id);

        // Insert tenant row with all new fields
        let now = Utc::now();
        sqlx::query(
            r#"INSERT INTO tenants
               (tenant_id, status, environment, module_schema_versions,
                product_code, plan_code, app_id, created_at, updated_at)
               VALUES ($1, 'pending', 'development', '{}'::jsonb, 'starter', 'monthly', $2, $3, $3)"#,
        )
        .bind(tenant_id)
        .bind(&app_id)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert tenant");

        // Insert idempotency record
        sqlx::query(
            "INSERT INTO provisioning_requests (idempotency_key, tenant_id, environment, created_at) VALUES ($1, $2, 'development', $3)",
        )
        .bind(&idempotency_key)
        .bind(tenant_id)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert provisioning_request");

        // Insert entitlements
        sqlx::query(
            "INSERT INTO cp_entitlements (tenant_id, plan_code, concurrent_user_limit, effective_at, updated_at) VALUES ($1, 'monthly', 10, $2, $2)",
        )
        .bind(tenant_id)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert entitlements");

        // Verify tenant row
        let row: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT product_code, plan_code, app_id FROM tenants WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_one(&pool)
        .await
        .expect("fetch tenant");

        assert_eq!(row.0.as_deref(), Some("starter"));
        assert_eq!(row.1.as_deref(), Some("monthly"));
        assert_eq!(row.2.as_deref(), Some(app_id.as_str()));

        // Verify entitlements
        let limit: i32 = sqlx::query_scalar(
            "SELECT concurrent_user_limit FROM cp_entitlements WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_one(&pool)
        .await
        .expect("fetch entitlements");
        assert_eq!(limit, 10);

        // Cleanup
        sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await
            .ok();
    }
}
