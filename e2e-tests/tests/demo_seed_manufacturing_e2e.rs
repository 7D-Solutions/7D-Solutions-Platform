//! E2E Integration + Determinism Tests for demo-seed Manufacturing Pipeline
//!
//! Tests the full demo-seed pipeline against real services:
//! 1. Full pipeline — all modules, verify resource counts
//! 2. Deterministic rerun — same seed → identical digest
//! 3. Idempotent rerun — second run creates zero new resources
//! 4. Different seeds — different digests
//! 5. Module selection — specific modules only
//! 6. Backwards compatibility — AR module still works
//! 7. Manifest output — valid JSON with expected structure
//!
//! **Prerequisite:** all platform services must be running (standard dev stack).
//! Build demo-seed first: `./scripts/cargo-slot.sh build -p demo-seed`

use serde_json::Value;
use serial_test::serial;
use std::path::PathBuf;
use std::process::Command;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Find the pre-built demo-seed binary in target-slot directories.
fn find_demo_seed_binary() -> PathBuf {
    let root = project_root();
    for slot in 1..=4 {
        let p = root.join(format!("target-slot-{}/debug/demo-seed", slot));
        if p.exists() {
            return p;
        }
    }
    let p = root.join("target/debug/demo-seed");
    if p.exists() {
        return p;
    }
    panic!("demo-seed binary not found. Run: ./scripts/cargo-slot.sh build -p demo-seed");
}

/// Unique tenant ID for test isolation.
fn test_tenant(label: &str) -> String {
    format!("e2e-mfg-{}-{}", label, &Uuid::new_v4().to_string()[..8])
}

struct SeedResult {
    stdout: String,
    stderr: String,
    success: bool,
}

impl SeedResult {
    /// Extract the digest (first line of stdout that is 64 hex chars).
    fn digest(&self) -> Option<String> {
        for line in self.stdout.lines() {
            let trimmed = line.trim();
            if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(trimmed.to_string());
            }
        }
        None
    }

    /// Parse the manifest JSON from stdout (everything after the digest line).
    fn manifest_json(&self) -> Option<Value> {
        // The manifest is the JSON object after the digest line
        let stdout = &self.stdout;
        // Find the first '{' which starts the manifest JSON
        if let Some(start) = stdout.find('{') {
            let json_str = &stdout[start..];
            serde_json::from_str(json_str).ok()
        } else {
            None
        }
    }
}

fn run_seed(args: &[&str]) -> SeedResult {
    let binary = find_demo_seed_binary();
    let output = Command::new(&binary)
        .args(args)
        .output()
        .expect("Failed to execute demo-seed binary");

    SeedResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
    }
}

// ============================================================================
// Test 1: Full pipeline — all modules, verify resource counts from manifest
// ============================================================================

