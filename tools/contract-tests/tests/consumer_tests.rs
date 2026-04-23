use contract_tests::{validate_consumer_schema, validate_openapi_spec, ConsumerSchema};
use std::path::PathBuf;

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("contracts")
}

// Static map from consumer directory name to platform spec path (relative to contracts/).
// Add new consumers here when new frontend repos declare contract expectations.
fn spec_for_consumer(consumer_dir: &str) -> Option<&'static str> {
    match consumer_dir {
        "pdf-creation" => Some("pdf-editor/pdf-editor-v0.1.0.yaml"),
        _ => None,
    }
}

#[test]
fn consumer_contracts_pass() {
    let consumers_dir = contracts_dir().join("consumers");
    let pattern = consumers_dir.to_string_lossy().to_string() + "/**/*.json";

    let mut validated = 0;
    for entry in glob::glob(&pattern).expect("Failed to read glob pattern") {
        let path = entry.expect("Glob error");

        let rel = path
            .strip_prefix(&consumers_dir)
            .expect("Strip prefix failed");
        let consumer_dir = rel
            .components()
            .next()
            .and_then(|c| c.as_os_str().to_str())
            .expect("Cannot determine consumer directory");

        let spec_rel = spec_for_consumer(consumer_dir)
            .unwrap_or_else(|| panic!("No spec mapping for consumer dir: {}", consumer_dir));

        let spec_path = contracts_dir().join(spec_rel);
        let platform_spec = validate_openapi_spec(&spec_path)
            .unwrap_or_else(|e| panic!("Failed to load spec {}: {}", spec_rel, e));

        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));
        let consumer: ConsumerSchema = serde_json::from_str(&contents)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {}", path.display(), e));

        validate_consumer_schema(&platform_spec, &consumer)
            .unwrap_or_else(|e| panic!("Contract failed for {}: {}", path.display(), e));

        println!("✓ {}", path.display());
        validated += 1;
    }

    assert!(validated > 0, "No consumer contracts found under contracts/consumers/");
}

#[test]
fn consumer_contract_fails_on_missing_field() {
    let spec_path = contracts_dir().join("pdf-editor/pdf-editor-v0.1.0.yaml");
    let mut platform_spec =
        validate_openapi_spec(&spec_path).expect("Failed to load pdf-editor spec");

    platform_spec
        .get_mut("components")
        .and_then(|c| c.get_mut("schemas"))
        .and_then(|s| s.get_mut("FormTemplate"))
        .and_then(|t| t.get_mut("properties"))
        .and_then(|p| p.as_object_mut())
        .expect("FormTemplate.properties not found in spec")
        .remove("name");

    let consumer_path =
        contracts_dir().join("consumers/pdf-creation/pdf-editor-get-template.json");
    let contents =
        std::fs::read_to_string(&consumer_path).expect("Failed to read seed consumer schema");
    let consumer: ConsumerSchema =
        serde_json::from_str(&contents).expect("Failed to parse seed consumer schema");

    let result = validate_consumer_schema(&platform_spec, &consumer);
    assert!(
        result.is_err(),
        "Expected validation failure when required field 'name' is missing from spec"
    );
}
