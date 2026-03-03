//! Integration tests for table render primitives (bd-120qw).
//!
//! Covers: simple table render E2E, pagination, border styling,
//! tenant isolation, idempotency, and determinism.

mod submission_helpers;

use pdf_editor_rs::domain::tables::{
    BorderConfig, RenderTableRequest, TableColumn, TableDefinition, TableRenderRepo, TableRow,
};
use serial_test::serial;
use submission_helpers::{setup_db, unique_tenant};
use uuid::Uuid;

/// Load the test PDF fixture.
fn test_pdf_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/test.pdf"
    ))
    .expect("test.pdf fixture must exist")
}

/// Build a simple 3×3 table definition.
fn simple_3x3_table() -> TableDefinition {
    TableDefinition {
        columns: vec![
            TableColumn {
                header: "Part Number".into(),
                width: 120.0,
            },
            TableColumn {
                header: "Description".into(),
                width: 200.0,
            },
            TableColumn {
                header: "Qty".into(),
                width: 60.0,
            },
        ],
        rows: vec![
            TableRow {
                cells: vec!["PN-001".into(), "Turbine Blade".into(), "4".into()],
            },
            TableRow {
                cells: vec!["PN-002".into(), "Compressor Disk".into(), "2".into()],
            },
            TableRow {
                cells: vec!["PN-003".into(), "Combustion Liner".into(), "1".into()],
            },
        ],
        x: 50.0,
        y: 100.0,
        page: 1,
        font_size: 10.0,
        row_height: 20.0,
        border: BorderConfig::default(),
    }
}

// ============================================================================
// 1. Simple table render E2E
// ============================================================================

#[tokio::test]
#[serial]
async fn simple_table_render_e2e() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let pdf_bytes = test_pdf_bytes();

    let result = TableRenderRepo::render(
        &pool,
        &RenderTableRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("simple-{}", Uuid::new_v4()),
            table_definition: simple_3x3_table(),
            pdf_template: pdf_bytes.clone(),
        },
    )
    .await
    .unwrap();

    assert_eq!(result.tenant_id, tid);
    assert_eq!(result.status, "rendered");
    assert!(result.error_message.is_none());
    assert!(result.rendered_at.is_some());

    let output = result.pdf_output.expect("rendered request should have PDF output");
    assert!(output.starts_with(b"%PDF-"), "output must be valid PDF");
    assert!(
        output.len() > pdf_bytes.len(),
        "output PDF ({} bytes) should be larger than input ({} bytes)",
        output.len(),
        pdf_bytes.len()
    );

    // Verify outbox event was created
    let event: Option<(String, String)> = sqlx::query_as(
        r#"
        SELECT event_type, tenant_id
        FROM events_outbox
        WHERE tenant_id = $1 AND event_type = 'pdf.table.rendered'
        ORDER BY created_at DESC LIMIT 1
        "#,
    )
    .bind(&tid)
    .fetch_optional(&pool)
    .await
    .unwrap();

    let (event_type, event_tid) = event.expect("pdf.table.rendered event should be in outbox");
    assert_eq!(event_type, "pdf.table.rendered");
    assert_eq!(event_tid, tid);
}

// ============================================================================
// 2. Pagination test
// ============================================================================

#[tokio::test]
#[serial]
async fn pagination_creates_new_pages_for_long_tables() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let pdf_bytes = test_pdf_bytes();

    // Create a table with enough rows to span multiple pages.
    // A standard PDF page is ~842 points tall (A4). Starting at y=100 with
    // row_height=20 and bottom_margin=50, we can fit ~(842-100-50)/20 = ~34 rows
    // on the first page. 80 rows should need at least 3 pages.
    let many_rows: Vec<TableRow> = (1..=80)
        .map(|i| TableRow {
            cells: vec![
                format!("PN-{:03}", i),
                format!("Part description {}", i),
                format!("{}", i * 2),
            ],
        })
        .collect();

    let table = TableDefinition {
        columns: vec![
            TableColumn {
                header: "Part".into(),
                width: 100.0,
            },
            TableColumn {
                header: "Description".into(),
                width: 200.0,
            },
            TableColumn {
                header: "Qty".into(),
                width: 60.0,
            },
        ],
        rows: many_rows,
        x: 50.0,
        y: 100.0,
        page: 1,
        font_size: 10.0,
        row_height: 20.0,
        border: BorderConfig::default(),
    };

    let result = TableRenderRepo::render(
        &pool,
        &RenderTableRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("paginate-{}", Uuid::new_v4()),
            table_definition: table,
            pdf_template: pdf_bytes,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.status, "rendered");
    let output = result.pdf_output.expect("should have PDF output");
    assert!(output.starts_with(b"%PDF-"));

    // Verify the output has more pages than input by loading with pdfium
    use pdf_editor_rs::domain::generate::create_pdfium;
    let pdfium = create_pdfium().unwrap();
    let doc = pdfium.load_pdf_from_byte_slice(&output, None).unwrap();
    let page_count = doc.pages().len();

    assert!(
        page_count >= 3,
        "80 rows should span at least 3 pages, got {}",
        page_count
    );
}

// ============================================================================
// 3. Border styling test
// ============================================================================