#[test]
#[serial]
fn test_full_pipeline_resource_counts() {
    let tenant = test_tenant("full");
    let result = run_seed(&[
        "--tenant", &tenant,
        "--seed", "42",
        "--modules", "numbering,gl,party,inventory,bom,production",
    ]);

    if !result.success {
        eprintln!("STDERR:\n{}", result.stderr);
    }
    assert!(result.success, "demo-seed should exit successfully");

    let digest = result.digest();
    assert!(digest.is_some(), "Should output a 64-char hex digest");
    assert_eq!(digest.as_ref().unwrap().len(), 64);

    let manifest = result.manifest_json();
    assert!(manifest.is_some(), "Should output manifest JSON to stdout");
    let m = manifest.unwrap();

    // Verify resource counts from manifest
    // 20 GL accounts
    let gl_accounts = m["gl"]["accounts"].as_array().expect("gl.accounts");
    assert_eq!(gl_accounts.len(), 20, "Expected 20 GL accounts");

    // Verify critical account codes exist
    let codes: Vec<&str> = gl_accounts.iter()
        .map(|a| a["code"].as_str().unwrap())
        .collect();
    for expected in ["1200", "1210", "1220", "5000", "5100", "5120"] {
        assert!(codes.contains(&expected), "Missing GL account code: {}", expected);
    }

    // 8 numbering policies
    let policies = m["numbering"]["policies"].as_array().expect("numbering.policies");
    assert_eq!(policies.len(), 8, "Expected 8 numbering policies");

    // 10 parties (5 customers + 5 suppliers)
    let customers = m["parties"]["customers"].as_array().expect("parties.customers");
    let suppliers = m["parties"]["suppliers"].as_array().expect("parties.suppliers");
    assert_eq!(customers.len(), 5, "Expected 5 customers");
    assert_eq!(suppliers.len(), 5, "Expected 5 suppliers");

    // 5 UoMs, 7 locations, 13 items
    let uoms = m["inventory"]["uoms"].as_array().expect("inventory.uoms");
    let locations = m["inventory"]["locations"].as_array().expect("inventory.locations");
    let items = m["inventory"]["items"].as_array().expect("inventory.items");
    assert_eq!(uoms.len(), 5, "Expected 5 UoMs");
    assert_eq!(locations.len(), 7, "Expected 7 locations");
    assert_eq!(items.len(), 13, "Expected 13 items");

    // Warehouse ID is present and valid UUID
    let wh = m["inventory"]["warehouse_id"].as_str().expect("warehouse_id");
    assert!(Uuid::parse_str(wh).is_ok(), "warehouse_id must be valid UUID");

    // 5 BOMs with revisions
    let boms = m["bom"]["boms"].as_array().expect("bom.boms");
    assert_eq!(boms.len(), 5, "Expected 5 BOMs");
    for bom in boms {
        assert!(bom["id"].as_str().is_some(), "BOM must have id");
        assert!(bom["revision_id"].as_str().is_some(), "BOM must have revision_id");
        assert_eq!(bom["revision_label"].as_str().unwrap(), "A", "Revision label must be A");
    }

    // 6 work centers, 5 routings
    let workcenters = m["production"]["workcenters"].as_array().expect("production.workcenters");
    let routings = m["production"]["routings"].as_array().expect("production.routings");
    assert_eq!(workcenters.len(), 6, "Expected 6 work centers");
    assert_eq!(routings.len(), 5, "Expected 5 routings");

    // Users section present with admin email
    let admin = &m["users"]["admin"];
    assert_eq!(admin["email"].as_str().unwrap(), "admin@7dsolutions.local");
}

// ============================================================================
// Test 2: Deterministic rerun — same seed produces identical digest
// ============================================================================

#[test]
#[serial]
fn test_deterministic_rerun() {
    let tenant = test_tenant("det");

    let r1 = run_seed(&["--tenant", &tenant, "--seed", "42",
        "--modules", "numbering,gl,party,inventory,bom,production"]);
    assert!(r1.success, "First run should succeed");

    let r2 = run_seed(&["--tenant", &tenant, "--seed", "42",
        "--modules", "numbering,gl,party,inventory,bom,production"]);
    assert!(r2.success, "Second run should succeed");

    let d1 = r1.digest().expect("First run must produce digest");
    let d2 = r2.digest().expect("Second run must produce digest");
    assert_eq!(d1, d2, "Same seed must produce identical digests across runs");
}

// ============================================================================
// Test 3: Idempotent rerun — second run creates zero new resources
// ============================================================================

#[test]
#[serial]
fn test_idempotent_rerun() {
    let tenant = test_tenant("idem");

    let r1 = run_seed(&["--tenant", &tenant, "--seed", "42",
        "--modules", "numbering,gl,party,inventory,bom,production"]);
    assert!(r1.success, "First run should succeed");

    let r2 = run_seed(&["--tenant", &tenant, "--seed", "42",
        "--modules", "numbering,gl,party,inventory,bom,production"]);
    assert!(r2.success, "Second run should succeed");

    let m1 = r1.manifest_json().expect("First manifest");
    let m2 = r2.manifest_json().expect("Second manifest");

    // Compare resource counts — second run should not create additional resources
    let items1 = m1["inventory"]["items"].as_array().unwrap().len();
    let items2 = m2["inventory"]["items"].as_array().unwrap().len();
    assert_eq!(items1, items2, "Item count should not change on rerun");

    let boms1 = m1["bom"]["boms"].as_array().unwrap().len();
    let boms2 = m2["bom"]["boms"].as_array().unwrap().len();
    assert_eq!(boms1, boms2, "BOM count should not change on rerun");

    let wc1 = m1["production"]["workcenters"].as_array().unwrap().len();
    let wc2 = m2["production"]["workcenters"].as_array().unwrap().len();
    assert_eq!(wc1, wc2, "Workcenter count should not change on rerun");
}

// ============================================================================
// Test 4: Different seeds produce different digests
// ============================================================================

