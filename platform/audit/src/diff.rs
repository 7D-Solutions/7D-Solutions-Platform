//! Audit diff (before/after state changes)
//!
//! Provides field-level diff capture for mutable_with_audit entities.
//! Diffs are deterministic (stable ordering) and include complete metadata.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Field-level change record
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldChange {
    /// Field name
    pub field: String,
    /// Value before change (None for creation)
    pub old_value: Option<Value>,
    /// Value after change (None for deletion)
    pub new_value: Option<Value>,
}

impl FieldChange {
    pub fn new(field: String, old_value: Option<Value>, new_value: Option<Value>) -> Self {
        Self {
            field,
            old_value,
            new_value,
        }
    }

    pub fn is_addition(&self) -> bool {
        self.old_value.is_none() && self.new_value.is_some()
    }

    pub fn is_removal(&self) -> bool {
        self.old_value.is_some() && self.new_value.is_none()
    }

    pub fn is_modification(&self) -> bool {
        self.old_value.is_some() && self.new_value.is_some()
    }
}

/// Complete audit diff with field-level changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diff {
    /// Full before state (optional)
    pub before: Option<Value>,
    /// Full after state (optional)
    pub after: Option<Value>,
    /// Field-level changes (deterministically ordered)
    pub field_changes: Vec<FieldChange>,
}

impl Diff {
    pub fn new(before: Option<Value>, after: Option<Value>) -> Self {
        let field_changes = Self::compute_field_changes(&before, &after);
        Self {
            before,
            after,
            field_changes,
        }
    }

    /// Create a diff with explicit field changes (for testing)
    pub fn with_field_changes(
        before: Option<Value>,
        after: Option<Value>,
        field_changes: Vec<FieldChange>,
    ) -> Self {
        Self {
            before,
            after,
            field_changes,
        }
    }

    pub fn is_creation(&self) -> bool {
        self.before.is_none() && self.after.is_some()
    }

    pub fn is_deletion(&self) -> bool {
        self.before.is_some() && self.after.is_none()
    }

    pub fn is_modification(&self) -> bool {
        self.before.is_some() && self.after.is_some()
    }

    /// Compute field-level changes from before/after snapshots
    ///
    /// Returns a deterministically ordered (by field name) list of changes.
    /// Only top-level fields are compared (no deep recursive diff).
    fn compute_field_changes(before: &Option<Value>, after: &Option<Value>) -> Vec<FieldChange> {
        match (before, after) {
            (None, None) => vec![],
            (None, Some(_)) => {
                // Creation: all fields in 'after' are additions
                vec![]
            }
            (Some(_), None) => {
                // Deletion: all fields in 'before' are removals
                vec![]
            }
            (Some(before_val), Some(after_val)) => {
                // Modification: compute field-level diff
                Self::diff_objects(before_val, after_val)
            }
        }
    }

    /// Diff two JSON objects at the top level
    ///
    /// Returns a deterministically ordered list of field changes.
    fn diff_objects(before: &Value, after: &Value) -> Vec<FieldChange> {
        let before_obj = match before.as_object() {
            Some(obj) => obj,
            None => return vec![],
        };

        let after_obj = match after.as_object() {
            Some(obj) => obj,
            None => return vec![],
        };

        // Use BTreeMap for deterministic ordering
        let mut changes: BTreeMap<String, FieldChange> = BTreeMap::new();

        // Find removed and modified fields
        for (key, before_value) in before_obj {
            if let Some(after_value) = after_obj.get(key) {
                // Field exists in both: check if modified
                if before_value != after_value {
                    changes.insert(
                        key.clone(),
                        FieldChange::new(
                            key.clone(),
                            Some(before_value.clone()),
                            Some(after_value.clone()),
                        ),
                    );
                }
            } else {
                // Field removed
                changes.insert(
                    key.clone(),
                    FieldChange::new(key.clone(), Some(before_value.clone()), None),
                );
            }
        }

        // Find added fields
        for (key, after_value) in after_obj {
            if !before_obj.contains_key(key) {
                changes.insert(
                    key.clone(),
                    FieldChange::new(key.clone(), None, Some(after_value.clone())),
                );
            }
        }

        // Return as deterministically ordered Vec
        changes.into_values().collect()
    }

    /// Get count of changed fields
    pub fn changed_field_count(&self) -> usize {
        self.field_changes.len()
    }

    /// Check if a specific field was changed
    pub fn has_field_change(&self, field: &str) -> bool {
        self.field_changes.iter().any(|c| c.field == field)
    }

