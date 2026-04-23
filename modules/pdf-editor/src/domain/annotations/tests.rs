use super::types::Annotation;

#[test]
fn schema_version_explicit() {
    let json = "{\"id\":\"t1\",\"x\":1.0,\"y\":2.0,\"pageNumber\":1,\"type\":\"TEXT\",\"schemaVersion\":\"1.0\"}";
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, "1.0");
}

#[test]
fn schema_version_absent_defaults_to_1_0() {
    let json = "{\"id\":\"t1\",\"x\":1.0,\"y\":2.0,\"pageNumber\":1,\"type\":\"TEXT\"}";
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, "1.0");
}

#[test]
fn schema_version_snake_case_wire_name_ignored() {
    // "schema_version" (snake_case) is not the wire name — "schemaVersion" (camelCase) is.
    // serde ignores unknown keys, so schema_version field stays at default "1.0".
    let json = "{\"id\":\"t1\",\"x\":1.0,\"y\":2.0,\"pageNumber\":1,\"type\":\"TEXT\",\"schema_version\":\"2.0\"}";
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, "1.0");
}
