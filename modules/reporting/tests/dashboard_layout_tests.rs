//! Integration tests for dashboard layout framework.
//!
//! All tests run against real Postgres (REPORTING_DATABASE_URL, port 5443).
//! No mocks, no stubs.

mod helpers;

use helpers::{setup_db, unique_tenant};
use reporting::domain::dashboards::{
    models::WidgetInput,
    service::{create_layout, get_layout, get_widgets, list_layouts, update_widget_positions},
};
use serial_test::serial;

fn sample_widgets() -> Vec<WidgetInput> {
    vec![
        WidgetInput {
            widget_type: "chart".to_string(),
            title: "Revenue Trend".to_string(),
            report_query: "trial_balance".to_string(),
            position_x: 0,
            position_y: 0,
            width: 6,
            height: 4,
            display_config: serde_json::json!({"chart_type": "line"}),
        },
        WidgetInput {
            widget_type: "table".to_string(),
            title: "AR Aging Summary".to_string(),
            report_query: "ar_aging".to_string(),
            position_x: 6,
            position_y: 0,
            width: 6,
            height: 4,
            display_config: serde_json::json!({"columns": ["customer", "total"]}),
        },
    ]
}

// ═══════════════════════════════════════════════════════════════════════════════
// 1. LAYOUT CRUD E2E
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn layout_crud_create_update_positions_and_verify() {
    let pool = setup_db().await;
    let tid = unique_tenant().to_string();
    let widgets = sample_widgets();

    // Create layout with 2 widgets
    let layout = create_layout(
        &pool,
        &tid,
        "Finance Overview",
        Some("Main finance dashboard"),
        &widgets,
        None,
    )
    .await
    .expect("Create layout should succeed");

    assert_eq!(layout.tenant_id, tid);
    assert_eq!(layout.name, "Finance Overview");
    assert_eq!(
        layout.description.as_deref(),
        Some("Main finance dashboard")
    );
    assert_eq!(layout.version, 1);

    // Verify layout can be fetched
    let fetched = get_layout(&pool, &tid, layout.id)
        .await
        .expect("Get should succeed")
        .expect("Layout should exist");
    assert_eq!(fetched.id, layout.id);

    // Get widgets
    let ws = get_widgets(&pool, &tid, layout.id)
        .await
        .expect("Get widgets should succeed");
    assert_eq!(ws.len(), 2);

    // Update widget positions
    let updates: Vec<(uuid::Uuid, i32, i32)> = ws
        .iter()
        .map(|w| (w.id, w.position_x + 1, w.position_y + 2))
        .collect();
    let updated_layout = update_widget_positions(&pool, &tid, layout.id, &updates)
        .await
        .expect("Update positions should succeed");

    assert_eq!(updated_layout.version, 2);

    // Verify updated positions persisted
    let ws_after = get_widgets(&pool, &tid, layout.id)
        .await
        .expect("Get widgets should succeed");
    for (original, updated) in ws.iter().zip(ws_after.iter()) {
        assert_eq!(updated.position_x, original.position_x + 1);
        assert_eq!(updated.position_y, original.position_y + 2);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 2. WIDGET CONFIG TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn widget_config_multiple_types_stored_correctly() {
    let pool = setup_db().await;
    let tid = unique_tenant().to_string();

    let widgets = vec![
        WidgetInput {
            widget_type: "chart".to_string(),
            title: "Revenue Chart".to_string(),
            report_query: "trial_balance".to_string(),
            position_x: 0,
            position_y: 0,
            width: 6,
            height: 4,
            display_config: serde_json::json!({"chart_type": "bar", "stacked": true}),
        },
        WidgetInput {
            widget_type: "table".to_string(),
            title: "AP Aging Table".to_string(),
            report_query: "ap_aging".to_string(),
            position_x: 6,
            position_y: 0,
            width: 6,
            height: 3,
            display_config: serde_json::json!({"sortable": true, "page_size": 25}),
        },
        WidgetInput {
            widget_type: "kpi".to_string(),
            title: "Cash Balance".to_string(),
            report_query: "kpi_cash".to_string(),
            position_x: 0,
            position_y: 4,
            width: 3,
            height: 2,
            display_config: serde_json::json!({"format": "currency", "trend": true}),
        },
    ];

    let layout = create_layout(&pool, &tid, "Multi-Widget Dashboard", None, &widgets, None)
        .await
        .expect("Create layout should succeed");

    let ws = get_widgets(&pool, &tid, layout.id)
        .await
        .expect("Get widgets should succeed");
    assert_eq!(ws.len(), 3);

    // Verify each widget type and its config
    let chart = ws
        .iter()
        .find(|w| w.widget_type == "chart")
        .expect("chart widget");
    assert_eq!(chart.title, "Revenue Chart");
    assert_eq!(chart.display_config["chart_type"], "bar");
    assert_eq!(chart.display_config["stacked"], true);
    assert_eq!(chart.width, 6);
    assert_eq!(chart.height, 4);

    let table = ws
        .iter()
        .find(|w| w.widget_type == "table")
        .expect("table widget");
    assert_eq!(table.title, "AP Aging Table");
    assert_eq!(table.display_config["sortable"], true);
    assert_eq!(table.display_config["page_size"], 25);
    assert_eq!(table.width, 6);
    assert_eq!(table.height, 3);

    let kpi = ws
        .iter()
        .find(|w| w.widget_type == "kpi")
        .expect("kpi widget");
    assert_eq!(kpi.title, "Cash Balance");
    assert_eq!(kpi.display_config["format"], "currency");
    assert_eq!(kpi.display_config["trend"], true);
    assert_eq!(kpi.width, 3);
    assert_eq!(kpi.height, 2);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 3. TENANT ISOLATION TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn tenant_isolation_layouts_invisible_across_tenants() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant().to_string();
    let tenant_b = unique_tenant().to_string();
    let widgets = sample_widgets();

    // Create layout under tenant A
    let layout_a = create_layout(&pool, &tenant_a, "Tenant A Dashboard", None, &widgets, None)
        .await
        .expect("Tenant A layout should succeed");
    assert_eq!(layout_a.tenant_id, tenant_a);

    // Tenant B should see zero layouts
    let layouts_b = list_layouts(&pool, &tenant_b)
        .await
        .expect("list should succeed");
    assert!(
        layouts_b.is_empty(),
        "Tenant B must not see tenant A's layouts"
    );

    // Tenant B should not be able to fetch tenant A's layout by ID
    let fetched = get_layout(&pool, &tenant_b, layout_a.id)
        .await
        .expect("get should succeed");
    assert!(
        fetched.is_none(),
        "Tenant B must not access tenant A's layout by ID"
    );

    // Tenant B should see no widgets from tenant A's layout
    let widgets_b = get_widgets(&pool, &tenant_b, layout_a.id)
        .await
        .expect("get widgets should succeed");
    assert!(
        widgets_b.is_empty(),
        "Tenant B must not see tenant A's widgets"
    );

    // Tenant A should see their layout
    let layouts_a = list_layouts(&pool, &tenant_a)
        .await
        .expect("list should succeed");
    assert_eq!(layouts_a.len(), 1);
    assert_eq!(layouts_a[0].id, layout_a.id);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 4. IDEMPOTENCY TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn idempotent_layout_creation_no_duplicate() {
    let pool = setup_db().await;
    let tid = unique_tenant().to_string();
    let key = format!("idem-{}", uuid::Uuid::new_v4());
    let widgets = sample_widgets();

    // First request
    let layout1 = create_layout(
        &pool,
        &tid,
        "Idempotent Dashboard",
        None,
        &widgets,
        Some(&key),
    )
    .await
    .expect("First create should succeed");

    // Second request with same key
    let layout2 = create_layout(
        &pool,
        &tid,
        "Idempotent Dashboard",
        None,
        &widgets,
        Some(&key),
    )
    .await
    .expect("Second create should return existing");

    assert_eq!(
        layout1.id, layout2.id,
        "Same idempotency key must return same layout"
    );

    // Verify only one layout exists
    let layouts = list_layouts(&pool, &tid)
        .await
        .expect("list should succeed");
    assert_eq!(layouts.len(), 1, "No duplicate layout should be created");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 5. OUTBOX EVENT TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn outbox_event_emitted_with_correct_type_and_tenant_id() {
    let pool = setup_db().await;
    let tid = unique_tenant().to_string();
    let widgets = sample_widgets();

    let layout = create_layout(&pool, &tid, "Event Dashboard", None, &widgets, None)
        .await
        .expect("Create layout should succeed");

    // Query the outbox for the creation event
    let event: (String, serde_json::Value, String) = sqlx::query_as(
        r#"SELECT event_type, payload, tenant_id
           FROM events_outbox
           WHERE aggregate_type = 'dashboard_layout' AND aggregate_id = $1
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(layout.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("Outbox event should exist");

    let (event_type, payload, tenant_id) = event;

    assert_eq!(event_type, "reporting.dashboard_layout.created");
    assert_eq!(tenant_id, tid);

    // Verify payload contains layout_id, name, and widget_count
    let inner = &payload["payload"];
    assert_eq!(inner["layout_id"], layout.id.to_string());
    assert_eq!(inner["name"], "Event Dashboard");
    assert_eq!(inner["widget_count"], 2);

    // Verify envelope has tenant_id
    assert_eq!(payload["tenant_id"], tid);
}