    /// Get the change for a specific field
    pub fn get_field_change(&self, field: &str) -> Option<&FieldChange> {
        self.field_changes.iter().find(|c| c.field == field)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_diff_creation() {
        let before = None;
        let after = Some(json!({ "id": "123", "status": "active" }));
        let diff = Diff::new(before, after);

        assert!(diff.is_creation());
        assert!(!diff.is_deletion());
        assert!(!diff.is_modification());
        assert_eq!(diff.changed_field_count(), 0); // Creation has no field changes
    }

    #[test]
    fn test_diff_deletion() {
        let before = Some(json!({ "id": "123", "status": "active" }));
        let after = None;
        let diff = Diff::new(before, after);

        assert!(diff.is_deletion());
        assert!(!diff.is_creation());
        assert!(!diff.is_modification());
        assert_eq!(diff.changed_field_count(), 0); // Deletion has no field changes
    }

    #[test]
    fn test_diff_modification_single_field() {
        let before = Some(json!({ "id": "123", "status": "draft" }));
        let after = Some(json!({ "id": "123", "status": "active" }));
        let diff = Diff::new(before, after);

        assert!(diff.is_modification());
        assert_eq!(diff.changed_field_count(), 1);

        let change = diff.get_field_change("status").unwrap();
        assert_eq!(change.field, "status");
        assert_eq!(change.old_value, Some(json!("draft")));
        assert_eq!(change.new_value, Some(json!("active")));
        assert!(change.is_modification());
    }

    #[test]
    fn test_diff_modification_multiple_fields() {
        let before = Some(json!({
            "id": "123",
            "status": "draft",
            "amount_cents": 1000,
            "customer_id": "cust_1"
        }));
        let after = Some(json!({
            "id": "123",
            "status": "active",
            "amount_cents": 2000,
            "customer_id": "cust_1"
        }));
        let diff = Diff::new(before, after);

        assert!(diff.is_modification());
        assert_eq!(diff.changed_field_count(), 2);

        assert!(diff.has_field_change("status"));
        assert!(diff.has_field_change("amount_cents"));
        assert!(!diff.has_field_change("id"));
        assert!(!diff.has_field_change("customer_id"));
    }

    #[test]
    fn test_diff_field_addition() {
        let before = Some(json!({ "id": "123", "status": "draft" }));
        let after = Some(json!({
            "id": "123",
            "status": "draft",
            "new_field": "value"
        }));
        let diff = Diff::new(before, after);

        assert_eq!(diff.changed_field_count(), 1);

        let change = diff.get_field_change("new_field").unwrap();
        assert!(change.is_addition());
        assert_eq!(change.old_value, None);
        assert_eq!(change.new_value, Some(json!("value")));
    }

    #[test]
    fn test_diff_field_removal() {
        let before = Some(json!({
            "id": "123",
            "status": "draft",
            "removed_field": "old_value"
        }));
        let after = Some(json!({ "id": "123", "status": "draft" }));
        let diff = Diff::new(before, after);

        assert_eq!(diff.changed_field_count(), 1);

        let change = diff.get_field_change("removed_field").unwrap();
        assert!(change.is_removal());
        assert_eq!(change.old_value, Some(json!("old_value")));
        assert_eq!(change.new_value, None);
    }

    #[test]
    fn test_diff_deterministic_ordering() {
        // Test that field changes are deterministically ordered
        let before = Some(json!({
            "zebra": "1",
            "apple": "2",
            "middle": "3"
        }));
        let after = Some(json!({
            "zebra": "10",
            "apple": "20",
            "middle": "30"
        }));
        let diff = Diff::new(before, after);

        // BTreeMap ensures lexicographic ordering
        let field_names: Vec<&str> = diff
            .field_changes
            .iter()
            .map(|c| c.field.as_str())
            .collect();
        assert_eq!(field_names, vec!["apple", "middle", "zebra"]);
    }

    #[test]
    fn test_diff_no_changes() {
        let before = Some(json!({ "id": "123", "status": "active" }));
        let after = Some(json!({ "id": "123", "status": "active" }));
        let diff = Diff::new(before.clone(), after.clone());

        assert!(diff.is_modification());
        assert_eq!(diff.changed_field_count(), 0);
    }

    #[test]
    fn test_diff_complex_values() {
        // Test with nested objects (should treat as single field change)
        let before = Some(json!({
            "id": "123",
            "metadata": { "nested": "old" }
        }));
        let after = Some(json!({
            "id": "123",
            "metadata": { "nested": "new" }
        }));
        let diff = Diff::new(before, after);

        assert_eq!(diff.changed_field_count(), 1);
        let change = diff.get_field_change("metadata").unwrap();
        assert_eq!(change.old_value, Some(json!({ "nested": "old" })));
        assert_eq!(change.new_value, Some(json!({ "nested": "new" })));
    }
}
