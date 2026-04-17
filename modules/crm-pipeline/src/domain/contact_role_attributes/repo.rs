//! Contact role attributes repository.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{ContactRoleAttributes, ContactRoleError, UpsertContactRoleRequest};

pub async fn get_attributes(
    pool: &PgPool,
    tenant_id: &str,
    party_contact_id: Uuid,
) -> Result<Option<ContactRoleAttributes>, ContactRoleError> {
    let row = sqlx::query_as::<_, ContactRoleAttributes>(
        "SELECT * FROM contact_role_attributes WHERE tenant_id = $1 AND party_contact_id = $2",
    )
    .bind(tenant_id)
    .bind(party_contact_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn upsert_attributes(
    pool: &PgPool,
    tenant_id: &str,
    party_contact_id: Uuid,
    req: &UpsertContactRoleRequest,
    actor: &str,
) -> Result<ContactRoleAttributes, ContactRoleError> {
    let row = sqlx::query_as::<_, ContactRoleAttributes>(
        r#"
        INSERT INTO contact_role_attributes (
            id, tenant_id, party_contact_id, sales_role,
            is_primary_buyer, is_economic_buyer, is_active, notes, updated_by, updated_at
        ) VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, TRUE, $6, $7, NOW())
        ON CONFLICT (tenant_id, party_contact_id)
        DO UPDATE SET
            sales_role        = COALESCE($3, contact_role_attributes.sales_role),
            is_primary_buyer  = COALESCE($4, contact_role_attributes.is_primary_buyer),
            is_economic_buyer = COALESCE($5, contact_role_attributes.is_economic_buyer),
            notes             = COALESCE($6, contact_role_attributes.notes),
            updated_by        = $7,
            updated_at        = NOW()
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(party_contact_id)
    .bind(req.sales_role.as_deref().unwrap_or("unknown"))
    .bind(req.is_primary_buyer.unwrap_or(false))
    .bind(req.is_economic_buyer.unwrap_or(false))
    .bind(&req.notes)
    .bind(actor)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn deactivate_contact(
    pool: &PgPool,
    tenant_id: &str,
    party_contact_id: Uuid,
) -> Result<(), ContactRoleError> {
    sqlx::query(
        r#"
        UPDATE contact_role_attributes
        SET is_active = FALSE, updated_at = NOW(), updated_by = 'system'
        WHERE tenant_id = $1 AND party_contact_id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(party_contact_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn nullify_opp_primary_contact(
    pool: &PgPool,
    tenant_id: &str,
    party_contact_id: Uuid,
) -> Result<(), ContactRoleError> {
    sqlx::query(
        r#"
        UPDATE opportunities
        SET primary_party_contact_id = NULL, updated_at = NOW()
        WHERE tenant_id = $1 AND primary_party_contact_id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(party_contact_id)
    .execute(pool)
    .await?;
    Ok(())
}
