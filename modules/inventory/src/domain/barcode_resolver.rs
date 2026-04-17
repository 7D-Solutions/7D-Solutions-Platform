//! Barcode Resolution Service.
//!
//! Evaluates tenant-scoped format rules against a raw barcode string and returns
//! a typed entity reference. Rules are evaluated in (priority ASC, id ASC) order;
//! the first matching rule wins. Deactivated rules are never evaluated.
//!
//! Invariants:
//! - Resolution is deterministic and tenant-scoped. Same barcode + same active
//!   rules + same tenant always produces the same output.
//! - Rules are never leaked across tenants.
//! - Inventory-native entity types (item/lot/serial) are verified against
//!   existing records. If no record exists, resolved=false is returned.
//! - Cross-module types (work_order/operation/badge/other) return the captured
//!   reference string; the caller is responsible for validating existence.
//! - This service never creates or mutates inventory records.

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::barcode_resolver_repo::{self, BarcodeFormatRule};
use crate::events::{
    build_barcode_resolved_envelope, BarcodeResolvedPayload, EVENT_TYPE_BARCODE_RESOLVED,
};

pub use crate::domain::barcode_resolver_repo::BarcodeFormatRule as BarcodeFormatRuleModel;

// ============================================================================
// Constants
// ============================================================================

pub const ENTITY_TYPE_WORK_ORDER: &str = "work_order";
pub const ENTITY_TYPE_OPERATION: &str = "operation";
pub const ENTITY_TYPE_ITEM: &str = "item";
pub const ENTITY_TYPE_LOT: &str = "lot";
pub const ENTITY_TYPE_SERIAL: &str = "serial";
pub const ENTITY_TYPE_BADGE: &str = "badge";
pub const ENTITY_TYPE_OTHER: &str = "other";

const VALID_ENTITY_TYPES: &[&str] = &[
    ENTITY_TYPE_WORK_ORDER,
    ENTITY_TYPE_OPERATION,
    ENTITY_TYPE_ITEM,
    ENTITY_TYPE_LOT,
    ENTITY_TYPE_SERIAL,
    ENTITY_TYPE_BADGE,
    ENTITY_TYPE_OTHER,
];

pub const BATCH_MAX_BARCODES: usize = 100;
pub const BARCODE_MAX_LEN: usize = 256;

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum BarcodeError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Invalid regex pattern: {0}")]
    InvalidRegex(String),

    #[error("Rule not found")]
    RuleNotFound,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateRuleRequest {
    pub tenant_id: String,
    pub rule_name: String,
    pub pattern_regex: String,
    pub entity_type_when_matched: String,
    #[serde(default)]
    pub capture_group_index: i32,
    #[serde(default = "default_priority")]
    pub priority: i32,
    pub updated_by: Option<String>,
}

