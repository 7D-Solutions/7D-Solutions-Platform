use super::types::{Annotation, CURRENT_ANNOTATION_SCHEMA_VERSION};

#[test]
fn annotation_schema_version_explicit_integer() {
    let json = r#"{"id":"t1","x":1.0,"y":2.0,"pageNumber":1,"type":"TEXT","schemaVersion":1}"#;
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, 1);
    ann.validate_schema_version().expect("version 1 must be valid");
}

#[test]
fn annotation_schema_version_absent_defaults_to_current() {
    let json = r#"{"id":"t1","x":1.0,"y":2.0,"pageNumber":1,"type":"TEXT"}"#;
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, CURRENT_ANNOTATION_SCHEMA_VERSION);
    ann.validate_schema_version().expect("default version must be valid");
}

#[test]
fn annotation_schema_version_snake_case_wire_name_ignored() {
    // "schema_version" (snake_case) is NOT the wire name — "schemaVersion" (camelCase) is.
    // serde ignores unknown keys, so schema_version stays at the default.
    let json = r#"{"id":"t1","x":1.0,"y":2.0,"pageNumber":1,"type":"TEXT","schema_version":99}"#;
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, CURRENT_ANNOTATION_SCHEMA_VERSION);
}

#[test]
fn annotation_schema_version_unsupported_returns_error() {
    // schema_version=99 is out of range — validate_schema_version must return Err
    let json = r#"{"id":"t1","x":1.0,"y":2.0,"pageNumber":1,"type":"TEXT","schemaVersion":99}"#;
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, 99);
    let err = ann.validate_schema_version().unwrap_err();
    assert_eq!(err.version, 99);
    assert!(err.to_string().contains("unsupported annotation schema_version=99"));
}
