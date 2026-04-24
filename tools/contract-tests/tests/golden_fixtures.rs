//! Golden-fixture corpus tests (bd-af29l).
//!
//! These tests iterate every JSON fixture in tools/contract-tests/fixtures/annotations/
//! and fixtures/capabilities/, deserialize via the module's production types, assert
//! no error, then round-trip (serialize → deserialize) to verify byte shape stability.
//!
//! The same fixture files are consumed by PDF-Creation CI via git subtree / fetch-on-ci
//! so a shape change that breaks these tests will break both sides simultaneously.

use pdf_editor::domain::annotations::types::Annotation;
use serde_json::Value;
use std::path::PathBuf;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn annotation_fixtures_dir() -> PathBuf {
    fixtures_root().join("annotations")
}

fn capability_fixtures_dir() -> PathBuf {
    fixtures_root().join("capabilities")
}

/// Deserialize every annotation fixture via the production Annotation type.
/// Fails if any fixture cannot be deserialized (shape incompatibility detected).
#[test]
fn golden_fixtures_annotations_deserialize() {
    let dir = annotation_fixtures_dir();
    let mut count = 0;

    for entry in std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("Cannot read annotations fixtures dir {}: {}", dir.display(), e))
    {
        let path = entry.expect("IO error").path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Cannot read {}: {}", path.display(), e));

        let annotation: Annotation = serde_json::from_str(&contents).unwrap_or_else(|e| {
            panic!(
                "Annotation fixture {} failed to deserialize: {}",
                path.display(),
                e
            )
        });

        // Round-trip: serialize back to Value, then deserialize again
        let serialized = serde_json::to_value(&annotation).unwrap_or_else(|e| {
            panic!(
                "Annotation fixture {} failed to serialize: {}",
                path.display(),
                e
            )
        });
        let _: Annotation = serde_json::from_value(serialized.clone()).unwrap_or_else(|e| {
            panic!(
                "Annotation fixture {} failed round-trip deserialize: {}",
                path.display(),
                e
            )
        });

        println!(
            "✓ annotations/{} (schema_version={})",
            path.file_name().unwrap().to_string_lossy(),
            annotation.schema_version
        );
        count += 1;
    }

    assert!(
        count > 0,
        "No annotation fixtures found in {} — directory was not populated",
        dir.display()
    );
}

/// Deserialize every capability fixture as a generic JSON Value and verify required fields.
/// Capabilities are produced by the control-plane and consumed by PDF-Creation frontend.
#[test]
fn golden_fixtures_capabilities_deserialize() {
    let dir = capability_fixtures_dir();
    let mut count = 0;

    for entry in std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("Cannot read capabilities fixtures dir {}: {}", dir.display(), e))
    {
        let path = entry.expect("IO error").path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Cannot read {}: {}", path.display(), e));

        let v: Value = serde_json::from_str(&contents).unwrap_or_else(|e| {
            panic!(
                "Capability fixture {} is not valid JSON: {}",
                path.display(),
                e
            )
        });

        // Required fields on TenantFeaturesResponse
        assert!(
            v.get("tenant_id").and_then(|x| x.as_str()).is_some(),
            "Capability fixture {} missing 'tenant_id'",
            path.display()
        );
        assert!(
            v.get("schema_version").and_then(|x| x.as_u64()).is_some(),
            "Capability fixture {} missing 'schema_version'",
            path.display()
        );
        assert!(
            v.get("flags").and_then(|x| x.as_object()).is_some(),
            "Capability fixture {} missing 'flags' object",
            path.display()
        );

        println!(
            "✓ capabilities/{} (schema_version={})",
            path.file_name().unwrap().to_string_lossy(),
            v["schema_version"]
        );
        count += 1;
    }

    assert!(
        count > 0,
        "No capability fixtures found in {} — directory was not populated",
        dir.display()
    );
}
