use pdf_editor::domain::annotations::types::Annotation;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("pdf-editor-consumer")
}

/// Every JSON file in fixtures/pdf-editor-consumer/ must deserialize into
/// the platform's production Annotation type without error. A pdf-editor
/// change that removes or renames a field our frontend sends will fail here.
#[test]
fn all_consumer_fixtures_deserialize() {
    let dir = fixtures_dir();
    let mut count = 0;

    // Fixtures excluded from this iteration (platform-side type not yet declared):
    // - WHITEOUT.json: AnnotationType::Whiteout variant not yet in Rust type.
    //   TODO(platform): add Whiteout variant to AnnotationType + renderer arm,
    //   then drop this skip and add WHITEOUT.json to fixtures/pdf-editor-consumer/.
    let skip = std::collections::HashSet::from(["WHITEOUT.json"]);

    for entry in std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("Cannot read fixtures dir {}: {}", dir.display(), e))
    {
        let entry = entry.expect("IO error reading fixture dir");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let filename = path.file_name().unwrap().to_string_lossy().into_owned();
        if skip.contains(filename.as_str()) {
            println!("⊘ {} (skipped — platform type not yet declared)", filename);
            continue;
        }

        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Cannot read {}: {}", path.display(), e));

        let _annotation: Annotation = serde_json::from_str(&contents).unwrap_or_else(|e| {
            panic!(
                "Fixture {} failed to deserialize into Annotation: {}",
                path.display(),
                e
            )
        });

        println!("✓ {}", filename);
        count += 1;
    }

    assert!(
        count > 0,
        "No fixtures found in {} — was the fixtures directory populated?",
        dir.display()
    );
}
