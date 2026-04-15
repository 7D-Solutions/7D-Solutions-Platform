// E2E: BOM Lifecycle — create, revise, multi-level explosion, cost rollup
//
// Tests the full BOM lifecycle via real HTTP calls against the live BOM service:
//   1. Create BOM header (assembly + sub-assembly)
//   2. Create revisions and set effectivity
//   3. Add BOM lines (multi-level: assembly → sub-assembly → raw material)
//   4. Explode BOM — verifies 2-level recursive expansion (basis for cost rollup)
//   5. BOM revision management — create second revision, verify list
//   6. Where-used lookup
//
// All tests use unique tenant_id per run; no mocks; hits real Docker containers.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const BOM_DEFAULT_URL: &str = "http://localhost:8107";

fn bom_url() -> String {
    std::env::var("BOM_URL").unwrap_or_else(|_| BOM_DEFAULT_URL.to_string())
}

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    app_id: Option<String>,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn dev_private_key() -> Option<EncodingKey> {
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM").ok()?;
    EncodingKey::from_rsa_pem(pem.replace("\\n", "\n").as_bytes()).ok()
}

fn make_jwt(key: &EncodingKey, tenant_id: &str, perms: &[&str]) -> String {
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        app_id: Some(tenant_id.to_string()),
        roles: vec!["operator".to_string()],
        perms: perms.iter().map(|s| s.to_string()).collect(),
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key).unwrap()
}

