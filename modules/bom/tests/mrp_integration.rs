use bom_rs::domain::bom_service;
use bom_rs::domain::models::*;
use bom_rs::domain::mrp_engine;
use chrono::{Duration, Utc};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://bom_user:bom_pass@localhost:5450/bom_db".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to BOM test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run BOM migrations");

    pool
}

fn unique_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

/// Creates a BOM header + revision + lines (with scrap) + makes it effective.
async fn create_effective_bom_with_scrap(
    pool: &sqlx::PgPool,
    tenant: &str,
    corr: &str,
    part_id: Uuid,
    label: &str,
    components: &[(Uuid, f64, f64)], // (item_id, quantity, scrap_factor)
    effective_from: chrono::DateTime<Utc>,
    effective_to: Option<chrono::DateTime<Utc>>,
) -> (BomHeader, BomRevision) {
    let bom = bom_service::create_bom(
        pool,
        tenant,
        &CreateBomRequest {
            part_id,
            description: Some(format!("BOM {}", label)),
        },
        corr,
        None,
    )
    .await
    .unwrap();

    let rev = bom_service::create_revision(
        pool,
        tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: label.to_string(),
        },
        corr,
        None,
    )
    .await
    .unwrap();

    for (comp_id, qty, scrap) in components {
        bom_service::add_line(
            pool,
            tenant,
            rev.id,
            &AddLineRequest {
                component_item_id: *comp_id,
                quantity: *qty,
                uom: Some("EA".to_string()),
                scrap_factor: Some(*scrap),
                find_number: None,
            },
            corr,
            None,
        )
        .await
        .unwrap();
    }

    bom_service::set_effectivity(
        pool,
        tenant,
        rev.id,
        &SetEffectivityRequest {
            effective_from,
            effective_to,
        },
        corr,
        None,
    )
    .await
    .unwrap();

    (bom, rev)
}

// ============================================================================
// Test 1: Single-level BOM, no on-hand → scrap_adjusted=55, net=55
// demand=5, qty_per_unit=10, scrap=0.1
// gross = 5*10 = 50
// scrap_adjusted = 50 * (1 + 0.1) = 55
// net = max(0, 55 - 0) = 55
// ============================================================================