fn default_priority() -> i32 {
    100
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UpdateRuleRequest {
    pub tenant_id: String,
    pub rule_name: String,
    pub pattern_regex: String,
    pub entity_type_when_matched: String,
    pub capture_group_index: i32,
    pub priority: i32,
    pub updated_by: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResolveRequest {
    pub tenant_id: String,
    pub barcode_raw: String,
    pub resolved_by: Option<String>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BatchResolveRequest {
    pub tenant_id: String,
    pub barcodes: Vec<String>,
    pub resolved_by: Option<String>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestRuleRequest {
    pub tenant_id: String,
    pub barcode_raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResolveResult {
    pub barcode_raw: String,
    pub resolved: bool,
    pub entity_type: Option<String>,
    /// The resolved entity ID (for Inventory-native types with a verified record).
    pub entity_id: Option<Uuid>,
    /// The decoded reference string (for all matched types).
    pub entity_ref: Option<String>,
    pub matched_rule_id: Option<Uuid>,
    pub matched_rule_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestRuleResult {
    pub barcode_raw: String,
    pub matched: bool,
    pub matched_rule_id: Option<Uuid>,
    pub matched_rule_name: Option<String>,
    pub entity_type: Option<String>,
    pub captured_ref: Option<String>,
}

// ============================================================================
// Validation
// ============================================================================

fn validate_regex(pattern: &str) -> Result<Regex, BarcodeError> {
    Regex::new(pattern).map_err(|e| BarcodeError::InvalidRegex(e.to_string()))
}

fn validate_entity_type(entity_type: &str) -> Result<(), BarcodeError> {
    if !VALID_ENTITY_TYPES.contains(&entity_type) {
        return Err(BarcodeError::Validation(format!(
            "entity_type_when_matched must be one of: {}",
            VALID_ENTITY_TYPES.join(", ")
        )));
    }
    Ok(())
}

fn validate_create_rule(req: &CreateRuleRequest) -> Result<Regex, BarcodeError> {
    if req.tenant_id.trim().is_empty() {
        return Err(BarcodeError::Validation("tenant_id is required".to_string()));
    }
    if req.rule_name.trim().is_empty() {
        return Err(BarcodeError::Validation("rule_name is required".to_string()));
    }
    validate_entity_type(&req.entity_type_when_matched)?;
    if req.capture_group_index < 0 {
        return Err(BarcodeError::Validation(
            "capture_group_index must be >= 0".to_string(),
        ));
    }
    validate_regex(&req.pattern_regex)
}

fn validate_update_rule(req: &UpdateRuleRequest) -> Result<Regex, BarcodeError> {
    if req.rule_name.trim().is_empty() {
        return Err(BarcodeError::Validation("rule_name is required".to_string()));
    }
    validate_entity_type(&req.entity_type_when_matched)?;
    if req.capture_group_index < 0 {
        return Err(BarcodeError::Validation(
            "capture_group_index must be >= 0".to_string(),
        ));
    }
    validate_regex(&req.pattern_regex)
}

// ============================================================================
// Rule resolution logic (pure, no I/O)
// ============================================================================

/// Apply a single rule's regex to the barcode. Returns the captured reference
/// string if it matches; None if no match or capture group index out of range.
fn apply_rule(rule: &BarcodeFormatRule, barcode: &str) -> Option<String> {
    let re = Regex::new(&rule.pattern_regex).ok()?;
    let caps = re.captures(barcode)?;
    let idx = rule.capture_group_index as usize;
    caps.get(idx).map(|m| m.as_str().to_string())
}

// ============================================================================
// Rule CRUD
// ============================================================================

pub async fn create_rule(
    pool: &PgPool,
    req: &CreateRuleRequest,
) -> Result<BarcodeFormatRule, BarcodeError> {
    validate_create_rule(req)?;
    let rule = barcode_resolver_repo::insert_rule(
        pool,
        &req.tenant_id,
        &req.rule_name,
        &req.pattern_regex,
        &req.entity_type_when_matched,
        req.capture_group_index,
        req.priority,
        req.updated_by.as_deref(),
    )
    .await?;
    Ok(rule)
}

pub async fn update_rule(
    pool: &PgPool,
    tenant_id: &str,
    rule_id: Uuid,
    req: &UpdateRuleRequest,
) -> Result<BarcodeFormatRule, BarcodeError> {
    validate_update_rule(req)?;
    let rule = barcode_resolver_repo::update_rule(
        pool,
        tenant_id,
        rule_id,
        &req.rule_name,
        &req.pattern_regex,
        &req.entity_type_when_matched,
        req.capture_group_index,
        req.priority,
        req.updated_by.as_deref(),
    )
    .await?
    .ok_or(BarcodeError::RuleNotFound)?;
    Ok(rule)
}

pub async fn deactivate_rule(
    pool: &PgPool,
    tenant_id: &str,
    rule_id: Uuid,
    updated_by: Option<&str>,
) -> Result<BarcodeFormatRule, BarcodeError> {
    let rule =
        barcode_resolver_repo::deactivate_rule(pool, tenant_id, rule_id, updated_by).await?;
    rule.ok_or(BarcodeError::RuleNotFound)
}

pub async fn list_rules(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<BarcodeFormatRule>, BarcodeError> {
    Ok(barcode_resolver_repo::list_all_rules(pool, tenant_id).await?)
}

// ============================================================================
// Resolution
// ============================================================================

/// Resolve a single barcode to an entity reference.
///
/// This is the core resolution function. It:
/// 1. Loads active rules for the tenant (priority order).
/// 2. Evaluates each rule's regex against the barcode.
/// 3. On first match, for Inventory-native types, verifies the entity exists.
/// 4. Emits an outbox event.
/// 5. Returns the resolution result.
pub async fn resolve(
    pool: &PgPool,
    req: &ResolveRequest,
) -> Result<ResolveResult, BarcodeError> {
    let rules = barcode_resolver_repo::list_active_rules(pool, &req.tenant_id).await?;

    let mut result = evaluate_rules(&rules, &req.barcode_raw);

    // For Inventory-native types, verify the entity exists
    if result.resolved {
        if let (Some(entity_type), Some(ref entity_ref)) =
            (&result.entity_type, &result.entity_ref)
        {
            match entity_type.as_str() {
                ENTITY_TYPE_ITEM => {
                    match barcode_resolver_repo::find_item_by_sku(
                        pool,
                        &req.tenant_id,
                        entity_ref,
                    )
                    .await?
                    {
                        Some(row) => {
                            result.entity_id = Some(row.id);
                        }
                        None => {
                            result.resolved = false;
                            result.entity_id = None;
                            result.error = Some(format!(
                                "No item found with SKU '{}' for this tenant",
                                entity_ref
                            ));
                        }
                    }
                }
                ENTITY_TYPE_LOT => {
                    match barcode_resolver_repo::find_lot_by_code(
                        pool,
                        &req.tenant_id,
                        entity_ref,
                    )
                    .await?
                    {
                        Some(row) => {
                            result.entity_id = Some(row.id);
                        }
                        None => {
                            result.resolved = false;
                            result.entity_id = None;
                            result.error = Some(format!(
                                "No lot found with code '{}' for this tenant",
                                entity_ref
                            ));
                        }
                    }
                }
                ENTITY_TYPE_SERIAL => {
                    match barcode_resolver_repo::find_serial_by_code(
                        pool,
                        &req.tenant_id,
                        entity_ref,
                    )
                    .await?
                    {
                        Some(row) => {
                            result.entity_id = Some(row.id);
                        }
                        None => {
                            result.resolved = false;
                            result.entity_id = None;
                            result.error = Some(format!(
                                "No serial instance found with code '{}' for this tenant",
                                entity_ref
                            ));
                        }
                    }
                }
                // Cross-module types: return the reference string, entity_id stays None
                _ => {}
            }
        }
    }

    // Emit outbox event
    emit_barcode_resolved_event(pool, req, &result).await?;

    Ok(result)
}

/// Resolve a batch of barcodes. Returns one ResolveResult per input barcode.
///
/// The batch cap (100 barcodes, 256 chars each) is enforced at the HTTP layer
/// before this function is called.
pub async fn resolve_batch(
    pool: &PgPool,
    req: &BatchResolveRequest,
) -> Result<Vec<ResolveResult>, BarcodeError> {
    let rules = barcode_resolver_repo::list_active_rules(pool, &req.tenant_id).await?;

    let mut results = Vec::with_capacity(req.barcodes.len());

    for barcode_raw in &req.barcodes {
        let mut result = evaluate_rules(&rules, barcode_raw);

        if result.resolved {
            if let (Some(entity_type), Some(ref entity_ref)) =
                (&result.entity_type, &result.entity_ref)
            {
                match entity_type.as_str() {
                    ENTITY_TYPE_ITEM => {
                        match barcode_resolver_repo::find_item_by_sku(
                            pool,
                            &req.tenant_id,
                            entity_ref,
                        )
                        .await?
                        {
                            Some(row) => result.entity_id = Some(row.id),
                            None => {
                                result.resolved = false;
                                result.entity_id = None;
                                result.error = Some(format!(
                                    "No item found with SKU '{}' for this tenant",
                                    entity_ref
                                ));
                            }
                        }
                    }
                    ENTITY_TYPE_LOT => {
                        match barcode_resolver_repo::find_lot_by_code(
                            pool,
                            &req.tenant_id,
                            entity_ref,
                        )
                        .await?
                        {
                            Some(row) => result.entity_id = Some(row.id),
                            None => {
                                result.resolved = false;
                                result.entity_id = None;
                                result.error = Some(format!(
                                    "No lot found with code '{}' for this tenant",
                                    entity_ref
                                ));
                            }
                        }
                    }
                    ENTITY_TYPE_SERIAL => {
                        match barcode_resolver_repo::find_serial_by_code(
                            pool,
                            &req.tenant_id,
                            entity_ref,
                        )
                        .await?
                        {
                            Some(row) => result.entity_id = Some(row.id),
                            None => {
                                result.resolved = false;
                                result.entity_id = None;
                                result.error = Some(format!(
                                    "No serial found with code '{}' for this tenant",
                                    entity_ref
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let single_req = ResolveRequest {
            tenant_id: req.tenant_id.clone(),
            barcode_raw: barcode_raw.clone(),
            resolved_by: req.resolved_by.clone(),
            correlation_id: req.correlation_id.clone(),
            causation_id: req.causation_id.clone(),
        };
        emit_barcode_resolved_event(pool, &single_req, &result).await?;

        results.push(result);
    }

    Ok(results)
}

/// Test a barcode against current active rules without persisting or emitting events.
pub async fn test_barcode(
    pool: &PgPool,
    req: &TestRuleRequest,
) -> Result<TestRuleResult, BarcodeError> {
    let rules = barcode_resolver_repo::list_active_rules(pool, &req.tenant_id).await?;
    let result = evaluate_rules(&rules, &req.barcode_raw);

    Ok(TestRuleResult {
        barcode_raw: req.barcode_raw.clone(),
        matched: result.resolved,
        matched_rule_id: result.matched_rule_id,
        matched_rule_name: result.matched_rule_name,
        entity_type: result.entity_type,
        captured_ref: result.entity_ref,
    })
}

// ============================================================================
// Pure evaluation (no I/O)
// ============================================================================

fn evaluate_rules(rules: &[BarcodeFormatRule], barcode: &str) -> ResolveResult {
    for rule in rules {
        if let Some(captured_ref) = apply_rule(rule, barcode) {
            return ResolveResult {
                barcode_raw: barcode.to_string(),
                resolved: true,
                entity_type: Some(rule.entity_type_when_matched.clone()),
                entity_id: None,
                entity_ref: Some(captured_ref),
                matched_rule_id: Some(rule.id),
                matched_rule_name: Some(rule.rule_name.clone()),
                error: None,
            };
        }
    }

    ResolveResult {
        barcode_raw: barcode.to_string(),
        resolved: false,
        entity_type: None,
        entity_id: None,
        entity_ref: None,
        matched_rule_id: None,
        matched_rule_name: None,
        error: Some("No matching barcode rule found".to_string()),
    }
}

// ============================================================================
// Event emission
// ============================================================================

async fn emit_barcode_resolved_event(
    pool: &PgPool,
    req: &ResolveRequest,
    result: &ResolveResult,
) -> Result<(), BarcodeError> {
    let event_id = Uuid::new_v4();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let payload = BarcodeResolvedPayload {
        barcode_raw: req.barcode_raw.clone(),
        entity_type: result.entity_type.clone(),
        resolved: result.resolved,
        matched_rule_id: result.matched_rule_id,
        resolved_by: req.resolved_by.clone(),
        resolved_at: Utc::now(),
    };

    let envelope = build_barcode_resolved_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    let mut tx = pool.begin().await?;
    barcode_resolver_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_BARCODE_RESOLVED,
        &req.barcode_raw,
        &req.tenant_id,
        &envelope_json,
        &correlation_id,
        req.causation_id.as_deref(),
    )
    .await?;
    tx.commit().await?;

    Ok(())
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(
        id: Uuid,
        pattern: &str,
        entity_type: &str,
        capture_group: i32,
        priority: i32,
    ) -> BarcodeFormatRule {
        BarcodeFormatRule {
            id,
            tenant_id: "t1".to_string(),
            rule_name: format!("rule-{}", id),
            pattern_regex: pattern.to_string(),
            entity_type_when_matched: entity_type.to_string(),
            capture_group_index: capture_group,
            priority,
            active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: None,
        }
    }

    #[test]
    fn evaluate_rules_first_match_wins() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let rules = vec![
            make_rule(id1, r"^WO-(\d+)$", "work_order", 1, 10),
            make_rule(id2, r"^WO-", "work_order", 0, 20),
        ];
        let result = evaluate_rules(&rules, "WO-12345");
        assert!(result.resolved);
        assert_eq!(result.matched_rule_id, Some(id1));
        assert_eq!(result.entity_ref.as_deref(), Some("12345"));
    }

    #[test]
    fn evaluate_rules_no_match_returns_unresolved() {
        let rules = vec![make_rule(Uuid::new_v4(), r"^WO-\d+$", "work_order", 0, 10)];
        let result = evaluate_rules(&rules, "LOT-ABC");
        assert!(!result.resolved);
        assert!(result.matched_rule_id.is_none());
        assert!(result.entity_ref.is_none());
    }

    #[test]
    fn evaluate_rules_empty_rules_returns_unresolved() {
        let result = evaluate_rules(&[], "anything");
        assert!(!result.resolved);
    }

    #[test]
    fn evaluate_rules_priority_order_respected() {
        let id_low = Uuid::from_u128(1);
        let id_high = Uuid::from_u128(2);
        // Rules are pre-sorted by (priority ASC, id ASC) as the DB query delivers them.
        // priority=5 comes before priority=10.
        let mut rules = vec![
            make_rule(id_high, r"^LOT-(.+)$", "lot", 1, 10),
            make_rule(id_low, r"^LOT-(.+)$", "lot", 1, 5),
        ];
        rules.sort_by_key(|r| (r.priority, r.id));
        let result = evaluate_rules(&rules, "LOT-XYZ");
        assert_eq!(result.matched_rule_id, Some(id_low));
    }

    #[test]
    fn apply_rule_capture_group_zero_returns_full_match() {
        let rule = make_rule(Uuid::new_v4(), r"^ITEM-[A-Z0-9-]+$", "item", 0, 10);
        let captured = apply_rule(&rule, "ITEM-SKU-001");
        assert_eq!(captured, Some("ITEM-SKU-001".to_string()));
    }

    #[test]
    fn apply_rule_invalid_regex_returns_none() {
        let mut rule = make_rule(Uuid::new_v4(), r"^ITEM-\w+$", "item", 0, 10);
        rule.pattern_regex = r"[invalid".to_string();
        let result = apply_rule(&rule, "anything");
        assert!(result.is_none());
    }

    #[test]
    fn apply_rule_capture_group_out_of_range_returns_none() {
        let rule = make_rule(Uuid::new_v4(), r"^LOT-(.+)$", "lot", 5, 10);
        let result = apply_rule(&rule, "LOT-ABC");
        assert!(result.is_none());
    }

    #[test]
    fn validate_create_rule_invalid_regex_rejected() {
        let req = CreateRuleRequest {
            tenant_id: "t1".to_string(),
            rule_name: "test".to_string(),
            pattern_regex: "[invalid".to_string(),
            entity_type_when_matched: "item".to_string(),
            capture_group_index: 0,
            priority: 10,
            updated_by: None,
        };
        assert!(matches!(validate_create_rule(&req), Err(BarcodeError::InvalidRegex(_))));
    }

    #[test]
    fn validate_create_rule_invalid_entity_type_rejected() {
        let req = CreateRuleRequest {
            tenant_id: "t1".to_string(),
            rule_name: "test".to_string(),
            pattern_regex: r"^LOT-\w+$".to_string(),
            entity_type_when_matched: "pallet".to_string(),
            capture_group_index: 0,
            priority: 10,
            updated_by: None,
        };
        assert!(matches!(validate_create_rule(&req), Err(BarcodeError::Validation(_))));
    }

    #[test]
    fn validate_create_rule_valid_passes() {
        let req = CreateRuleRequest {
            tenant_id: "t1".to_string(),
            rule_name: "work order rule".to_string(),
            pattern_regex: r"^WO-(\d+)$".to_string(),
            entity_type_when_matched: "work_order".to_string(),
            capture_group_index: 1,
            priority: 10,
            updated_by: None,
        };
        assert!(validate_create_rule(&req).is_ok());
    }
}
