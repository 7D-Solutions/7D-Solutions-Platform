//! Asset and Category repository — SQL layer for fa_assets and fa_categories.
//!
//! All raw SQL lives here. Guard → Mutation → Outbox orchestration
//! happens in these functions since assets/categories have no separate
//! service layer (the CRUD operations are straightforward).

use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;
use crate::outbox;

// ============================================================================
// Category repository
// ============================================================================

pub struct CategoryRepo;

impl CategoryRepo {
    /// Create a new asset category.
    ///
    /// Guard: validates input. DB enforces (tenant_id, code) uniqueness.
    /// Outbox: emits category_created event.
    pub async fn create(
        pool: &PgPool,
        req: &CreateCategoryRequest,
    ) -> Result<Category, AssetError> {
        req.validate()?;

        let id = Uuid::new_v4();
        let method = req
            .default_method
            .unwrap_or(DepreciationMethod::StraightLine);
        let life_months = req.default_useful_life_months.unwrap_or(60);
        let salvage_bp = req.default_salvage_pct_bp.unwrap_or(0);

        let mut tx = pool.begin().await?;

        let cat = sqlx::query_as::<_, Category>(
            r#"
            INSERT INTO fa_categories
                (id, tenant_id, code, name, description,
                 default_method, default_useful_life_months, default_salvage_pct_bp,
                 asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
                 gain_loss_account_ref, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, TRUE, NOW(), NOW())
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.code.trim())
        .bind(req.name.trim())
        .bind(req.description.as_deref())
        .bind(method.as_str())
        .bind(life_months)
        .bind(salvage_bp)
        .bind(req.asset_account_ref.trim())
        .bind(req.depreciation_expense_ref.trim())
        .bind(req.accum_depreciation_ref.trim())
        .bind(req.gain_loss_account_ref.as_deref())
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| map_unique_violation(e, &req.code, &req.tenant_id, true))?;

        let event = CategoryCreatedEvent {
            category_id: cat.id,
            tenant_id: cat.tenant_id.clone(),
            code: cat.code.clone(),
        };
        outbox::enqueue_event_tx(
            &mut tx,
            &cat.tenant_id,
            Uuid::new_v4(),
            "category_created",
            "fa_category",
            &cat.id.to_string(),
            &event,
        )
        .await?;

