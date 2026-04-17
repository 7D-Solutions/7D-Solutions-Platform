//! Barcode resolver repository — database operations for format rules and entity lookups.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

// ============================================================================
// Domain models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct BarcodeFormatRule {
    pub id: Uuid,
    pub tenant_id: String,
    pub rule_name: String,
    pub pattern_regex: String,
    pub entity_type_when_matched: String,
    pub capture_group_index: i32,
    pub priority: i32,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

// ============================================================================
// Internal row types for entity lookups
// ============================================================================

#[derive(sqlx::FromRow)]
pub(crate) struct ItemLookupRow {
    pub id: Uuid,
    #[allow(dead_code)]
    pub sku: String,
}

#[derive(sqlx::FromRow)]
pub(crate) struct LotLookupRow {
    pub id: Uuid,
    #[allow(dead_code)]
    pub lot_code: String,
    #[allow(dead_code)]
    pub item_id: Uuid,
}

#[derive(sqlx::FromRow)]
pub(crate) struct SerialLookupRow {
    pub id: Uuid,
    #[allow(dead_code)]
    pub serial_code: String,
    #[allow(dead_code)]
    pub item_id: Uuid,
    #[allow(dead_code)]
    pub status: String,
}

// ============================================================================
// Rule queries
// ============================================================================

/// Fetch all active rules for a tenant, ordered by (priority ASC, id ASC).
pub(crate) async fn list_active_rules(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<BarcodeFormatRule>, sqlx::Error> {
    sqlx::query_as::<_, BarcodeFormatRule>(
        r#"
        SELECT * FROM barcode_format_rules
        WHERE tenant_id = $1 AND active = TRUE
        ORDER BY priority ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

/// Fetch all rules for a tenant (active and inactive), ordered by (priority ASC, id ASC).
pub(crate) async fn list_all_rules(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<BarcodeFormatRule>, sqlx::Error> {
    sqlx::query_as::<_, BarcodeFormatRule>(
        r#"
        SELECT * FROM barcode_format_rules
        WHERE tenant_id = $1
        ORDER BY priority ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

/// Fetch a single rule by id, scoped to tenant.
#[allow(dead_code)]
pub(crate) async fn get_rule(
    pool: &PgPool,
    tenant_id: &str,
    rule_id: Uuid,
) -> Result<Option<BarcodeFormatRule>, sqlx::Error> {
    sqlx::query_as::<_, BarcodeFormatRule>(
        "SELECT * FROM barcode_format_rules WHERE id = $1 AND tenant_id = $2",
    )
    .bind(rule_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

// ============================================================================
// Rule mutations
// ============================================================================

pub(crate) async fn insert_rule(
    pool: &PgPool,
    tenant_id: &str,
    rule_name: &str,
    pattern_regex: &str,
    entity_type: &str,
    capture_group_index: i32,
    priority: i32,
    updated_by: Option<&str>,
) -> Result<BarcodeFormatRule, sqlx::Error> {
    let now = Utc::now();
    sqlx::query_as::<_, BarcodeFormatRule>(
        r#"
        INSERT INTO barcode_format_rules
            (tenant_id, rule_name, pattern_regex, entity_type_when_matched,
             capture_group_index, priority, active, created_at, updated_at, updated_by)
        VALUES ($1, $2, $3, $4, $5, $6, TRUE, $7, $7, $8)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(rule_name)
    .bind(pattern_regex)
    .bind(entity_type)
    .bind(capture_group_index)
    .bind(priority)
    .bind(now)
    .bind(updated_by)
    .fetch_one(pool)
    .await
}

pub(crate) async fn update_rule(
    pool: &PgPool,
    tenant_id: &str,
    rule_id: Uuid,
    rule_name: &str,
    pattern_regex: &str,
    entity_type: &str,
    capture_group_index: i32,
    priority: i32,
    updated_by: Option<&str>,
) -> Result<Option<BarcodeFormatRule>, sqlx::Error> {
    let now = Utc::now();
    sqlx::query_as::<_, BarcodeFormatRule>(
        r#"
        UPDATE barcode_format_rules
        SET rule_name = $3,
            pattern_regex = $4,
            entity_type_when_matched = $5,
            capture_group_index = $6,
            priority = $7,
            updated_at = $8,
            updated_by = $9
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(rule_id)
    .bind(tenant_id)
    .bind(rule_name)
    .bind(pattern_regex)
    .bind(entity_type)
    .bind(capture_group_index)
    .bind(priority)
    .bind(now)
    .bind(updated_by)
    .fetch_optional(pool)
    .await
}

pub(crate) async fn deactivate_rule(
    pool: &PgPool,
    tenant_id: &str,
    rule_id: Uuid,
    updated_by: Option<&str>,
) -> Result<Option<BarcodeFormatRule>, sqlx::Error> {
    let now = Utc::now();
    sqlx::query_as::<_, BarcodeFormatRule>(
        r#"
        UPDATE barcode_format_rules
        SET active = FALSE, updated_at = $3, updated_by = $4
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(rule_id)
    .bind(tenant_id)
    .bind(now)
    .bind(updated_by)
    .fetch_optional(pool)
    .await
}

// ============================================================================
// Entity lookups (for Inventory-native entity types)
// ============================================================================

/// Look up an item by SKU within a tenant.
pub(crate) async fn find_item_by_sku(
    pool: &PgPool,
    tenant_id: &str,
    sku: &str,
) -> Result<Option<ItemLookupRow>, sqlx::Error> {
    sqlx::query_as::<_, ItemLookupRow>(
        "SELECT id, sku FROM items WHERE tenant_id = $1 AND sku = $2 AND active = TRUE",
    )
    .bind(tenant_id)
    .bind(sku)
    .fetch_optional(pool)
    .await
}

/// Look up a lot by lot_code within a tenant. Lot codes are per tenant+item,
/// but barcode resolution searches across items within the tenant.
pub(crate) async fn find_lot_by_code(
    pool: &PgPool,
    tenant_id: &str,
    lot_code: &str,
) -> Result<Option<LotLookupRow>, sqlx::Error> {
    sqlx::query_as::<_, LotLookupRow>(
        "SELECT id, lot_code, item_id FROM inventory_lots WHERE tenant_id = $1 AND lot_code = $2 LIMIT 1",
    )
    .bind(tenant_id)
    .bind(lot_code)
    .fetch_optional(pool)
    .await
}

/// Look up a serial instance by serial_code within a tenant.
pub(crate) async fn find_serial_by_code(
    pool: &PgPool,
    tenant_id: &str,
    serial_code: &str,
) -> Result<Option<SerialLookupRow>, sqlx::Error> {
    sqlx::query_as::<_, SerialLookupRow>(
        "SELECT id, serial_code, item_id, status FROM inventory_serial_instances WHERE tenant_id = $1 AND serial_code = $2 LIMIT 1",
    )
    .bind(tenant_id)
    .bind(serial_code)
    .fetch_optional(pool)
    .await
}

// ============================================================================
// Outbox
// ============================================================================

pub(crate) async fn insert_outbox_event(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_id: &str,
    tenant_id: &str,
    envelope_json: &str,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'barcode_resolution', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(envelope_json)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
