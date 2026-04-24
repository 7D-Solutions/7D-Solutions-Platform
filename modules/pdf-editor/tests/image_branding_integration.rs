//! Integration tests for branding image upload, injection, tenant isolation,
//! format validation, idempotency, and outbox events (bd-1yal9).

mod submission_helpers;

use pdf_editor::domain::generate::{inject_images, ImageInjection, ImageInjectionPoint};
use pdf_editor::domain::images::{
    ImageError, ImageRepo, ListImagesQuery, UploadImageRequest, MAX_IMAGE_SIZE,
};
use serial_test::serial;
use submission_helpers::{setup_db, unique_tenant};
use uuid::Uuid;

/// Create a minimal valid 1×1 red PNG in memory.
fn tiny_png() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        use image::ImageEncoder;
        // 1×1 red pixel (RGBA)
        let pixel: [u8; 4] = [255, 0, 0, 255];
        encoder
            .write_image(&pixel, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
    }
    buf
}

/// Create a minimal valid 2×2 JPEG in memory.
fn tiny_jpeg() -> Vec<u8> {
    use image::{ImageBuffer, Rgb};
    let img = ImageBuffer::from_pixel(2, 2, Rgb([0u8, 0, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
    buf.into_inner()
}

// ============================================================================
// 1. Image upload E2E
// ============================================================================

#[tokio::test]
#[serial]
async fn image_upload_e2e_persists_with_correct_metadata() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let png_data = tiny_png();

    let img = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("logo-{}", Uuid::new_v4()),
            name: "Company Logo".into(),
            image_format: "png".into(),
            image_data: png_data.clone(),
            placement: "header_logo".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    assert_eq!(img.tenant_id, tid);
    assert_eq!(img.name, "Company Logo");
    assert_eq!(img.image_format, "png");
    assert_eq!(img.placement, "header_logo");
    assert_eq!(img.size_bytes, png_data.len() as i64);
    assert_eq!(img.width_px, Some(1));
    assert_eq!(img.height_px, Some(1));
    assert_eq!(img.image_data, png_data);

    // Verify we can retrieve it
    let found = ImageRepo::find_by_id(&pool, img.id, &tid)
        .await
        .unwrap()
        .expect("image should be found");
    assert_eq!(found.id, img.id);
    assert_eq!(found.name, "Company Logo");
}

#[tokio::test]
#[serial]
async fn image_upload_jpeg_detects_dimensions() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let jpeg_data = tiny_jpeg();

    let img = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("jpeg-{}", Uuid::new_v4()),
            name: "Footer Banner".into(),
            image_format: "jpeg".into(),
            image_data: jpeg_data,
            placement: "footer_branding".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    assert_eq!(img.image_format, "jpeg");
    assert_eq!(img.width_px, Some(2));
    assert_eq!(img.height_px, Some(2));
    assert_eq!(img.placement, "footer_branding");
}

// ============================================================================
// 2. Template injection test
// ============================================================================

#[tokio::test]
#[serial]
async fn template_injection_renders_image_into_pdf() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let png_data = tiny_png();

    // Upload an image
    let img = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("inject-{}", Uuid::new_v4()),
            name: "Header Logo".into(),
            image_format: "png".into(),
            image_data: png_data,
            placement: "header_logo".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    // Load the test PDF
    let pdf_bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/test.pdf"
    ))
    .unwrap();

    // Inject the uploaded image at header position
    let injections = vec![ImageInjection {
        image_data: img.image_data.clone(),
        image_format: img.image_format.clone(),
        injection_point: ImageInjectionPoint {
            x: 50.0,
            y: 20.0,
            width: 100.0,
            page: Some(1),
            placement: "header_logo".into(),
        },
    }];

    let result = inject_images(&pdf_bytes, &injections).unwrap();

    // Output should be valid PDF and larger than input (contains image data)
    assert!(result.starts_with(b"%PDF-"));
    assert!(
        result.len() > pdf_bytes.len(),
        "output PDF should be larger than input (image was injected)"
    );
}

#[tokio::test]
#[serial]
async fn template_injection_with_multiple_placements() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let png_data = tiny_png();

    let header_img = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("header-{}", Uuid::new_v4()),
            name: "Header".into(),
            image_format: "png".into(),
            image_data: png_data.clone(),
            placement: "header_logo".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    let footer_img = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("footer-{}", Uuid::new_v4()),
            name: "Footer".into(),
            image_format: "png".into(),
            image_data: png_data.clone(),
            placement: "footer_branding".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    let pdf_bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/test.pdf"
    ))
    .unwrap();

    let injections = vec![
        ImageInjection {
            image_data: header_img.image_data,
            image_format: header_img.image_format,
            injection_point: ImageInjectionPoint {
                x: 50.0,
                y: 20.0,
                width: 100.0,
                page: Some(1),
                placement: "header_logo".into(),
            },
        },
        ImageInjection {
            image_data: footer_img.image_data,
            image_format: footer_img.image_format,
            injection_point: ImageInjectionPoint {
                x: 50.0,
                y: 750.0,
                width: 80.0,
                page: Some(1),
                placement: "footer_branding".into(),
            },
        },
    ];

    let result = inject_images(&pdf_bytes, &injections).unwrap();
    assert!(result.starts_with(b"%PDF-"));
    assert!(result.len() > pdf_bytes.len());
}

