use bom_rs::domain::bom_service;
use bom_rs::domain::models::*;
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

async fn create_effective_bom(
    pool: &sqlx::PgPool,
    tenant: &str,
    corr: &str,
    part_id: Uuid,
    label: &str,
    components: &[(Uuid, f64)],
    effective_from: chrono::DateTime<Utc>,
) -> (BomHeader, BomRevision) {
    let bom = bom_service::create_bom(
        pool,
        tenant,
        &CreateBomRequest {
            part_id,
            description: Some(format!("BOM for {}", label)),
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

    for (comp_id, qty) in components {
        bom_service::add_line(
            pool,
            tenant,
            rev.id,
            &AddLineRequest {
                component_item_id: *comp_id,
                quantity: *qty,
                uom: None,
                scrap_factor: None,
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
            effective_to: None,
        },
        corr,
        None,
    )
    .await
    .unwrap();

    (bom, rev)
}

// ---- Explosion with depth guard ----

#[tokio::test]
#[serial]
async fn explosion_multi_level_with_depth_guard() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();

    let part_a = Uuid::new_v4();
    let part_b = Uuid::new_v4();
    let part_c = Uuid::new_v4();

    let (bom_a, _) = create_effective_bom(
        &pool,
        &tenant,
        &corr,
        part_a,
        "A-1",
        &[(part_b, 2.0)],
        now - Duration::hours(1),
    )
    .await;

    create_effective_bom(
        &pool,
        &tenant,
        &corr,
        part_b,
        "B-1",
        &[(part_c, 3.0)],
        now - Duration::hours(1),
    )
    .await;

    // Explode A — expect 2 rows: B (level 1), C (level 2)
    let explosion = bom_service::explode(
        &pool,
        &tenant,
        bom_a.id,
        &ExplosionQuery {
            date: Some(now),
            max_depth: Some(20),
        },
    )
    .await
    .expect("explosion");

    assert_eq!(explosion.len(), 2);
    assert_eq!(explosion[0].level, 1);
    assert_eq!(explosion[0].component_item_id, part_b);
    assert_eq!(explosion[1].level, 2);
    assert_eq!(explosion[1].component_item_id, part_c);

    // Explode with depth=1 — only see B
    let shallow = bom_service::explode(
        &pool,
        &tenant,
        bom_a.id,
        &ExplosionQuery {
            date: Some(now),
            max_depth: Some(1),
        },
    )
    .await
    .expect("shallow explosion");

    assert_eq!(shallow.len(), 1);
    assert_eq!(shallow[0].component_item_id, part_b);
}

// ---- Where-used reverse lookup ----

#[tokio::test]
#[serial]
async fn where_used_returns_correct_parents() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();

    let component = Uuid::new_v4();
    let parent1 = Uuid::new_v4();
    let parent2 = Uuid::new_v4();

    for (part_id, label) in [(parent1, "P1-1"), (parent2, "P2-1")] {
        create_effective_bom(
            &pool,
            &tenant,
            &corr,
            part_id,
            label,
            &[(component, 1.0)],
            now - Duration::hours(1),
        )
        .await;
    }

    let results = bom_service::where_used(
        &pool,
        &tenant,
        component,
        &WhereUsedQuery { date: Some(now) },
    )
    .await
    .expect("where_used");

    assert_eq!(results.len(), 2);
    let parent_ids: Vec<Uuid> = results.iter().map(|r| r.part_id).collect();
    assert!(parent_ids.contains(&parent1));
    assert!(parent_ids.contains(&parent2));
}