async fn wait_for_service(client: &Client) -> bool {
    let url = format!("{}/api/health", bom_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  BOM health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  BOM health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// Create a BOM header and return its id.
async fn create_bom(
    client: &Client,
    base: &str,
    jwt: &str,
    part_id: Uuid,
    description: &str,
) -> Uuid {
    let resp = client
        .post(format!("{base}/api/bom"))
        .bearer_auth(jwt)
        .json(&json!({"part_id": part_id, "description": description}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED,
        "Create BOM failed: {status} - {body}"
    );
    let id = body["id"].as_str().expect("no id in BOM response");
    Uuid::parse_str(id).unwrap()
}

/// Create a revision and return its id.
async fn create_revision(
    client: &Client,
    base: &str,
    jwt: &str,
    bom_id: Uuid,
    label: &str,
) -> Uuid {
    let resp = client
        .post(format!("{base}/api/bom/{bom_id}/revisions"))
        .bearer_auth(jwt)
        .json(&json!({"revision_label": label}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED,
        "Create revision failed: {status} - {body}"
    );
    let id = body["id"].as_str().expect("no id in revision response");
    Uuid::parse_str(id).unwrap()
}

/// Set effectivity on a revision (makes it 'effective').
async fn set_effectivity(
    client: &Client,
    base: &str,
    jwt: &str,
    revision_id: Uuid,
    effective_from: &str,
) {
    let resp = client
        .post(format!(
            "{base}/api/bom/revisions/{revision_id}/effectivity"
        ))
        .bearer_auth(jwt)
        .json(&json!({"effective_from": effective_from}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "Set effectivity failed: {status} - {body}"
    );
    assert_eq!(
        body["status"], "effective",
        "revision must be effective after setting effectivity"
    );
}

/// Add a line to a revision and return its id.
async fn add_line(
    client: &Client,
    base: &str,
    jwt: &str,
    revision_id: Uuid,
    component_item_id: Uuid,
    quantity: f64,
    uom: &str,
) -> Uuid {
    let resp = client
        .post(format!("{base}/api/bom/revisions/{revision_id}/lines"))
        .bearer_auth(jwt)
        .json(&json!({
            "component_item_id": component_item_id,
            "quantity": quantity,
            "uom": uom,
            "scrap_factor": 0.02
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED,
        "Add line failed: {status} - {body}"
    );
    let id = body["id"].as_str().expect("no id in line response");
    Uuid::parse_str(id).unwrap()
}

#[tokio::test]
async fn bom_lifecycle_e2e() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        panic!(
            "BOM service not reachable at {} — start the dev stack before running E2E tests",
            bom_url()
        );
    }
    println!("BOM service healthy at {}", bom_url());

    let key = dev_private_key().expect("JWT_PRIVATE_KEY_PEM must be set to run BOM E2E tests");

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["bom.mutate", "bom.read"]);
    let base = bom_url();

    // Probe JWT acceptance
    let probe = client
        .get(format!("{base}/api/health"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    assert_ne!(
        probe.status().as_u16(),
        401,
        "BOM returned 401 with valid JWT — set JWT_PUBLIC_KEY before running E2E tests"
    );

    // Part IDs: 3 distinct UUIDs representing inventory items
    let assembly_part_id = Uuid::new_v4(); // top-level assembly
    let sub_assembly_part_id = Uuid::new_v4(); // mid-level sub-assembly
    let raw_material_part_id = Uuid::new_v4(); // raw material (leaf)

    // Effectivity date: 1 year ago, so all revisions are in range
    let effective_from = (Utc::now() - chrono::Duration::days(365)).to_rfc3339();

    // =========================================================================
    // 1. Create assembly BOM
    // =========================================================================
    println!("\n--- 1. Create assembly BOM ---");
    let assembly_bom_id =
        create_bom(&client, &base, &jwt, assembly_part_id, "Top-Level Assembly").await;
    println!("  assembly bom_id={assembly_bom_id}");

    // GET /api/bom/{bom_id}
    let resp = client
        .get(format!("{base}/api/bom/{assembly_bom_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "GET BOM failed: {}",
        resp.status()
    );
    let fetched: Value = resp.json().await.unwrap();
    assert_eq!(fetched["id"].as_str().unwrap(), assembly_bom_id.to_string());
    println!("  GET BOM: ok, part_id={}", fetched["part_id"]);

    // =========================================================================
    // 2. Create assembly revision Rev-A and set effectivity
    // =========================================================================
    println!("\n--- 2. Create revision Rev-A for assembly ---");
    let assembly_rev_a_id = create_revision(&client, &base, &jwt, assembly_bom_id, "Rev-A").await;
    println!("  revision_id={assembly_rev_a_id} status=draft");

    // =========================================================================
    // 3. Add sub-assembly as a line on assembly Rev-A
    // =========================================================================
    println!("\n--- 3. Add sub-assembly line to assembly Rev-A ---");
    let line1_id = add_line(
        &client,
        &base,
        &jwt,
        assembly_rev_a_id,
        sub_assembly_part_id,
        2.0,
        "EA",
    )
    .await;
    println!("  line added: sub_assembly_part_id qty=2 uom=EA");

    // GET lines
    let resp = client
        .get(format!(
            "{base}/api/bom/revisions/{assembly_rev_a_id}/lines"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "GET lines failed: {}",
        resp.status()
    );
    let lines: Value = resp.json().await.unwrap();
    assert!(
        lines.as_array().map_or(0, |a| a.len()) >= 1,
        "Expected at least 1 line on assembly Rev-A"
    );
    println!(
        "  listed {} line(s) on Rev-A",
        lines.as_array().map_or(0, |a| a.len())
    );

    // Set effectivity on assembly Rev-A
    println!("  Setting effectivity on Rev-A...");
    set_effectivity(&client, &base, &jwt, assembly_rev_a_id, &effective_from).await;
    println!("  Rev-A is now effective");

    // =========================================================================
    // 4. Create sub-assembly BOM
    // =========================================================================
    println!("\n--- 4. Create sub-assembly BOM ---");
    let sub_assembly_bom_id = create_bom(
        &client,
        &base,
        &jwt,
        sub_assembly_part_id,
        "Sub-Assembly Widget",
    )
    .await;
    println!("  sub_assembly bom_id={sub_assembly_bom_id}");

    // Create revision for sub-assembly
    let sub_assembly_rev_id =
        create_revision(&client, &base, &jwt, sub_assembly_bom_id, "Rev-A").await;

    // Add raw material as a line on sub-assembly
    let _line2_id = add_line(
        &client,
        &base,
        &jwt,
        sub_assembly_rev_id,
        raw_material_part_id,
        5.0,
        "KG",
    )
    .await;
    println!("  line added: raw_material_part_id qty=5 uom=KG");

    // Set effectivity
    set_effectivity(&client, &base, &jwt, sub_assembly_rev_id, &effective_from).await;
    println!("  sub-assembly Rev-A is effective");

    // =========================================================================
    // 5. Explode assembly BOM — verifies 2-level recursive expansion
    //    Level 1: assembly → sub_assembly (qty 2)
    //    Level 2: sub_assembly → raw_material (qty 5)
    //    This is the basis for cost rollup across BOM levels.
    // =========================================================================
    println!("\n--- 5. Explode assembly BOM (2-level) ---");
    let resp = client
        .get(format!("{base}/api/bom/{assembly_bom_id}/explosion"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "GET explosion failed: {}",
        resp.status()
    );
    let explosion: Value = resp.json().await.unwrap();
    let rows = explosion.as_array().expect("explosion must be an array");
    assert!(
        rows.len() >= 2,
        "Expected at least 2 explosion rows for 2-level BOM, got {}. \
         Rows: {explosion}",
        rows.len()
    );

    // Verify level 1 and level 2 are present
    let level1 = rows
        .iter()
        .find(|r| r["level"] == 1)
        .expect("missing level 1 in explosion");
    let level2 = rows
        .iter()
        .find(|r| r["level"] == 2)
        .expect("missing level 2 in explosion");
    assert_eq!(
        level1["component_item_id"].as_str().unwrap(),
        sub_assembly_part_id.to_string(),
        "level 1 component must be sub_assembly"
    );
    assert_eq!(
        level2["component_item_id"].as_str().unwrap(),
        raw_material_part_id.to_string(),
        "level 2 component must be raw_material"
    );
    assert_eq!(level1["quantity"], 2.0, "level 1 qty must be 2");
    assert_eq!(level2["quantity"], 5.0, "level 2 qty must be 5");
    println!(
        "  explosion: {} row(s), level1=sub_assembly qty={}, level2=raw_material qty={}",
        rows.len(),
        level1["quantity"],
        level2["quantity"]
    );

    // =========================================================================
    // 6. BOM revision management — create Rev-B, verify list shows both
    // =========================================================================
    println!("\n--- 6. BOM revision management (create Rev-B) ---");
    let assembly_rev_b_id = create_revision(&client, &base, &jwt, assembly_bom_id, "Rev-B").await;
    println!("  created Rev-B id={assembly_rev_b_id}");

    // List revisions
    let resp = client
        .get(format!("{base}/api/bom/{assembly_bom_id}/revisions"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List revisions failed: {}",
        resp.status()
    );
    let revisions: Value = resp.json().await.unwrap();
    let revs = revisions.as_array().expect("revisions must be an array");
    assert!(
        revs.len() >= 2,
        "Expected at least 2 revisions (Rev-A + Rev-B), got {}",
        revs.len()
    );

    let has_rev_a = revs.iter().any(|r| r["revision_label"] == "Rev-A");
    let has_rev_b = revs.iter().any(|r| r["revision_label"] == "Rev-B");
    assert!(has_rev_a, "Rev-A must appear in revisions list");
    assert!(has_rev_b, "Rev-B must appear in revisions list");

    // Rev-A should be effective, Rev-B should be draft
    let rev_a = revs
        .iter()
        .find(|r| r["revision_label"] == "Rev-A")
        .unwrap();
    let rev_b = revs
        .iter()
        .find(|r| r["revision_label"] == "Rev-B")
        .unwrap();
    assert_eq!(rev_a["status"], "effective", "Rev-A must be effective");
    assert_eq!(rev_b["status"], "draft", "Rev-B must be draft");
    println!(
        "  revisions: {} total, Rev-A={} Rev-B={}",
        revs.len(),
        rev_a["status"],
        rev_b["status"]
    );

    // =========================================================================
    // 7. Update a BOM line on draft Rev-B (PUT /api/bom/lines/{line_id})
    //    Lines on effective revisions are immutable; add a line to Rev-B first.
    // =========================================================================
    println!("\n--- 7. Update BOM line quantity (on draft Rev-B) ---");
    let rev_b_line_id = add_line(
        &client,
        &base,
        &jwt,
        assembly_rev_b_id,
        sub_assembly_part_id,
        2.0,
        "EA",
    )
    .await;
    let resp = client
        .put(format!("{base}/api/bom/lines/{rev_b_line_id}"))
        .bearer_auth(&jwt)
        .json(&json!({"quantity": 3.0, "uom": "EA"}))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "PUT line failed: {}",
        resp.status()
    );
    let updated_line: Value = resp.json().await.unwrap();
    assert_eq!(updated_line["quantity"], 3.0, "updated quantity must be 3");
    println!("  line updated: quantity={}", updated_line["quantity"]);

    // =========================================================================
    // 8. Where-used lookup for sub-assembly part
    // =========================================================================
    println!("\n--- 8. Where-used: sub_assembly_part ---");
    let resp = client
        .get(format!("{base}/api/bom/where-used/{sub_assembly_part_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Where-used failed: {}",
        resp.status()
    );
    let where_used: Value = resp.json().await.unwrap();
    let wu_rows = where_used.as_array().expect("where-used must be an array");
    assert!(
        wu_rows.len() >= 1,
        "sub_assembly must appear in at least 1 where-used result"
    );
    let found = wu_rows
        .iter()
        .find(|r| r["part_id"].as_str().unwrap_or("") == assembly_part_id.to_string());
    assert!(
        found.is_some(),
        "assembly must be listed as a parent of sub_assembly in where-used"
    );
    println!(
        "  where-used: {} result(s), found assembly as parent",
        wu_rows.len()
    );

    println!("\n=== BOM lifecycle E2E passed ===");
    println!("  - BOM create/read: ok");
    println!("  - Revision create + effectivity: ok");
    println!("  - Multi-level BOM lines: ok");
    println!("  - 2-level explosion (cost rollup basis): ok");
    println!("  - Revision management (Rev-A effective, Rev-B draft): ok");
    println!("  - Line update: ok");
    println!("  - Where-used lookup: ok");
}