#[tokio::test]
#[serial]
async fn mrp_single_component_no_on_hand() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();

    let part_a = Uuid::new_v4();
    let part_b = Uuid::new_v4();

    let (bom_a, rev_a) = create_effective_bom_with_scrap(
        &pool,
        &tenant,
        &corr,
        part_a,
        "Rev-A",
        &[(part_b, 10.0, 0.1)],
        now - Duration::hours(1),
        None,
    )
    .await;

    let result = mrp_engine::explode(
        &pool,
        &tenant,
        &MrpExplodeRequest {
            bom_id: bom_a.id,
            demand_quantity: 5.0,
            effectivity_date: now,
            on_hand: vec![],
            created_by: "test".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("mrp explode");

    assert_eq!(result.lines.len(), 1);
    let line = &result.lines[0];
    assert_eq!(line.component_item_id, part_b);
    assert_eq!(line.level, 1);
    assert_eq!(line.revision_id, rev_a.id);
    assert!(
        (line.gross_quantity - 50.0).abs() < 1e-9,
        "gross={}",
        line.gross_quantity
    );
    assert!(
        (line.scrap_adjusted_quantity - 55.0).abs() < 1e-9,
        "scrap_adj={}",
        line.scrap_adjusted_quantity
    );
    assert!((line.on_hand_quantity - 0.0).abs() < 1e-9);
    assert!(
        (line.net_quantity - 55.0).abs() < 1e-9,
        "net={}",
        line.net_quantity
    );
}

// ============================================================================
// Test 2: Same BOM, on_hand=60 → net=0 (clamped, not negative)
// ============================================================================

#[tokio::test]
#[serial]
async fn mrp_on_hand_exceeds_requirement_clamps_to_zero() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();

    let part_a = Uuid::new_v4();
    let part_b = Uuid::new_v4();

    let (bom_a, _) = create_effective_bom_with_scrap(
        &pool,
        &tenant,
        &corr,
        part_a,
        "Rev-A",
        &[(part_b, 10.0, 0.1)],
        now - Duration::hours(1),
        None,
    )
    .await;

    let result = mrp_engine::explode(
        &pool,
        &tenant,
        &MrpExplodeRequest {
            bom_id: bom_a.id,
            demand_quantity: 5.0,
            effectivity_date: now,
            on_hand: vec![OnHandEntry {
                item_id: part_b,
                quantity: 60.0,
            }],
            created_by: "test".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("mrp explode");

    assert_eq!(result.lines.len(), 1);
    let line = &result.lines[0];
    assert!((line.scrap_adjusted_quantity - 55.0).abs() < 1e-9);
    assert!((line.on_hand_quantity - 60.0).abs() < 1e-9);
    // net must clamp to 0, never go negative
    assert!(
        (line.net_quantity - 0.0).abs() < 1e-9,
        "net should be 0, got {}",
        line.net_quantity
    );
}

// ============================================================================
// Test 3: Multi-level BOM (A→B→C) → lines emitted for every level
// ============================================================================

#[tokio::test]
#[serial]
async fn mrp_multi_level_bom_all_levels_present() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();

    let part_a = Uuid::new_v4();
    let part_b = Uuid::new_v4();
    let part_c = Uuid::new_v4();

    let (bom_a, _) = create_effective_bom_with_scrap(
        &pool,
        &tenant,
        &corr,
        part_a,
        "A-1",
        &[(part_b, 2.0, 0.0)],
        now - Duration::hours(1),
        None,
    )
    .await;

    create_effective_bom_with_scrap(
        &pool,
        &tenant,
        &corr,
        part_b,
        "B-1",
        &[(part_c, 3.0, 0.0)],
        now - Duration::hours(1),
        None,
    )
    .await;

    let result = mrp_engine::explode(
        &pool,
        &tenant,
        &MrpExplodeRequest {
            bom_id: bom_a.id,
            demand_quantity: 10.0,
            effectivity_date: now,
            on_hand: vec![],
            created_by: "test".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("mrp multi-level explode");

    // Must have lines for every level
    assert_eq!(
        result.lines.len(),
        2,
        "expected 2 lines (B at level 1, C at level 2)"
    );

    let level1: Vec<_> = result.lines.iter().filter(|l| l.level == 1).collect();
    let level2: Vec<_> = result.lines.iter().filter(|l| l.level == 2).collect();
    assert_eq!(level1.len(), 1);
    assert_eq!(level2.len(), 1);
    assert_eq!(level1[0].component_item_id, part_b);
    assert_eq!(level2[0].component_item_id, part_c);

    // Level 1: demand=10, qty=2 → gross=20, scrap_adj=20
    assert!((level1[0].gross_quantity - 20.0).abs() < 1e-9);
    // Level 2: parent_demand=20 (scrap_adj of B), qty=3 → gross=60
    assert!((level2[0].gross_quantity - 60.0).abs() < 1e-9);
}

// ============================================================================
// Test 4: Two revisions; effectivity_date between them → only effective rev
// ============================================================================

#[tokio::test]
#[serial]
async fn mrp_effectivity_date_selects_correct_revision() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();
    let cutover = now - Duration::hours(5);

    let part_a = Uuid::new_v4();
    let old_comp = Uuid::new_v4();
    let new_comp = Uuid::new_v4();

    // Revision 1: effective before cutover (old component)
    let bom = bom_service::create_bom(
        &pool,
        &tenant,
        &CreateBomRequest {
            part_id: part_a,
            description: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let rev1 = bom_service::create_revision(
        &pool,
        &tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "Rev-1".to_string(),
        },
        &corr,
        None,
    )
    .await
    .unwrap();
    bom_service::add_line(
        &pool,
        &tenant,
        rev1.id,
        &AddLineRequest {
            component_item_id: old_comp,
            quantity: 1.0,
            uom: None,
            scrap_factor: None,
            find_number: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();
    bom_service::set_effectivity(
        &pool,
        &tenant,
        rev1.id,
        &SetEffectivityRequest {
            effective_from: now - Duration::hours(24),
            effective_to: Some(cutover),
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Revision 2: effective after cutover (new component)
    let rev2 = bom_service::create_revision(
        &pool,
        &tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "Rev-2".to_string(),
        },
        &corr,
        None,
    )
    .await
    .unwrap();
    bom_service::add_line(
        &pool,
        &tenant,
        rev2.id,
        &AddLineRequest {
            component_item_id: new_comp,
            quantity: 1.0,
            uom: None,
            scrap_factor: None,
            find_number: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();
    bom_service::set_effectivity(
        &pool,
        &tenant,
        rev2.id,
        &SetEffectivityRequest {
            effective_from: cutover,
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Explode at a date BEFORE cutover → should see old_comp only
    let before = mrp_engine::explode(
        &pool,
        &tenant,
        &MrpExplodeRequest {
            bom_id: bom.id,
            demand_quantity: 1.0,
            effectivity_date: cutover - Duration::hours(1),
            on_hand: vec![],
            created_by: "test".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("explode before cutover");

    assert_eq!(before.lines.len(), 1);
    assert_eq!(
        before.lines[0].component_item_id, old_comp,
        "expected old component before cutover"
    );

    // Explode AT/AFTER cutover → should see new_comp only
    let after = mrp_engine::explode(
        &pool,
        &tenant,
        &MrpExplodeRequest {
            bom_id: bom.id,
            demand_quantity: 1.0,
            effectivity_date: cutover + Duration::hours(1),
            on_hand: vec![],
            created_by: "test".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("explode after cutover");

    assert_eq!(after.lines.len(), 1);
    assert_eq!(
        after.lines[0].component_item_id, new_comp,
        "expected new component after cutover"
    );
}

// ============================================================================
// Test 5: Snapshot persisted with full on_hand_snapshot JSONB for audit
// ============================================================================

#[tokio::test]
#[serial]
async fn mrp_snapshot_persisted_with_on_hand_jsonb() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();

    let part_a = Uuid::new_v4();
    let part_b = Uuid::new_v4();

    let (bom_a, _) = create_effective_bom_with_scrap(
        &pool,
        &tenant,
        &corr,
        part_a,
        "Rev-A",
        &[(part_b, 1.0, 0.0)],
        now - Duration::hours(1),
        None,
    )
    .await;

    let on_hand_input = vec![OnHandEntry {
        item_id: part_b,
        quantity: 5.0,
    }];

    let result = mrp_engine::explode(
        &pool,
        &tenant,
        &MrpExplodeRequest {
            bom_id: bom_a.id,
            demand_quantity: 10.0,
            effectivity_date: now,
            on_hand: on_hand_input.clone(),
            created_by: "auditor".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("mrp explode");

    // Fetch snapshot back and verify JSONB
    let fetched = mrp_engine::get_snapshot(&pool, &tenant, result.snapshot.id)
        .await
        .expect("get_snapshot");

    let snapshot_json = &fetched.snapshot.on_hand_snapshot;
    let arr = snapshot_json
        .as_array()
        .expect("on_hand_snapshot must be a JSON array");
    assert_eq!(arr.len(), 1);
    let entry = &arr[0];
    assert_eq!(entry["item_id"].as_str().unwrap(), part_b.to_string());
    assert!((entry["quantity"].as_f64().unwrap() - 5.0).abs() < 1e-9);
}

// ============================================================================
// Test 6: Event bom.mrp_exploded written to outbox with correct counts
// ============================================================================

#[tokio::test]
#[serial]
async fn mrp_event_written_to_outbox_with_correct_counts() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();

    let part_a = Uuid::new_v4();
    let part_b = Uuid::new_v4();
    let part_c = Uuid::new_v4();

    let (bom_a, _) = create_effective_bom_with_scrap(
        &pool,
        &tenant,
        &corr,
        part_a,
        "Rev-A",
        &[(part_b, 5.0, 0.0), (part_c, 2.0, 0.0)],
        now - Duration::hours(1),
        None,
    )
    .await;

    // part_b has on_hand=100 (no shortage), part_c has none (shortage)
    let result = mrp_engine::explode(
        &pool,
        &tenant,
        &MrpExplodeRequest {
            bom_id: bom_a.id,
            demand_quantity: 10.0,
            effectivity_date: now,
            on_hand: vec![OnHandEntry {
                item_id: part_b,
                quantity: 100.0,
            }],
            created_by: "test".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("mrp explode");

    // Read outbox entry for this snapshot
    let row: (String, serde_json::Value) = sqlx::query_as(
        "SELECT event_type, payload FROM bom_outbox WHERE aggregate_id = $1 AND tenant_id = $2",
    )
    .bind(result.snapshot.id.to_string())
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox row");

    assert_eq!(row.0, "bom.mrp_exploded");

    let payload = &row.1;
    // 2 lines total (part_b + part_c at level 1)
    assert_eq!(
        payload["payload"]["line_count"].as_i64().unwrap(),
        2,
        "line_count mismatch"
    );
    // only part_c has a shortage (part_b on_hand=100 covers gross=50)
    assert_eq!(
        payload["payload"]["net_shortage_count"].as_i64().unwrap(),
        1,
        "net_shortage_count mismatch"
    );
    assert_eq!(
        payload["payload"]["snapshot_id"].as_str().unwrap(),
        result.snapshot.id.to_string()
    );
}

// ============================================================================
// Test 7: Tenant isolation — tenant A snapshots not visible to tenant B
// ============================================================================

#[tokio::test]
#[serial]
async fn mrp_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();

    let part_a = Uuid::new_v4();
    let part_b = Uuid::new_v4();

    // Create BOM under tenant_a
    let (bom_a, _) = create_effective_bom_with_scrap(
        &pool,
        &tenant_a,
        &corr,
        part_a,
        "A-1",
        &[(part_b, 1.0, 0.0)],
        now - Duration::hours(1),
        None,
    )
    .await;

    let result = mrp_engine::explode(
        &pool,
        &tenant_a,
        &MrpExplodeRequest {
            bom_id: bom_a.id,
            demand_quantity: 1.0,
            effectivity_date: now,
            on_hand: vec![],
            created_by: "test".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("explode as tenant_a");

    let snapshot_id = result.snapshot.id;

    // Tenant B cannot see tenant A's snapshot
    let not_found = mrp_engine::get_snapshot(&pool, &tenant_b, snapshot_id).await;
    assert!(
        not_found.is_err(),
        "tenant B should not see tenant A snapshot"
    );

    // Tenant B list returns empty
    let b_list = mrp_engine::list_snapshots(
        &pool,
        &tenant_b,
        &MrpSnapshotListQuery {
            bom_id: None,
            page: 1,
            page_size: 50,
        },
    )
    .await
    .expect("list as tenant_b");
    assert!(b_list.is_empty(), "tenant B list should be empty");
}

// ============================================================================
// Test 8: Deterministic — same inputs produce identical output
// ============================================================================

#[tokio::test]
#[serial]
async fn mrp_deterministic_same_inputs_identical_output() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();

    let part_a = Uuid::new_v4();
    let part_b = Uuid::new_v4();

    let (bom_a, _) = create_effective_bom_with_scrap(
        &pool,
        &tenant,
        &corr,
        part_a,
        "Rev-A",
        &[(part_b, 4.0, 0.05)],
        now - Duration::hours(1),
        None,
    )
    .await;

    let req = MrpExplodeRequest {
        bom_id: bom_a.id,
        demand_quantity: 7.0,
        effectivity_date: now,
        on_hand: vec![OnHandEntry {
            item_id: part_b,
            quantity: 3.0,
        }],
        created_by: "test".to_string(),
    };

    let run1 = mrp_engine::explode(&pool, &tenant, &req, &corr, None)
        .await
        .expect("run 1");
    let run2 = mrp_engine::explode(&pool, &tenant, &req, &corr, None)
        .await
        .expect("run 2");

    assert_eq!(run1.lines.len(), run2.lines.len());
    for (l1, l2) in run1.lines.iter().zip(run2.lines.iter()) {
        assert!((l1.gross_quantity - l2.gross_quantity).abs() < 1e-9);
        assert!((l1.scrap_adjusted_quantity - l2.scrap_adjusted_quantity).abs() < 1e-9);
        assert!((l1.net_quantity - l2.net_quantity).abs() < 1e-9);
        assert_eq!(l1.component_item_id, l2.component_item_id);
    }
}
