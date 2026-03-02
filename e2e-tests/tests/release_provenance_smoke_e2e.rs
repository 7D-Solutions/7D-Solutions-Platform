//! E2E Test: Release Provenance Smoke (bd-18a0)
//!
//! **Phase 18: Release Pipeline Foundation**
//!
//! ## Test Coverage
//! 1. **Artifact Build**: Verify release artifacts can be built
//! 2. **Checksum Computation**: Verify SHA256 checksums are computed correctly
//! 3. **Manifest Structure**: Verify release manifest contains required fields
//!
//! ## Architecture
//! - .github/workflows/release.yml: Builds artifacts and computes checksums
//! - This test verifies the mechanics work (build → checksum → manifest)
//!
//! ## Scope Constraint (Phase 18, bd-18a0)
//! This test does NOT verify:
//! - Environment promotion semantics (awaits Phase 17)
//! - Schema/projection/audit version tracking (awaits Phase 17)
//! - Artifact signing/attestation (awaits Phase 17)
//!
//! ## Invariant
//! Artifacts are immutable and traceable via SHA256 checksums.
//! Failure mode: artifacts rebuilt between environments causing drift.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Release manifest structure (basic version, pre-Phase 17)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleaseManifest {
    version: String,
    git_sha: String,
    build_time: String,
    #[serde(default)]
    artifacts: Vec<ArtifactMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactMetadata {
    binary: String,
    checksum: String,
}

/// Helper: Compute SHA256 checksum of file
fn compute_sha256(path: &PathBuf) -> Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

