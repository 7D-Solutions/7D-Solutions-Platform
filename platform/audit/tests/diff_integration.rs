//! Integration tests for audit diff computation.
//!
//! Verifies that Diff produces accurate before/after snapshots and
//! field-level changes for all mutation scenarios.

use audit::diff::{Diff, FieldChange};
use serde_json::json;

#[test]
fn diff_creation_has_no_field_changes() {
    let diff = Diff::new(None, Some(json!({"id": "1", "name": "New"})));

    assert!(diff.is_creation());
    assert!(!diff.is_deletion());
    assert!(!diff.is_modification());
    assert_eq!(diff.changed_field_count(), 0);
    assert!(diff.before.is_none());
    assert!(diff.after.is_some());
}

#[test]
fn diff_deletion_has_no_field_changes() {
    let diff = Diff::new(Some(json!({"id": "1", "name": "Old"})), None);

    assert!(diff.is_deletion());
    assert!(!diff.is_creation());
    assert_eq!(diff.changed_field_count(), 0);
    assert!(diff.before.is_some());
    assert!(diff.after.is_none());
}

#[test]
fn diff_detects_single_field_modification() {
    let before = json!({"id": "1", "status": "draft"});
    let after = json!({"id": "1", "status": "active"});
    let diff = Diff::new(Some(before), Some(after));

    assert!(diff.is_modification());
    assert_eq!(diff.changed_field_count(), 1);
    assert!(diff.has_field_change("status"));
    assert!(!diff.has_field_change("id"));

    let change = diff.get_field_change("status").unwrap();
    assert!(change.is_modification());
    assert_eq!(change.old_value, Some(json!("draft")));
    assert_eq!(change.new_value, Some(json!("active")));
}

#[test]
fn diff_detects_multiple_field_modifications() {
    let before = json!({"a": 1, "b": 2, "c": 3, "d": 4});
    let after = json!({"a": 10, "b": 2, "c": 30, "d": 4});
    let diff = Diff::new(Some(before), Some(after));

    assert_eq!(diff.changed_field_count(), 2);
    assert!(diff.has_field_change("a"));
    assert!(diff.has_field_change("c"));
    assert!(!diff.has_field_change("b"));
    assert!(!diff.has_field_change("d"));
}

#[test]
fn diff_detects_field_addition() {
    let before = json!({"id": "1"});
    let after = json!({"id": "1", "email": "new@test.com"});
    let diff = Diff::new(Some(before), Some(after));

    assert_eq!(diff.changed_field_count(), 1);
    let change = diff.get_field_change("email").unwrap();
    assert!(change.is_addition());
    assert_eq!(change.old_value, None);
    assert_eq!(change.new_value, Some(json!("new@test.com")));
}

#[test]
fn diff_detects_field_removal() {
    let before = json!({"id": "1", "temp": "gone"});
    let after = json!({"id": "1"});
    let diff = Diff::new(Some(before), Some(after));

    assert_eq!(diff.changed_field_count(), 1);
    let change = diff.get_field_change("temp").unwrap();
    assert!(change.is_removal());
    assert_eq!(change.old_value, Some(json!("gone")));
    assert_eq!(change.new_value, None);
}

#[test]
fn diff_mixed_add_modify_remove() {
    let before = json!({"keep": "same", "modify": "old", "remove": "bye"});
    let after = json!({"keep": "same", "modify": "new", "add": "hello"});
    let diff = Diff::new(Some(before), Some(after));

    assert_eq!(diff.changed_field_count(), 3);
    assert!(diff.get_field_change("modify").unwrap().is_modification());
    assert!(diff.get_field_change("remove").unwrap().is_removal());
    assert!(diff.get_field_change("add").unwrap().is_addition());
    assert!(!diff.has_field_change("keep"));
}

#[test]
fn diff_field_ordering_is_deterministic() {
    let before = json!({"zebra": 1, "apple": 2, "mango": 3});
    let after = json!({"zebra": 10, "apple": 20, "mango": 30});

    // Run multiple times to verify ordering stability
    for _ in 0..10 {
        let diff = Diff::new(Some(before.clone()), Some(after.clone()));
        let names: Vec<&str> = diff
            .field_changes
            .iter()
            .map(|c| c.field.as_str())
            .collect();
        assert_eq!(names, vec!["apple", "mango", "zebra"]);
    }
}

#[test]
fn diff_no_changes_when_identical() {
    let val = json!({"id": "1", "status": "active", "amount": 500});
    let diff = Diff::new(Some(val.clone()), Some(val));

    assert!(diff.is_modification());
    assert_eq!(diff.changed_field_count(), 0);
}

#[test]
fn diff_handles_nested_objects_as_opaque() {
    let before = json!({"id": "1", "meta": {"a": 1}});
    let after = json!({"id": "1", "meta": {"a": 2}});
    let diff = Diff::new(Some(before), Some(after));

    assert_eq!(diff.changed_field_count(), 1);
    let change = diff.get_field_change("meta").unwrap();
    assert_eq!(change.old_value, Some(json!({"a": 1})));
    assert_eq!(change.new_value, Some(json!({"a": 2})));
}

#[test]
fn diff_handles_non_object_values_gracefully() {
    // If before/after are not JSON objects, no field changes produced
    let diff = Diff::new(Some(json!("a string")), Some(json!("another")));
    assert_eq!(diff.changed_field_count(), 0);
}

#[test]
fn diff_none_none_produces_empty() {
    let diff = Diff::new(None, None);
    assert!(!diff.is_creation());
    assert!(!diff.is_deletion());
    assert!(!diff.is_modification());
    assert_eq!(diff.changed_field_count(), 0);
}

#[test]
fn diff_preserves_full_snapshots() {
    let before = json!({"id": "1", "name": "old", "extra": [1, 2, 3]});
    let after = json!({"id": "1", "name": "new", "extra": [1, 2, 3]});
    let diff = Diff::new(Some(before.clone()), Some(after.clone()));

    assert_eq!(diff.before, Some(before));
    assert_eq!(diff.after, Some(after));
}

#[test]
fn field_change_classification() {
    let addition = FieldChange::new("f".into(), None, Some(json!(1)));
    assert!(addition.is_addition());
    assert!(!addition.is_removal());
    assert!(!addition.is_modification());

    let removal = FieldChange::new("f".into(), Some(json!(1)), None);
    assert!(removal.is_removal());
    assert!(!removal.is_addition());
    assert!(!removal.is_modification());

    let modification = FieldChange::new("f".into(), Some(json!(1)), Some(json!(2)));
    assert!(modification.is_modification());
    assert!(!modification.is_addition());
    assert!(!modification.is_removal());
}