// ============================================================================
// 3. Tenant isolation test
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_b_cannot_see_tenant_a_images() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let img = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid_a.clone(),
            idempotency_key: format!("iso-{}", Uuid::new_v4()),
            name: "Secret Logo".into(),
            image_format: "png".into(),
            image_data: tiny_png(),
            placement: "header_logo".into(),
            created_by: "admin-a".into(),
        },
    )
    .await
    .unwrap();

    // find_by_id returns None for wrong tenant
    assert!(ImageRepo::find_by_id(&pool, img.id, &tid_b)
        .await
        .unwrap()
        .is_none());

    // list returns empty for wrong tenant
    let list = ImageRepo::list(
        &pool,
        &ListImagesQuery {
            tenant_id: tid_b,
            placement: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(list.is_empty(), "Tenant B should see zero images");

    // Tenant A can see their own image
    let own_list = ImageRepo::list(
        &pool,
        &ListImagesQuery {
            tenant_id: tid_a,
            placement: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(own_list.len(), 1);
    assert_eq!(own_list[0].id, img.id);
}

// ============================================================================
// 4. Format validation test
// ============================================================================

#[tokio::test]
#[serial]
async fn rejects_unsupported_image_format() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let err = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("bmp-{}", Uuid::new_v4()),
            name: "Bad Format".into(),
            image_format: "bmp".into(),
            image_data: vec![0x42, 0x4D, 0x00, 0x00],
            placement: "header_logo".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, ImageError::UnsupportedFormat(f) if f == "bmp"));
}

#[tokio::test]
#[serial]
async fn rejects_gif_format() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let err = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("gif-{}", Uuid::new_v4()),
            name: "Animated".into(),
            image_format: "gif".into(),
            image_data: vec![0x47, 0x49, 0x46, 0x38],
            placement: "inline".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, ImageError::UnsupportedFormat(f) if f == "gif"));
}

#[tokio::test]
#[serial]
async fn rejects_oversized_image() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let big_data = vec![0u8; MAX_IMAGE_SIZE + 1];
    let err = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("big-{}", Uuid::new_v4()),
            name: "Huge".into(),
            image_format: "png".into(),
            image_data: big_data,
            placement: "header_logo".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, ImageError::TooLarge));
}

#[tokio::test]
#[serial]
async fn rejects_empty_image_data() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let err = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("empty-{}", Uuid::new_v4()),
            name: "Empty".into(),
            image_format: "png".into(),
            image_data: vec![],
            placement: "header_logo".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, ImageError::Validation(_)));
}

// ============================================================================
// 5. Idempotency test
// ============================================================================

#[tokio::test]
#[serial]
async fn idempotent_upload_returns_same_image_no_duplicate() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let idem_key = format!("idem-{}", Uuid::new_v4());
    let png_data = tiny_png();

    let first = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: idem_key.clone(),
            name: "Logo".into(),
            image_format: "png".into(),
            image_data: png_data.clone(),
            placement: "header_logo".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    let second = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: idem_key.clone(),
            name: "Logo v2".into(),
            image_format: "png".into(),
            image_data: png_data,
            placement: "header_logo".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    // Same ID — no duplicate created
    assert_eq!(first.id, second.id);
    // Original name preserved
    assert_eq!(second.name, "Logo");

    // Only one image in the list
    let list = ImageRepo::list(
        &pool,
        &ListImagesQuery {
            tenant_id: tid,
            placement: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(list.len(), 1);
}

// ============================================================================
// 6. Outbox event test
// ============================================================================

#[tokio::test]
#[serial]
async fn upload_creates_outbox_event_with_correct_type_and_tenant() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let img = ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("evt-{}", Uuid::new_v4()),
            name: "Event Logo".into(),
            image_format: "png".into(),
            image_data: tiny_png(),
            placement: "header_logo".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    // Query the outbox for the event
    let row: (String, String, serde_json::Value) = sqlx::query_as(
        r#"
        SELECT event_type, tenant_id, payload
        FROM events_outbox
        WHERE tenant_id = $1 AND event_type = 'pdf.image.uploaded'
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("outbox event should exist");

    assert_eq!(row.0, "pdf.image.uploaded");
    assert_eq!(row.1, tid);

    // Verify payload contains the image_id
    let payload = &row.2;
    let inner = payload
        .get("payload")
        .expect("envelope should have payload field");
    assert_eq!(
        inner.get("image_id").and_then(|v| v.as_str()),
        Some(img.id.to_string()).as_deref()
    );
    assert_eq!(
        inner.get("tenant_id").and_then(|v| v.as_str()),
        Some(tid.as_str())
    );
    assert_eq!(
        inner.get("image_format").and_then(|v| v.as_str()),
        Some("png")
    );
}

// ============================================================================
// List by placement filter
// ============================================================================

#[tokio::test]
#[serial]
async fn list_filters_by_placement() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("h-{}", Uuid::new_v4()),
            name: "Header".into(),
            image_format: "png".into(),
            image_data: tiny_png(),
            placement: "header_logo".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    ImageRepo::upload(
        &pool,
        &UploadImageRequest {
            tenant_id: tid.clone(),
            idempotency_key: format!("f-{}", Uuid::new_v4()),
            name: "Footer".into(),
            image_format: "png".into(),
            image_data: tiny_png(),
            placement: "footer_branding".into(),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    let headers = ImageRepo::list(
        &pool,
        &ListImagesQuery {
            tenant_id: tid.clone(),
            placement: Some("header_logo".into()),
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].placement, "header_logo");

    let all = ImageRepo::list(
        &pool,
        &ListImagesQuery {
            tenant_id: tid,
            placement: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(all.len(), 2);
}