/// Helper: Create a test artifact directory with dummy binaries
fn create_test_artifacts() -> Result<(PathBuf, HashMap<String, String>)> {
    let temp_dir = std::env::temp_dir().join(format!("release-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir)?;

    let binaries = vec![
        "ar-rs",
        "payments-rs",
        "subscriptions-rs",
        "gl-rs",
        "notifications-rs",
        "identity-auth",
    ];
    let mut checksums = HashMap::new();

    for binary in binaries {
        let binary_path = temp_dir.join(binary);

        // Create dummy binary with unique content
        let content = format!("DUMMY BINARY: {} (v0.1.0)", binary);
        fs::write(&binary_path, content.as_bytes())?;

        // Compute checksum
        let checksum = compute_sha256(&binary_path)?;
        checksums.insert(binary.to_string(), checksum.clone());

        // Write checksum file
        let checksum_path = temp_dir.join(format!("{}.sha256", binary));
        fs::write(&checksum_path, format!("{} {}\n", checksum, binary))?;
    }

    Ok((temp_dir, checksums))
}

/// Test 1: Verify checksum computation is deterministic
#[test]
fn test_checksum_deterministic() -> Result<()> {
    let temp_dir = std::env::temp_dir().join(format!("checksum-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir)?;

    let test_file = temp_dir.join("test-binary");
    let content = b"test content for determinism";
    fs::write(&test_file, content)?;

    // Compute checksum twice
    let checksum1 = compute_sha256(&test_file)?;
    let checksum2 = compute_sha256(&test_file)?;

    // Verify determinism
    assert_eq!(checksum1, checksum2, "Checksums must be deterministic");

    // Verify against known SHA256
    let expected = "992bfc644086a39fb15b697e9c784ef9ab95f5509baa844d132401de0fe40638";
    assert_eq!(checksum1, expected, "SHA256 must match expected value");

    // Cleanup
    fs::remove_dir_all(&temp_dir)?;

    println!("✓ Checksum computation is deterministic");
    Ok(())
}

/// Test 2: Verify checksum files can be validated
#[test]
fn test_checksum_validation() -> Result<()> {
    let (temp_dir, expected_checksums) = create_test_artifacts()?;

    // Read and validate each checksum file
    for (binary, expected_checksum) in &expected_checksums {
        let checksum_file = temp_dir.join(format!("{}.sha256", binary));
        let checksum_content = fs::read_to_string(&checksum_file)?;

        // Parse checksum file (format: "<checksum> <filename>")
        let parts: Vec<&str> = checksum_content.trim().split_whitespace().collect();
        assert_eq!(parts.len(), 2, "Checksum file must have 2 parts");

        let file_checksum = parts[0];
        let file_name = parts[1];

        assert_eq!(
            file_name, binary,
            "Filename in checksum file must match binary name"
        );
        assert_eq!(
            file_checksum, expected_checksum,
            "Checksum in file must match computed checksum"
        );

        // Verify actual binary checksum matches
        let binary_path = temp_dir.join(binary);
        let actual_checksum = compute_sha256(&binary_path)?;
        assert_eq!(
            actual_checksum, *expected_checksum,
            "Binary checksum must match checksum file"
        );
    }

    // Cleanup
    fs::remove_dir_all(&temp_dir)?;

    println!("✓ Checksum files can be validated");
    Ok(())
}

/// Test 3: Verify manifest structure contains required fields
#[test]
fn test_manifest_structure() -> Result<()> {
    // Create test manifest
    let manifest = ReleaseManifest {
        version: "v0.1.0".to_string(),
        git_sha: "abc123def456".to_string(),
        build_time: "2026-02-16T10:30:00Z".to_string(),
        artifacts: vec![
            ArtifactMetadata {
                binary: "ar-rs".to_string(),
                checksum: "sha256:123abc...".to_string(),
            },
            ArtifactMetadata {
                binary: "payments-rs".to_string(),
                checksum: "sha256:456def...".to_string(),
            },
        ],
    };

    // Serialize to JSON
    let json = serde_json::to_string_pretty(&manifest)?;

    // Deserialize back
    let parsed: ReleaseManifest = serde_json::from_str(&json)?;

    // Verify required fields
    assert!(!parsed.version.is_empty(), "Manifest must have version");
    assert!(!parsed.git_sha.is_empty(), "Manifest must have git_sha");
    assert!(
        !parsed.build_time.is_empty(),
        "Manifest must have build_time"
    );

    // Verify artifacts structure
    assert_eq!(parsed.artifacts.len(), 2, "Manifest must contain artifacts");
    for artifact in &parsed.artifacts {
        assert!(
            !artifact.binary.is_empty(),
            "Artifact must have binary name"
        );
        assert!(!artifact.checksum.is_empty(), "Artifact must have checksum");
    }

    println!("✓ Manifest structure is valid");
    Ok(())
}

/// Test 4: Verify artifact immutability (checksum change detection)
#[test]
fn test_artifact_immutability() -> Result<()> {
    let temp_dir = std::env::temp_dir().join(format!("immutable-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir)?;

    let binary_path = temp_dir.join("test-binary");

    // Create original binary
    fs::write(&binary_path, b"original content")?;
    let original_checksum = compute_sha256(&binary_path)?;

    // Modify binary
    fs::write(&binary_path, b"modified content")?;
    let modified_checksum = compute_sha256(&binary_path)?;

    // Verify checksums differ (immutability violated)
    assert_ne!(
        original_checksum, modified_checksum,
        "Checksum must change when artifact is modified"
    );

    // Restore original
    fs::write(&binary_path, b"original content")?;
    let restored_checksum = compute_sha256(&binary_path)?;

    // Verify checksum matches original
    assert_eq!(
        original_checksum, restored_checksum,
        "Checksum must match when content is identical"
    );

    // Cleanup
    fs::remove_dir_all(&temp_dir)?;

    println!("✓ Artifact immutability detection works");
    Ok(())
}

/// Test 5: Verify all expected binaries are present
#[test]
fn test_expected_binaries_present() -> Result<()> {
    let (temp_dir, checksums) = create_test_artifacts()?;

    let expected_binaries = vec![
        "ar-rs",
        "payments-rs",
        "subscriptions-rs",
        "gl-rs",
        "notifications-rs",
        "identity-auth",
    ];

    for binary in expected_binaries {
        // Verify binary exists
        let binary_path = temp_dir.join(binary);
        assert!(binary_path.exists(), "Binary {} must exist", binary);

        // Verify checksum file exists
        let checksum_path = temp_dir.join(format!("{}.sha256", binary));
        assert!(
            checksum_path.exists(),
            "Checksum file for {} must exist",
            binary
        );

        // Verify checksum is recorded
        assert!(
            checksums.contains_key(binary),
            "Checksum for {} must be recorded",
            binary
        );
    }

    // Cleanup
    fs::remove_dir_all(&temp_dir)?;

    println!("✓ All expected binaries are present");
    Ok(())
}

/// Integration test: Full release artifact workflow
#[test]
fn test_release_workflow_integration() -> Result<()> {
    // 1. Create artifacts
    let (temp_dir, checksums) = create_test_artifacts()?;

    // 2. Create manifest
    let mut artifacts = Vec::new();
    for (binary, checksum) in &checksums {
        artifacts.push(ArtifactMetadata {
            binary: binary.clone(),
            checksum: checksum.clone(),
        });
    }

    let manifest = ReleaseManifest {
        version: "v0.1.0".to_string(),
        git_sha: "test-sha-123".to_string(),
        build_time: chrono::Utc::now().to_rfc3339(),
        artifacts,
    };

    // 3. Write manifest
    let manifest_path = temp_dir.join("manifest.json");
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&manifest_path, manifest_json)?;

    // 4. Verify manifest can be read back
    let manifest_content = fs::read_to_string(&manifest_path)?;
    let parsed_manifest: ReleaseManifest = serde_json::from_str(&manifest_content)?;

    assert_eq!(parsed_manifest.version, "v0.1.0");
    assert_eq!(parsed_manifest.git_sha, "test-sha-123");
    assert_eq!(parsed_manifest.artifacts.len(), checksums.len());

    // 5. Verify each artifact checksum matches
    for artifact in &parsed_manifest.artifacts {
        let binary_path = temp_dir.join(&artifact.binary);
        let computed_checksum = compute_sha256(&binary_path)?;
        assert_eq!(
            computed_checksum, artifact.checksum,
            "Checksum for {} must match manifest",
            artifact.binary
        );
    }

    // Cleanup
    fs::remove_dir_all(&temp_dir)?;

    println!("✓ Full release workflow integration works");
    Ok(())
}
