use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::models::*;
use crate::domain::service::QiError;

// ============================================================================
// Insert operations (require transaction)
// ============================================================================

pub async fn insert_receiving_inspection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    req: &CreateReceivingInspectionRequest,
    result_val: &str,
) -> Result<Inspection, QiError> {
    let inspection = sqlx::query_as::<_, Inspection>(
        r#"
        INSERT INTO inspections
            (tenant_id, plan_id, lot_id, inspector_id, inspection_type,
             result, notes, receipt_id, part_id, part_revision,
             inspected_at)
        VALUES ($1, $2, $3, $4, 'receiving', $5, $6, $7, $8, $9,
                CASE WHEN $5 != 'pending' THEN NOW() ELSE NULL END)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(req.plan_id)
    .bind(req.lot_id)
    .bind(req.inspector_id)
    .bind(result_val)
    .bind(&req.notes)
    .bind(req.receipt_id)
    .bind(req.part_id)
    .bind(&req.part_revision)
    .fetch_one(&mut **tx)
    .await?;
    Ok(inspection)
}

pub async fn insert_in_process_inspection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    req: &CreateInProcessInspectionRequest,
    result_val: &str,
) -> Result<Inspection, QiError> {
    let inspection = sqlx::query_as::<_, Inspection>(
        r#"
        INSERT INTO inspections
            (tenant_id, plan_id, lot_id, inspector_id, inspection_type,
             result, notes, wo_id, op_instance_id, part_id, part_revision,
             inspected_at)
        VALUES ($1, $2, $3, $4, 'in_process', $5, $6, $7, $8, $9, $10,
                CASE WHEN $5 != 'pending' THEN NOW() ELSE NULL END)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(req.plan_id)
    .bind(req.lot_id)
    .bind(req.inspector_id)
    .bind(result_val)
    .bind(&req.notes)
    .bind(req.wo_id)
    .bind(req.op_instance_id)
    .bind(req.part_id)
    .bind(&req.part_revision)
    .fetch_one(&mut **tx)
    .await?;
    Ok(inspection)
}

pub async fn insert_final_inspection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    req: &CreateFinalInspectionRequest,
    result_val: &str,
) -> Result<Inspection, QiError> {
    let inspection = sqlx::query_as::<_, Inspection>(
        r#"
        INSERT INTO inspections
            (tenant_id, plan_id, lot_id, inspector_id, inspection_type,
             result, notes, wo_id, part_id, part_revision,
             inspected_at)
        VALUES ($1, $2, $3, $4, 'final', $5, $6, $7, $8, $9,
                CASE WHEN $5 != 'pending' THEN NOW() ELSE NULL END)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(req.plan_id)
    .bind(req.lot_id)
    .bind(req.inspector_id)
    .bind(result_val)
    .bind(&req.notes)
    .bind(req.wo_id)
    .bind(req.part_id)
    .bind(&req.part_revision)
    .fetch_one(&mut **tx)
    .await?;
    Ok(inspection)
}

pub async fn update_disposition(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    inspection_id: Uuid,
    target: &str,
) -> Result<Inspection, QiError> {
    let updated = sqlx::query_as::<_, Inspection>(
        r#"
        UPDATE inspections
        SET disposition = $1, updated_at = NOW()
        WHERE id = $2 AND tenant_id = $3
        RETURNING *
        "#,
    )
    .bind(target)
    .bind(inspection_id)
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(updated)
}

// ============================================================================
// Read operations (pool)
// ============================================================================

pub async fn get_by_id(
    pool: &PgPool,
    tenant_id: &str,
    inspection_id: Uuid,
) -> Result<Option<Inspection>, QiError> {
    let row = sqlx::query_as::<_, Inspection>(
        "SELECT * FROM inspections WHERE id = $1 AND tenant_id = $2",
    )
    .bind(inspection_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_part_rev(
    pool: &PgPool,
    tenant_id: &str,
    part_id: Uuid,
    part_revision: Option<&str>,
) -> Result<Vec<Inspection>, QiError> {
    let rows = if let Some(rev) = part_revision {
        sqlx::query_as::<_, Inspection>(
            r#"
            SELECT * FROM inspections
            WHERE tenant_id = $1 AND part_id = $2 AND part_revision = $3
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(part_id)
        .bind(rev)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Inspection>(
            r#"
            SELECT * FROM inspections
            WHERE tenant_id = $1 AND part_id = $2
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(part_id)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

pub async fn list_by_part_rev_paginated(
    pool: &PgPool,
    tenant_id: &str,
    part_id: Uuid,
    part_revision: Option<&str>,
    page_size: i64,
    offset: i64,
) -> Result<(Vec<Inspection>, i64), QiError> {
    if let Some(rev) = part_revision {
        let rows = sqlx::query_as::<_, Inspection>(
            r#"SELECT * FROM inspections
               WHERE tenant_id = $1 AND part_id = $2 AND part_revision = $3
               ORDER BY created_at DESC LIMIT $4 OFFSET $5"#,
        )
        .bind(tenant_id)
        .bind(part_id)
        .bind(rev)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let total: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM inspections WHERE tenant_id = $1 AND part_id = $2 AND part_revision = $3",
        )
        .bind(tenant_id)
        .bind(part_id)
        .bind(rev)
        .fetch_one(pool)
        .await?;

        Ok((rows, total.0))
    } else {
        let rows = sqlx::query_as::<_, Inspection>(
            r#"SELECT * FROM inspections
               WHERE tenant_id = $1 AND part_id = $2
               ORDER BY created_at DESC LIMIT $3 OFFSET $4"#,
        )
        .bind(tenant_id)
        .bind(part_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let total: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM inspections WHERE tenant_id = $1 AND part_id = $2",
        )
        .bind(tenant_id)
        .bind(part_id)
        .fetch_one(pool)
        .await?;

        Ok((rows, total.0))
    }
}

pub async fn list_by_receipt(
    pool: &PgPool,
    tenant_id: &str,
    receipt_id: Uuid,
) -> Result<Vec<Inspection>, QiError> {
    let rows = sqlx::query_as::<_, Inspection>(
        r#"
        SELECT * FROM inspections
        WHERE tenant_id = $1 AND receipt_id = $2
        ORDER BY created_at DESC
        "#,
    )
    .bind(tenant_id)
    .bind(receipt_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_receipt_paginated(
    pool: &PgPool,
    tenant_id: &str,
    receipt_id: Uuid,
    page_size: i64,
    offset: i64,
) -> Result<(Vec<Inspection>, i64), QiError> {
    let rows = sqlx::query_as::<_, Inspection>(
        r#"SELECT * FROM inspections
           WHERE tenant_id = $1 AND receipt_id = $2
           ORDER BY created_at DESC LIMIT $3 OFFSET $4"#,
    )
    .bind(tenant_id)
    .bind(receipt_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let total: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM inspections WHERE tenant_id = $1 AND receipt_id = $2")
            .bind(tenant_id)
            .bind(receipt_id)
            .fetch_one(pool)
            .await?;

    Ok((rows, total.0))
}

pub async fn list_by_wo(
    pool: &PgPool,
    tenant_id: &str,
    wo_id: Uuid,
    inspection_type: Option<&str>,
) -> Result<Vec<Inspection>, QiError> {
    let rows = if let Some(itype) = inspection_type {
        sqlx::query_as::<_, Inspection>(
            r#"
            SELECT * FROM inspections
            WHERE tenant_id = $1 AND wo_id = $2 AND inspection_type = $3
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(wo_id)
        .bind(itype)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Inspection>(
            r#"
            SELECT * FROM inspections
            WHERE tenant_id = $1 AND wo_id = $2
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(wo_id)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

pub async fn list_by_wo_paginated(
    pool: &PgPool,
    tenant_id: &str,
    wo_id: Uuid,
    inspection_type: Option<&str>,
    page_size: i64,
    offset: i64,
) -> Result<(Vec<Inspection>, i64), QiError> {
    if let Some(itype) = inspection_type {
        let rows = sqlx::query_as::<_, Inspection>(
            r#"SELECT * FROM inspections
               WHERE tenant_id = $1 AND wo_id = $2 AND inspection_type = $3
               ORDER BY created_at DESC LIMIT $4 OFFSET $5"#,
        )
        .bind(tenant_id)
        .bind(wo_id)
        .bind(itype)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let total: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM inspections WHERE tenant_id = $1 AND wo_id = $2 AND inspection_type = $3",
        )
        .bind(tenant_id)
        .bind(wo_id)
        .bind(itype)
        .fetch_one(pool)
        .await?;

        Ok((rows, total.0))
    } else {
        let rows = sqlx::query_as::<_, Inspection>(
            r#"SELECT * FROM inspections
               WHERE tenant_id = $1 AND wo_id = $2
               ORDER BY created_at DESC LIMIT $3 OFFSET $4"#,
        )
        .bind(tenant_id)
        .bind(wo_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let total: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM inspections WHERE tenant_id = $1 AND wo_id = $2")
                .bind(tenant_id)
                .bind(wo_id)
                .fetch_one(pool)
                .await?;

        Ok((rows, total.0))
    }
}

pub async fn list_by_lot(
    pool: &PgPool,
    tenant_id: &str,
    lot_id: Uuid,
) -> Result<Vec<Inspection>, QiError> {
    let rows = sqlx::query_as::<_, Inspection>(
        r#"
        SELECT * FROM inspections
        WHERE tenant_id = $1 AND lot_id = $2
        ORDER BY created_at DESC
        "#,
    )
    .bind(tenant_id)
    .bind(lot_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_lot_paginated(
    pool: &PgPool,
    tenant_id: &str,
    lot_id: Uuid,
    page_size: i64,
    offset: i64,
) -> Result<(Vec<Inspection>, i64), QiError> {
    let rows = sqlx::query_as::<_, Inspection>(
        r#"SELECT * FROM inspections
           WHERE tenant_id = $1 AND lot_id = $2
           ORDER BY created_at DESC LIMIT $3 OFFSET $4"#,
    )
    .bind(tenant_id)
    .bind(lot_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let total: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM inspections WHERE tenant_id = $1 AND lot_id = $2")
            .bind(tenant_id)
            .bind(lot_id)
            .fetch_one(pool)
            .await?;

    Ok((rows, total.0))
}