        tx.commit().await?;
        Ok(cat)
    }

    /// Update mutable fields of a category.
    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateCategoryRequest,
    ) -> Result<Category, AssetError> {
        req.validate()?;

        let cat = sqlx::query_as::<_, Category>(
            r#"
            UPDATE fa_categories
            SET
                name                       = COALESCE($3, name),
                description                = CASE WHEN $4::TEXT IS NOT NULL THEN $4 ELSE description END,
                default_method             = COALESCE($5, default_method),
                default_useful_life_months = COALESCE($6, default_useful_life_months),
                default_salvage_pct_bp     = COALESCE($7, default_salvage_pct_bp),
                asset_account_ref          = COALESCE($8, asset_account_ref),
                depreciation_expense_ref   = COALESCE($9, depreciation_expense_ref),
                accum_depreciation_ref     = COALESCE($10, accum_depreciation_ref),
                gain_loss_account_ref      = CASE WHEN $11::TEXT IS NOT NULL THEN $11 ELSE gain_loss_account_ref END,
                updated_at                 = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.name.as_deref())
        .bind(req.description.as_deref())
        .bind(req.default_method.map(|m| m.as_str()))
        .bind(req.default_useful_life_months)
        .bind(req.default_salvage_pct_bp)
        .bind(req.asset_account_ref.as_deref())
        .bind(req.depreciation_expense_ref.as_deref())
        .bind(req.accum_depreciation_ref.as_deref())
        .bind(req.gain_loss_account_ref.as_deref())
        .fetch_optional(pool)
        .await?
        .ok_or(AssetError::NotFound)?;

        Ok(cat)
    }

    /// Deactivate a category (soft delete). Idempotent.
    pub async fn deactivate(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Category, AssetError> {
        let cat = sqlx::query_as::<_, Category>(
            r#"
            UPDATE fa_categories
            SET is_active = FALSE, updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(AssetError::NotFound)?;

        Ok(cat)
    }

    /// Find a category by id, tenant-scoped.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<Category>, AssetError> {
        let cat = sqlx::query_as::<_, Category>(
            "SELECT * FROM fa_categories WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;

        Ok(cat)
    }

    /// List all active categories for a tenant.
    pub async fn list(pool: &PgPool, tenant_id: &str) -> Result<Vec<Category>, AssetError> {
        let cats = sqlx::query_as::<_, Category>(
            "SELECT * FROM fa_categories WHERE tenant_id = $1 AND is_active = TRUE ORDER BY code",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;

        Ok(cats)
    }
}

// ============================================================================
// Asset repository
// ============================================================================

pub struct AssetRepo;

impl AssetRepo {
    /// Create a new asset.
    ///
    /// Guard: validates input + verifies category exists.
    /// Mutation: inserts asset with net_book_value = acquisition_cost - salvage.
    /// Outbox: emits asset_created event.
    pub async fn create(pool: &PgPool, req: &CreateAssetRequest) -> Result<Asset, AssetError> {
        req.validate()?;

        let mut tx = pool.begin().await?;

        // Guard: category must exist and belong to same tenant
        let cat = sqlx::query_as::<_, Category>(
            "SELECT * FROM fa_categories WHERE id = $1 AND tenant_id = $2",
        )
        .bind(req.category_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AssetError::CategoryNotFound(req.category_id))?;

        // Defaults from category if not overridden
        let method = req.depreciation_method.unwrap_or(
            DepreciationMethod::try_from(cat.default_method.clone())
                .unwrap_or(DepreciationMethod::StraightLine),
        );
        let life_months = req
            .useful_life_months
            .unwrap_or(cat.default_useful_life_months);
        let salvage = req.salvage_value_minor.unwrap_or(0);
        let currency = req.currency.as_deref().unwrap_or("usd");
        let nbv = req.acquisition_cost_minor - salvage;

        let id = Uuid::new_v4();

        let asset = sqlx::query_as::<_, Asset>(
            r#"
            INSERT INTO fa_assets
                (id, tenant_id, category_id, asset_tag, name, description,
                 status, acquisition_date, in_service_date,
                 acquisition_cost_minor, currency, depreciation_method,
                 useful_life_months, salvage_value_minor,
                 accum_depreciation_minor, net_book_value_minor,
                 asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
                 location, department, responsible_person,
                 serial_number, vendor, purchase_order_ref, notes,
                 created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,'draft',$7,$8,$9,$10,$11,$12,$13,0,$14,
                    $15,$16,$17,$18,$19,$20,$21,$22,$23,$24,NOW(),NOW())
            RETURNING *
            "#,
        )
        .bind(id) // $1
        .bind(&req.tenant_id) // $2
        .bind(req.category_id) // $3
        .bind(req.asset_tag.trim()) // $4
        .bind(req.name.trim()) // $5
        .bind(req.description.as_deref()) // $6
        .bind(req.acquisition_date) // $7
        .bind(req.in_service_date) // $8
        .bind(req.acquisition_cost_minor) // $9
        .bind(currency) // $10
        .bind(method.as_str()) // $11
        .bind(life_months) // $12
        .bind(salvage) // $13
        .bind(nbv) // $14
        .bind(None::<&str>) // $15 asset_account_ref (NULL = use category)
        .bind(None::<&str>) // $16 depreciation_expense_ref
        .bind(None::<&str>) // $17 accum_depreciation_ref
        .bind(req.location.as_deref()) // $18
        .bind(req.department.as_deref()) // $19
        .bind(req.responsible_person.as_deref()) // $20
        .bind(req.serial_number.as_deref()) // $21
        .bind(req.vendor.as_deref()) // $22
        .bind(req.purchase_order_ref.as_deref()) // $23
        .bind(req.notes.as_deref()) // $24
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| map_unique_violation(e, &req.asset_tag, &req.tenant_id, false))?;

        let event = AssetCreatedEvent {
            asset_id: asset.id,
            tenant_id: asset.tenant_id.clone(),
            asset_tag: asset.asset_tag.clone(),
            category_id: asset.category_id,
            acquisition_cost_minor: asset.acquisition_cost_minor,
            currency: asset.currency.clone(),
        };
        outbox::enqueue_event_tx(
            &mut tx,
            &asset.tenant_id,
            Uuid::new_v4(),
            "asset_created",
            "fa_asset",
            &asset.id.to_string(),
            &event,
        )
        .await?;

        tx.commit().await?;
        Ok(asset)
    }

    /// Update mutable fields of an asset.
    ///
    /// Only descriptive fields are updatable — cost, method, life are immutable
    /// post-creation (adjustments require explicit lifecycle events).
    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateAssetRequest,
    ) -> Result<Asset, AssetError> {
        req.validate()?;

        let mut tx = pool.begin().await?;

        let asset = sqlx::query_as::<_, Asset>(
            r#"
            UPDATE fa_assets
            SET
                name               = COALESCE($3, name),
                description        = CASE WHEN $4::TEXT IS NOT NULL THEN $4 ELSE description END,
                location           = CASE WHEN $5::TEXT IS NOT NULL THEN $5 ELSE location END,
                department         = CASE WHEN $6::TEXT IS NOT NULL THEN $6 ELSE department END,
                responsible_person = CASE WHEN $7::TEXT IS NOT NULL THEN $7 ELSE responsible_person END,
                notes              = CASE WHEN $8::TEXT IS NOT NULL THEN $8 ELSE notes END,
                updated_at         = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.name.as_deref())
        .bind(req.description.as_deref())
        .bind(req.location.as_deref())
        .bind(req.department.as_deref())
        .bind(req.responsible_person.as_deref())
        .bind(req.notes.as_deref())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AssetError::NotFound)?;

        let event = AssetUpdatedEvent {
            asset_id: asset.id,
            tenant_id: asset.tenant_id.clone(),
        };
        outbox::enqueue_event_tx(
            &mut tx,
            &asset.tenant_id,
            Uuid::new_v4(),
            "asset_updated",
            "fa_asset",
            &asset.id.to_string(),
            &event,
        )
        .await?;

        tx.commit().await?;
        Ok(asset)
    }

    /// Deactivate (dispose) an asset. Only draft/active assets can be deactivated.
    pub async fn deactivate(pool: &PgPool, id: Uuid, tenant_id: &str) -> Result<Asset, AssetError> {
        let mut tx = pool.begin().await?;

        // Guard: check current status
        let current =
            sqlx::query_as::<_, Asset>("SELECT * FROM fa_assets WHERE id = $1 AND tenant_id = $2")
                .bind(id)
                .bind(tenant_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(AssetError::NotFound)?;

        let status = AssetStatus::try_from(current.status.clone()).unwrap_or(AssetStatus::Draft);
        if status == AssetStatus::Disposed || status == AssetStatus::Impaired {
            // Already deactivated — idempotent return
            tx.commit().await?;
            return Ok(current);
        }

        let asset = sqlx::query_as::<_, Asset>(
            r#"
            UPDATE fa_assets
            SET status = 'disposed', updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        let event = AssetDeactivatedEvent {
            asset_id: asset.id,
            tenant_id: asset.tenant_id.clone(),
            previous_status: current.status,
        };
        outbox::enqueue_event_tx(
            &mut tx,
            &asset.tenant_id,
            Uuid::new_v4(),
            "asset_deactivated",
            "fa_asset",
            &asset.id.to_string(),
            &event,
        )
        .await?;

        tx.commit().await?;
        Ok(asset)
    }

    /// Find an asset by id, tenant-scoped.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<Asset>, AssetError> {
        let asset =
            sqlx::query_as::<_, Asset>("SELECT * FROM fa_assets WHERE id = $1 AND tenant_id = $2")
                .bind(id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?;

        Ok(asset)
    }

    /// List assets for a tenant, optionally filtered by status.
    pub async fn list(
        pool: &PgPool,
        tenant_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<Asset>, AssetError> {
        let assets = if let Some(status) = status_filter {
            sqlx::query_as::<_, Asset>(
                "SELECT * FROM fa_assets WHERE tenant_id = $1 AND status = $2 ORDER BY asset_tag",
            )
            .bind(tenant_id)
            .bind(status)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as::<_, Asset>(
                "SELECT * FROM fa_assets WHERE tenant_id = $1 ORDER BY asset_tag",
            )
            .bind(tenant_id)
            .fetch_all(pool)
            .await?
        };

        Ok(assets)
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn map_unique_violation(e: sqlx::Error, key: &str, tenant: &str, is_category: bool) -> AssetError {
    if let sqlx::Error::Database(ref dbe) = e {
        if dbe.code().as_deref() == Some("23505") {
            return if is_category {
                AssetError::DuplicateCategoryCode(key.to_string(), tenant.to_string())
            } else {
                AssetError::DuplicateTag(key.to_string(), tenant.to_string())
            };
        }
    }
    AssetError::Database(e)
}