#[test]
#[serial]
fn test_different_seeds_different_digests() {
    let tenant = test_tenant("diff");

    let r1 = run_seed(&["--tenant", &tenant, "--seed", "42",
        "--modules", "numbering,gl,party,inventory,bom,production"]);
    assert!(r1.success, "Seed 42 should succeed");

    // Use a different tenant for seed 99 to avoid shared state
    let tenant2 = test_tenant("diff2");
    let r2 = run_seed(&["--tenant", &tenant2, "--seed", "99",
        "--modules", "numbering,gl,party,inventory,bom,production"]);
    assert!(r2.success, "Seed 99 should succeed");

    let d1 = r1.digest().expect("Seed 42 digest");
    let d2 = r2.digest().expect("Seed 99 digest");
    assert_ne!(d1, d2, "Different seeds must produce different digests");
}

// ============================================================================
// Test 5: Module selection — only specified modules run
// ============================================================================

#[test]
#[serial]
fn test_module_selection() {
    let tenant = test_tenant("modsel");

    let result = run_seed(&["--tenant", &tenant, "--seed", "42",
        "--modules", "numbering,party"]);
    assert!(result.success, "Module selection should succeed");

    let m = result.manifest_json().expect("Manifest");

    // numbering and party should be present
    assert!(m["numbering"]["policies"].as_array().is_some(), "numbering should be seeded");
    assert!(m["parties"]["customers"].as_array().is_some(), "parties should be seeded");

    // gl, inventory, bom, production should NOT be present
    assert!(m["gl"].is_null(), "gl should not be seeded");
    assert!(m["inventory"].is_null(), "inventory should not be seeded");
    assert!(m["bom"].is_null(), "bom should not be seeded");
    assert!(m["production"].is_null(), "production should not be seeded");
}

// ============================================================================
// Test 6: Backwards compatibility — AR module still works
// ============================================================================

#[test]
#[serial]
fn test_backwards_compatibility_ar() {
    let tenant = test_tenant("arcompat");

    let result = run_seed(&[
        "--tenant", &tenant,
        "--seed", "42",
        "--modules", "ar",
        "--customers", "1",
        "--invoices-per-customer", "1",
    ]);

    if !result.success {
        eprintln!("STDERR:\n{}", result.stderr);
    }
    assert!(result.success, "AR-only seed should succeed");

    let digest = result.digest();
    assert!(digest.is_some(), "AR seed should produce a digest");
}

// ============================================================================
// Test 7: Manifest output to file
// ============================================================================

#[test]
#[serial]
fn test_manifest_output_file() {
    let tenant = test_tenant("manifest");
    let manifest_path = std::env::temp_dir().join(format!("demo-seed-test-{}.json", Uuid::new_v4()));

    let result = run_seed(&[
        "--tenant", &tenant,
        "--seed", "42",
        "--modules", "numbering,gl,party,inventory,bom,production",
        "--manifest-out", manifest_path.to_str().unwrap(),
    ]);

    if !result.success {
        eprintln!("STDERR:\n{}", result.stderr);
    }
    assert!(result.success, "Manifest file output should succeed");

    // Verify file exists and is valid JSON
    assert!(manifest_path.exists(), "Manifest file should be created");
    let contents = std::fs::read_to_string(&manifest_path).expect("Read manifest file");
    let m: Value = serde_json::from_str(&contents).expect("Manifest must be valid JSON");

    // Verify expected top-level keys
    assert!(m["tenant_id"].as_str().is_some(), "Must have tenant_id");
    assert!(m["seed"].as_u64().is_some(), "Must have seed");
    assert!(m["digest"].as_str().is_some(), "Must have digest");

    // Verify non-empty arrays
    assert!(!m["numbering"]["policies"].as_array().unwrap().is_empty(), "policies non-empty");
    assert!(!m["gl"]["accounts"].as_array().unwrap().is_empty(), "accounts non-empty");
    assert!(!m["parties"]["customers"].as_array().unwrap().is_empty(), "customers non-empty");
    assert!(!m["inventory"]["items"].as_array().unwrap().is_empty(), "items non-empty");
    assert!(!m["bom"]["boms"].as_array().unwrap().is_empty(), "boms non-empty");
    assert!(!m["production"]["workcenters"].as_array().unwrap().is_empty(), "workcenters non-empty");

    // Verify UUIDs are valid
    for item in m["inventory"]["items"].as_array().unwrap() {
        let id_str = item["id"].as_str().expect("item must have id");
        assert!(Uuid::parse_str(id_str).is_ok(), "Item ID must be valid UUID: {}", id_str);
    }

    // Admin user
    assert_eq!(m["users"]["admin"]["email"].as_str().unwrap(), "admin@7dsolutions.local");

    // Cleanup
    let _ = std::fs::remove_file(&manifest_path);
}