#[tokio::test]
#[serial]
async fn border_styling_produces_different_outputs() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let pdf_bytes = test_pdf_bytes();

    // Full borders (default)
    let full_border_table = simple_3x3_table();

    let full = TableRenderRepo::render(
        &pool,
        &RenderTableRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("border-full-{}", Uuid::new_v4()),
            table_definition: full_border_table,
            pdf_template: pdf_bytes.clone(),
        },
    )
    .await
    .unwrap();

    // No borders
    let mut no_border_table = simple_3x3_table();
    no_border_table.border = BorderConfig {
        outer: false,
        inner_horizontal: false,
        inner_vertical: false,
        width: 0.0,
    };

    let none = TableRenderRepo::render(
        &pool,
        &RenderTableRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("border-none-{}", Uuid::new_v4()),
            table_definition: no_border_table,
            pdf_template: pdf_bytes.clone(),
        },
    )
    .await
    .unwrap();

    // Outer-only borders
    let mut outer_only_table = simple_3x3_table();
    outer_only_table.border = BorderConfig {
        outer: true,
        inner_horizontal: false,
        inner_vertical: false,
        width: 2.0,
    };

    let outer = TableRenderRepo::render(
        &pool,
        &RenderTableRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("border-outer-{}", Uuid::new_v4()),
            table_definition: outer_only_table,
            pdf_template: pdf_bytes,
        },
    )
    .await
    .unwrap();

    let full_out = full.pdf_output.expect("full border output");
    let none_out = none.pdf_output.expect("no border output");
    let outer_out = outer.pdf_output.expect("outer border output");

    // All three should be valid PDFs
    assert!(full_out.starts_with(b"%PDF-"));
    assert!(none_out.starts_with(b"%PDF-"));
    assert!(outer_out.starts_with(b"%PDF-"));

    // All three should differ (different border configurations)
    assert_ne!(full_out, none_out, "full borders vs no borders should differ");
    assert_ne!(
        full_out, outer_out,
        "full borders vs outer-only should differ"
    );
    assert_ne!(
        none_out, outer_out,
        "no borders vs outer-only should differ"
    );
}

// ============================================================================
// 4. Tenant isolation test
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_b_cannot_see_tenant_a_render_requests() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let pdf_bytes = test_pdf_bytes();

    let request = TableRenderRepo::render(
        &pool,
        &RenderTableRequest {
            tenant_id: tid_a.clone(),
            idempotency_key: format!("iso-{}", Uuid::new_v4()),
            table_definition: simple_3x3_table(),
            pdf_template: pdf_bytes,
        },
    )
    .await
    .unwrap();

    // find_by_id returns None for wrong tenant
    assert!(TableRenderRepo::find_by_id(&pool, request.id, &tid_b)
        .await
        .unwrap()
        .is_none());

    // list returns empty for wrong tenant
    let list = TableRenderRepo::list(&pool, &tid_b, None, None)
        .await
        .unwrap();
    assert!(list.is_empty(), "Tenant B should see zero render requests");

    // Tenant A can see their own request
    let own = TableRenderRepo::find_by_id(&pool, request.id, &tid_a)
        .await
        .unwrap()
        .expect("Tenant A should find own request");
    assert_eq!(own.id, request.id);

    let own_list = TableRenderRepo::list(&pool, &tid_a, None, None)
        .await
        .unwrap();
    assert_eq!(own_list.len(), 1);
}

// ============================================================================
// 5. Idempotency test
// ============================================================================

#[tokio::test]
#[serial]
async fn idempotent_render_returns_same_request_no_duplicate() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let pdf_bytes = test_pdf_bytes();
    let idem_key = format!("idem-{}", Uuid::new_v4());

    let first = TableRenderRepo::render(
        &pool,
        &RenderTableRequest {
            tenant_id: tid.clone(),
            idempotency_key: idem_key.clone(),
            table_definition: simple_3x3_table(),
            pdf_template: pdf_bytes.clone(),
        },
    )
    .await
    .unwrap();

    let second = TableRenderRepo::render(
        &pool,
        &RenderTableRequest {
            tenant_id: tid.clone(),
            idempotency_key: idem_key.clone(),
            table_definition: simple_3x3_table(),
            pdf_template: pdf_bytes,
        },
    )
    .await
    .unwrap();

    // Same ID — no duplicate created
    assert_eq!(first.id, second.id);
    assert_eq!(first.status, second.status);

    // Only one request in the list
    let list = TableRenderRepo::list(&pool, &tid, None, None)
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
}

// ============================================================================
// 6. Determinism test
// ============================================================================

#[tokio::test]
#[serial]
async fn deterministic_render_produces_identical_output() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let pdf_bytes = test_pdf_bytes();

    // Render the same table twice with different idempotency keys
    let first = TableRenderRepo::render(
        &pool,
        &RenderTableRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("det-1-{}", Uuid::new_v4()),
            table_definition: simple_3x3_table(),
            pdf_template: pdf_bytes.clone(),
        },
    )
    .await
    .unwrap();

    let second = TableRenderRepo::render(
        &pool,
        &RenderTableRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("det-2-{}", Uuid::new_v4()),
            table_definition: simple_3x3_table(),
            pdf_template: pdf_bytes,
        },
    )
    .await
    .unwrap();

    let out1 = first.pdf_output.expect("first render output");
    let out2 = second.pdf_output.expect("second render output");

    assert_eq!(
        out1, out2,
        "rendering the same table twice should produce byte-identical PDF output"
    );
}
