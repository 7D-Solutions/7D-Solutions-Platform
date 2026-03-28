//! Item repository — database operations for the Item master.

use serde::Deserialize;
use sqlx::{PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::items::{CreateItemRequest, Item, ItemError, UpdateItemRequest};

// ============================================================================
// List / search query
// ============================================================================

fn default_limit() -> i64 {
    50
}

/// URL query parameters for `GET /api/inventory/items`.
#[derive(Debug, Deserialize)]
pub struct ListItemsQuery {
    pub search: Option<String>,
    pub tracking_mode: Option<String>,
    pub make_buy: Option<String>,
    /// `None` or omitted → active-only (default). `true` → active. `false` → inactive.
    pub active: Option<bool>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

// ============================================================================
// Repository
// ============================================================================

pub struct ItemRepo;

impl ItemRepo {
    /// Create a new item.
    ///
    /// Validates input, then checks SKU uniqueness via DB constraint.
    /// Returns `DuplicateSku` if the (tenant_id, sku) pair already exists.
    pub async fn create(pool: &PgPool, req: &CreateItemRequest) -> Result<Item, ItemError> {
        req.validate()?;

        let uom = req.uom.as_deref().unwrap_or("ea");
        let id = Uuid::new_v4();
        let now = chrono::Utc::now();

        let item = sqlx::query_as::<_, Item>(
            r#"
            INSERT INTO items
                (id, tenant_id, sku, name, description,
                 inventory_account_ref, cogs_account_ref, variance_account_ref,
                 uom, tracking_mode, make_buy, active, created_at, updated_at)
            VALUES
                ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, TRUE, $12, $12)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.sku.trim())
        .bind(req.name.trim())
        .bind(req.description.as_deref())
        .bind(req.inventory_account_ref.trim())
        .bind(req.cogs_account_ref.trim())
        .bind(req.variance_account_ref.trim())
        .bind(uom)
        .bind(req.tracking_mode.as_str())
        .bind(req.make_buy.as_deref())
        .bind(now)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return ItemError::DuplicateSku(req.sku.clone(), req.tenant_id.clone());
                }
            }
            ItemError::Database(e)
        })?;

        Ok(item)
    }

    /// Update mutable fields of an existing item.
    ///
    /// Only fields present in the request are updated; missing fields left unchanged.
    /// Scoped to tenant_id to prevent cross-tenant mutation.
    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateItemRequest,
    ) -> Result<Item, ItemError> {
        req.validate()?;

        let item = sqlx::query_as::<_, Item>(
            r#"
            UPDATE items
            SET
                name                 = COALESCE($3, name),
                description          = CASE WHEN $4::TEXT IS NOT NULL THEN $4 ELSE description END,
                inventory_account_ref = COALESCE($5, inventory_account_ref),
                cogs_account_ref     = COALESCE($6, cogs_account_ref),
                variance_account_ref = COALESCE($7, variance_account_ref),
                uom                  = COALESCE($8, uom),
                updated_at           = NOW()
            WHERE id = $1
              AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.name.as_deref())
        .bind(req.description.as_deref())
        .bind(req.inventory_account_ref.as_deref())
        .bind(req.cogs_account_ref.as_deref())
        .bind(req.variance_account_ref.as_deref())
        .bind(req.uom.as_deref())
        .fetch_optional(pool)
        .await?
        .ok_or(ItemError::NotFound)?;

        Ok(item)
    }

    /// Deactivate an item (soft delete). Idempotent.
    /// Scoped to tenant_id to prevent cross-tenant mutation.
    pub async fn deactivate(pool: &PgPool, id: Uuid, tenant_id: &str) -> Result<Item, ItemError> {
        let item = sqlx::query_as::<_, Item>(
            r#"
            UPDATE items
            SET active = FALSE, updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ItemError::NotFound)?;

        Ok(item)
    }

    /// Fetch an item by id, scoped to tenant.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<Item>, ItemError> {
        let item =
            sqlx::query_as::<_, Item>("SELECT * FROM items WHERE id = $1 AND tenant_id = $2")
                .bind(id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?;

        Ok(item)
    }

    /// List items with search, filtering, and pagination.
    ///
    /// Tenant isolation: always filters by `tenant_id`.
    /// Default: returns only active items when `active` is omitted.
    pub async fn list(
        pool: &PgPool,
        tenant_id: &str,
        q: &ListItemsQuery,
    ) -> Result<(Vec<Item>, i64), ItemError> {
        let limit = q.limit.clamp(1, 200);
        let offset = q.offset.max(0);
        let active_val = q.active.unwrap_or(true);

        let has_search = q.search.as_ref().is_some_and(|s| !s.trim().is_empty());
        let search_pattern = if has_search {
            format!("%{}%", q.search.as_ref().unwrap().trim())
        } else {
            String::new()
        };
        let tracking_mode = q
            .tracking_mode
            .as_deref()
            .filter(|s| !s.trim().is_empty());
        let make_buy = q.make_buy.as_deref().filter(|s| !s.trim().is_empty());

        // -- items query --
        let mut qb =
            QueryBuilder::<Postgres>::new("SELECT * FROM items WHERE tenant_id = ");
        qb.push_bind(tenant_id.to_string());
        qb.push(" AND active = ");
        qb.push_bind(active_val);
        if has_search {
            qb.push(" AND (sku ILIKE ");
            qb.push_bind(search_pattern.clone());
            qb.push(" OR name ILIKE ");
            qb.push_bind(search_pattern.clone());
            qb.push(" OR description ILIKE ");
            qb.push_bind(search_pattern.clone());
            qb.push(")");
        }
        if let Some(tm) = tracking_mode {
            qb.push(" AND tracking_mode = ");
            qb.push_bind(tm.to_string());
        }
        if let Some(mb) = make_buy {
            qb.push(" AND make_buy = ");
            qb.push_bind(mb.to_string());
        }
        qb.push(" ORDER BY name ASC, sku ASC LIMIT ");
        qb.push_bind(limit);
        qb.push(" OFFSET ");
        qb.push_bind(offset);

        let items: Vec<Item> = qb.build_query_as().fetch_all(pool).await?;

        // -- count query (same filters, no LIMIT/OFFSET) --
        let mut cqb =
            QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM items WHERE tenant_id = ");
        cqb.push_bind(tenant_id.to_string());
        cqb.push(" AND active = ");
        cqb.push_bind(active_val);
        if has_search {
            cqb.push(" AND (sku ILIKE ");
            cqb.push_bind(search_pattern.clone());
            cqb.push(" OR name ILIKE ");
            cqb.push_bind(search_pattern.clone());
            cqb.push(" OR description ILIKE ");
            cqb.push_bind(search_pattern);
            cqb.push(")");
        }
        if let Some(tm) = tracking_mode {
            cqb.push(" AND tracking_mode = ");
            cqb.push_bind(tm.to_string());
        }
        if let Some(mb) = make_buy {
            cqb.push(" AND make_buy = ");
            cqb.push_bind(mb.to_string());
        }

        let total: i64 = cqb.build_query_scalar().fetch_one(pool).await?;

        Ok((items, total))
    }
}
