use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

// ── Row types ──────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Permission {
    pub id: Uuid,
    pub key: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Role {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: String,
    pub is_system: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct UserRoleBinding {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub role_id: Uuid,
    pub granted_by: Option<Uuid>,
    pub granted_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

// ── Permissions CRUD ───────────────────────────────────────

pub async fn create_permission(
    pool: &PgPool,
    key: &str,
    description: &str,
) -> Result<Permission, sqlx::Error> {
    sqlx::query_as::<_, Permission>(
        r#"INSERT INTO permissions (key, description)
           VALUES ($1, $2)
           RETURNING id, key, description, created_at"#,
    )
    .bind(key)
    .bind(description)
    .fetch_one(pool)
    .await
}

pub async fn get_permission_by_key(
    pool: &PgPool,
    key: &str,
) -> Result<Option<Permission>, sqlx::Error> {
    sqlx::query_as::<_, Permission>(
        "SELECT id, key, description, created_at FROM permissions WHERE key = $1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
}

pub async fn list_permissions(pool: &PgPool) -> Result<Vec<Permission>, sqlx::Error> {
    sqlx::query_as::<_, Permission>(
        "SELECT id, key, description, created_at FROM permissions ORDER BY key",
    )
    .fetch_all(pool)
    .await
}

pub async fn delete_permission(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM permissions WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

// ── Roles CRUD ─────────────────────────────────────────────

pub async fn create_role(
    pool: &PgPool,
    tenant_id: Uuid,
    name: &str,
    description: &str,
    is_system: bool,
) -> Result<Role, sqlx::Error> {
    sqlx::query_as::<_, Role>(
        r#"INSERT INTO roles (tenant_id, name, description, is_system)
           VALUES ($1, $2, $3, $4)
           RETURNING id, tenant_id, name, description, is_system, created_at, updated_at"#,
    )
    .bind(tenant_id)
    .bind(name)
    .bind(description)
    .bind(is_system)
    .fetch_one(pool)
    .await
}

pub async fn get_role(
    pool: &PgPool,
    tenant_id: Uuid,
    role_id: Uuid,
) -> Result<Option<Role>, sqlx::Error> {
    sqlx::query_as::<_, Role>(
        r#"SELECT id, tenant_id, name, description, is_system, created_at, updated_at
           FROM roles WHERE tenant_id = $1 AND id = $2"#,
    )
    .bind(tenant_id)
    .bind(role_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_roles(pool: &PgPool, tenant_id: Uuid) -> Result<Vec<Role>, sqlx::Error> {
    sqlx::query_as::<_, Role>(
        r#"SELECT id, tenant_id, name, description, is_system, created_at, updated_at
           FROM roles WHERE tenant_id = $1 ORDER BY name"#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

pub async fn update_role(
    pool: &PgPool,
    tenant_id: Uuid,
    role_id: Uuid,
    name: &str,
    description: &str,
) -> Result<Option<Role>, sqlx::Error> {
    sqlx::query_as::<_, Role>(
        r#"UPDATE roles
           SET name = $3, description = $4, updated_at = NOW()
           WHERE tenant_id = $1 AND id = $2 AND is_system = false
           RETURNING id, tenant_id, name, description, is_system, created_at, updated_at"#,
    )
    .bind(tenant_id)
    .bind(role_id)
    .bind(name)
    .bind(description)
    .fetch_optional(pool)
    .await
}

pub async fn delete_role(
    pool: &PgPool,
    tenant_id: Uuid,
    role_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let res =
        sqlx::query("DELETE FROM roles WHERE tenant_id = $1 AND id = $2 AND is_system = false")
            .bind(tenant_id)
            .bind(role_id)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

// ── Role ↔ Permission management ──────────────────────────

pub async fn grant_permission_to_role(
    pool: &PgPool,
    role_id: Uuid,
    permission_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO role_permissions (role_id, permission_id)
           VALUES ($1, $2)
           ON CONFLICT (role_id, permission_id) DO NOTHING"#,
    )
    .bind(role_id)
    .bind(permission_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn revoke_permission_from_role(
    pool: &PgPool,
    role_id: Uuid,
    permission_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM role_permissions WHERE role_id = $1 AND permission_id = $2")
        .bind(role_id)
        .bind(permission_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list_permissions_for_role(
    pool: &PgPool,
    role_id: Uuid,
) -> Result<Vec<Permission>, sqlx::Error> {
    sqlx::query_as::<_, Permission>(
        r#"SELECT p.id, p.key, p.description, p.created_at
           FROM permissions p
           JOIN role_permissions rp ON rp.permission_id = p.id
           WHERE rp.role_id = $1
           ORDER BY p.key"#,
    )
    .bind(role_id)
    .fetch_all(pool)
    .await
}

// ── User ↔ Role bindings ──────────────────────────────────

pub async fn bind_user_role(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    role_id: Uuid,
    granted_by: Option<Uuid>,
) -> Result<UserRoleBinding, sqlx::Error> {
    let ctx = crate::db::user_lifecycle_audit::LifecycleAuditContext {
        producer: format!("auth-rs@{}", env!("CARGO_PKG_VERSION")),
        trace_id: format!("rbac-bind-{}", Uuid::new_v4()),
        causation_id: None,
        idempotency_key: format!(
            "role-bind:{}:{}:{}:{}",
            tenant_id,
            user_id,
            role_id,
            Uuid::new_v4()
        ),
    };
    bind_user_role_with_audit(pool, tenant_id, user_id, role_id, granted_by, &ctx).await
}

pub async fn bind_user_role_with_audit(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    role_id: Uuid,
    granted_by: Option<Uuid>,
    ctx: &crate::db::user_lifecycle_audit::LifecycleAuditContext,
) -> Result<UserRoleBinding, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let binding = sqlx::query_as::<_, UserRoleBinding>(
        r#"INSERT INTO user_role_bindings (tenant_id, user_id, role_id, granted_by)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (tenant_id, user_id, role_id)
           DO UPDATE SET revoked_at = NULL, granted_by = $4, granted_at = NOW()
           RETURNING id, tenant_id, user_id, role_id, granted_by, granted_at, revoked_at"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(role_id)
    .bind(granted_by)
    .fetch_one(&mut *tx)
    .await?;

    let payload = json!({
        "binding_id": binding.id,
        "user_id": user_id,
        "role_id": role_id,
        "granted_by": granted_by,
        "granted_at": binding.granted_at,
    });

    crate::db::user_lifecycle_audit::append_lifecycle_event_tx(
        &mut tx,
        tenant_id,
        user_id,
        crate::db::user_lifecycle_audit::LifecycleEventType::RoleAssigned,
        granted_by,
        Some(role_id),
        None,
        None,
        payload,
        ctx,
    )
    .await?;

    tx.commit().await?;
    Ok(binding)
}

pub async fn revoke_user_role(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    role_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let ctx = crate::db::user_lifecycle_audit::LifecycleAuditContext {
        producer: format!("auth-rs@{}", env!("CARGO_PKG_VERSION")),
        trace_id: format!("rbac-revoke-{}", Uuid::new_v4()),
        causation_id: None,
        idempotency_key: format!(
            "role-revoke:{}:{}:{}:{}",
            tenant_id,
            user_id,
            role_id,
            Uuid::new_v4()
        ),
    };
    revoke_user_role_with_audit(pool, tenant_id, user_id, role_id, None, &ctx).await
}

pub async fn revoke_user_role_with_audit(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    role_id: Uuid,
    revoked_by: Option<Uuid>,
    ctx: &crate::db::user_lifecycle_audit::LifecycleAuditContext,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        r#"UPDATE user_role_bindings
           SET revoked_at = NOW()
           WHERE tenant_id = $1 AND user_id = $2 AND role_id = $3 AND revoked_at IS NULL"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(role_id)
    .execute(&mut *tx)
    .await?;

    let changed = res.rows_affected() > 0;
    if changed {
        let payload = json!({
            "user_id": user_id,
            "role_id": role_id,
            "revoked_by": revoked_by,
            "revoked_at": Utc::now(),
        });

        crate::db::user_lifecycle_audit::append_lifecycle_event_tx(
            &mut tx,
            tenant_id,
            user_id,
            crate::db::user_lifecycle_audit::LifecycleEventType::RoleRevoked,
            revoked_by,
            Some(role_id),
            None,
            None,
            payload,
            ctx,
        )
        .await?;
    }

    tx.commit().await?;
    Ok(changed)
}

pub async fn list_roles_for_user(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<Role>, sqlx::Error> {
    sqlx::query_as::<_, Role>(
        r#"SELECT r.id, r.tenant_id, r.name, r.description, r.is_system,
                  r.created_at, r.updated_at
           FROM roles r
           JOIN user_role_bindings urb ON urb.role_id = r.id
           WHERE urb.tenant_id = $1 AND urb.user_id = $2 AND urb.revoked_at IS NULL
           ORDER BY r.name"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Effective permissions: all permission keys a user holds via active role bindings.
/// Used by JWT claims to embed permissions in tokens.
pub async fn effective_permissions_for_user(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query_scalar::<_, String>(
        r#"SELECT DISTINCT p.key
           FROM permissions p
           JOIN role_permissions rp ON rp.permission_id = p.id
           JOIN user_role_bindings urb ON urb.role_id = rp.role_id
           WHERE urb.tenant_id = $1
             AND urb.user_id = $2
             AND urb.revoked_at IS NULL
           ORDER BY p.key"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
