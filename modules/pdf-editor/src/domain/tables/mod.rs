//! Table render primitives for rich PDF formatting.
//!
//! Invariants:
//! - Every render request is tenant-scoped; all queries filter by tenant_id
//! - Idempotent via (tenant_id, idempotency_key) unique constraint
//! - Table rendering is deterministic: same input → byte-identical output
//! - Pagination: tables spanning beyond page bounds continue on new pages
//! - Follows Guard → Mutation → Outbox pattern

pub mod repo;

pub use repo::TableRenderRepo;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Table model
// ============================================================================

/// A single column definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableColumn {
    pub header: String,
    pub width: f32,
}

/// A single data row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRow {
    pub cells: Vec<String>,
}

/// Border configuration for the table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderConfig {
    pub outer: bool,
    pub inner_horizontal: bool,
    pub inner_vertical: bool,
    pub width: f32,
}

impl Default for BorderConfig {
    fn default() -> Self {
        Self {
            outer: true,
            inner_horizontal: true,
            inner_vertical: true,
            width: 1.0,
        }
    }
}

/// Complete table definition for rendering onto a PDF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDefinition {
    pub columns: Vec<TableColumn>,
    pub rows: Vec<TableRow>,
    /// X coordinate (top-left origin, PDF points).
    pub x: f32,
    /// Y coordinate (top-left origin, PDF points).
    pub y: f32,
    /// 1-based starting page number.
    pub page: u32,
    pub font_size: f32,
    pub row_height: f32,
    pub border: BorderConfig,
}

// ============================================================================
// DB model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TableRenderRequest {
    pub id: Uuid,
    pub tenant_id: String,
    pub idempotency_key: String,
    pub table_definition: serde_json::Value,
    pub pdf_output: Option<Vec<u8>>,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub rendered_at: Option<DateTime<Utc>>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug)]
pub struct RenderTableRequest {
    pub tenant_id: String,
    pub idempotency_key: String,
    pub table_definition: TableDefinition,
    pub pdf_template: Vec<u8>,
}

// ============================================================================
// Event payload
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct TableRenderedPayload {
    pub tenant_id: String,
    pub render_request_id: Uuid,
    pub status: String,
}

// ============================================================================
// Validation (Guard)
// ============================================================================

pub fn validate_render_request(req: &RenderTableRequest) -> Result<(), TableError> {
    if req.tenant_id.trim().is_empty() {
        return Err(TableError::Validation("tenant_id is required".into()));
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(TableError::Validation(
            "idempotency_key is required".into(),
        ));
    }
    if req.table_definition.columns.is_empty() {
        return Err(TableError::Validation(
            "table must have at least one column".into(),
        ));
    }
    if req.table_definition.rows.is_empty() {
        return Err(TableError::Validation(
            "table must have at least one row".into(),
        ));
    }
    for (i, row) in req.table_definition.rows.iter().enumerate() {
        if row.cells.len() != req.table_definition.columns.len() {
            return Err(TableError::Validation(format!(
                "row {} has {} cells but table has {} columns",
                i,
                row.cells.len(),
                req.table_definition.columns.len()
            )));
        }
    }
    if req.table_definition.font_size <= 0.0 {
        return Err(TableError::Validation("font_size must be positive".into()));
    }
    if req.table_definition.row_height <= 0.0 {
        return Err(TableError::Validation(
            "row_height must be positive".into(),
        ));
    }
    if req.table_definition.page < 1 {
        return Err(TableError::Validation(
            "page must be >= 1".into(),
        ));
    }
    crate::domain::generate::validate_pdf(&req.pdf_template)
        .map_err(|e| TableError::Validation(format!("invalid PDF template: {}", e)))?;
    Ok(())
}

// ============================================================================
// Render logic
// ============================================================================

/// Bottom margin in PDF points (~0.7 inches) to avoid rendering off-page.
const BOTTOM_MARGIN: f32 = 50.0;
/// Padding inside each cell.
const CELL_PADDING: f32 = 4.0;

/// Render a table onto a PDF template. Deterministic: same input → same output.
///
/// Handles pagination by creating new pages when rows exceed available space.
pub fn render_table(
    pdf_bytes: &[u8],
    table: &TableDefinition,
) -> Result<Vec<u8>, TableError> {
    use crate::domain::generate::{create_pdfium, validate_pdf};
    use pdfium_render::prelude::*;

    validate_pdf(pdf_bytes).map_err(|e| TableError::Render(e.to_string()))?;

    let pdfium = create_pdfium().map_err(|e| TableError::Render(e.to_string()))?;
    let mut document = pdfium
        .load_pdf_from_byte_slice(pdf_bytes, None)
        .map_err(|e| TableError::Render(e.to_string()))?;
    let page_count = document.pages().len() as u32;

    if table.page < 1 || table.page > page_count {
        return Err(TableError::Render(format!(
            "page {} out of range (document has {} pages)",
            table.page, page_count
        )));
    }

    let font_token = document.fonts_mut().helvetica();

    // Get initial page dimensions
    let (page_width, page_height) = {
        let page = document
            .pages()
            .get((table.page - 1) as u16)
            .map_err(|e| TableError::Render(e.to_string()))?;
        (page.width().value, page.height().value)
    };

    // Build the list of rows to render: header first, then data rows
    let header_cells: Vec<String> = table.columns.iter().map(|c| c.header.clone()).collect();

    // Track which rows go on which page
    struct PageSlice {
        page_index: u16,
        page_height: f32,
        start_y: f32,
        rows: Vec<(Vec<String>, bool)>, // (cells, is_header)
    }

    let mut slices: Vec<PageSlice> = Vec::new();
    let mut current_page_index = (table.page - 1) as u16;
    let mut cursor_y = table.y;
    let mut current_page_height = page_height;
    let mut existing_page_count = page_count;

    // Header on first page
    let mut current_slice = PageSlice {
        page_index: current_page_index,
        page_height: current_page_height,
        start_y: cursor_y,
        rows: vec![(header_cells.clone(), true)],
    };
    cursor_y += table.row_height;

    for row in &table.rows {
        // Check if this row fits on the current page
        if cursor_y + table.row_height > current_page_height - BOTTOM_MARGIN {
            // Save current slice
            slices.push(current_slice);

            // Move to next page — create one if needed
            current_page_index += 1;
            if (current_page_index as u32) >= existing_page_count {
                document
                    .pages_mut()
                    .create_page_at_end(PdfPagePaperSize::Custom(
                        PdfPoints::new(page_width),
                        PdfPoints::new(page_height),
                    ))
                    .map_err(|e| TableError::Render(e.to_string()))?;
                existing_page_count += 1;
            }

            current_page_height = {
                let p = document
                    .pages()
                    .get(current_page_index)
                    .map_err(|e| TableError::Render(e.to_string()))?;
                p.height().value
            };

            // Reset cursor to top of the new page, with header repeated
            cursor_y = table.y;
            current_slice = PageSlice {
                page_index: current_page_index,
                page_height: current_page_height,
                start_y: cursor_y,
                rows: vec![(header_cells.clone(), true)],
            };
            cursor_y += table.row_height;
        }

        current_slice
            .rows
            .push((row.cells.clone(), false));
        cursor_y += table.row_height;
    }
    slices.push(current_slice);

    // Render each page slice
    for slice in &slices {
        let mut page = document
            .pages_mut()
            .get(slice.page_index)
            .map_err(|e| TableError::Render(e.to_string()))?;
        let ph = slice.page_height;
        let border_color = PdfColor::new(0, 0, 0, 255);

        let mut row_y = slice.start_y;

        for (cells, _is_header) in &slice.rows {
            // Render cell text
            let mut cell_x = table.x;
            for (col_idx, cell_text) in cells.iter().enumerate() {
                let col = &table.columns[col_idx];
                let text_x = cell_x + CELL_PADDING;
                let text_y = row_y + CELL_PADDING;
                let pdf_y = ph - text_y - table.font_size;

                let mut obj = page
                    .objects_mut()
                    .create_text_object(
                        PdfPoints::new(text_x),
                        PdfPoints::new(pdf_y),
                        cell_text,
                        font_token,
                        PdfPoints::new(table.font_size),
                    )
                    .map_err(|e| TableError::Render(e.to_string()))?;
                obj.set_fill_color(PdfColor::new(0, 0, 0, 255))
                    .map_err(|e| TableError::Render(e.to_string()))?;

                cell_x += col.width;
            }
            row_y += table.row_height;
        }

        // Draw borders
        let total_width: f32 = table.columns.iter().map(|c| c.width).sum();
        let total_rows = slice.rows.len();
        let table_bottom_y = slice.start_y + (total_rows as f32) * table.row_height;

        let stroke_w = PdfPoints::new(table.border.width);

        if table.border.outer {
            // Top border
            let pdf_top = ph - slice.start_y;
            let pdf_bottom = ph - table_bottom_y;

            page.objects_mut()
                .create_path_object_line(
                    PdfPoints::new(table.x),
                    PdfPoints::new(pdf_top),
                    PdfPoints::new(table.x + total_width),
                    PdfPoints::new(pdf_top),
                    border_color,
                    stroke_w,
                )
                .map_err(|e| TableError::Render(e.to_string()))?;

            // Bottom border
            page.objects_mut()
                .create_path_object_line(
                    PdfPoints::new(table.x),
                    PdfPoints::new(pdf_bottom),
                    PdfPoints::new(table.x + total_width),
                    PdfPoints::new(pdf_bottom),
                    border_color,
                    stroke_w,
                )
                .map_err(|e| TableError::Render(e.to_string()))?;

            // Left border
            page.objects_mut()
                .create_path_object_line(
                    PdfPoints::new(table.x),
                    PdfPoints::new(pdf_top),
                    PdfPoints::new(table.x),
                    PdfPoints::new(pdf_bottom),
                    border_color,
                    stroke_w,
                )
                .map_err(|e| TableError::Render(e.to_string()))?;

            // Right border
            page.objects_mut()
                .create_path_object_line(
                    PdfPoints::new(table.x + total_width),
                    PdfPoints::new(pdf_top),
                    PdfPoints::new(table.x + total_width),
                    PdfPoints::new(pdf_bottom),
                    border_color,
                    stroke_w,
                )
                .map_err(|e| TableError::Render(e.to_string()))?;
        }

        // Inner horizontal lines
        if table.border.inner_horizontal {
            for i in 1..total_rows {
                let line_y = slice.start_y + (i as f32) * table.row_height;
                let pdf_line_y = ph - line_y;
                page.objects_mut()
                    .create_path_object_line(
                        PdfPoints::new(table.x),
                        PdfPoints::new(pdf_line_y),
                        PdfPoints::new(table.x + total_width),
                        PdfPoints::new(pdf_line_y),
                        border_color,
                        stroke_w,
                    )
                    .map_err(|e| TableError::Render(e.to_string()))?;
            }
        }

        // Inner vertical lines
        if table.border.inner_vertical {
            let mut col_x = table.x;
            for col in table.columns.iter().take(table.columns.len() - 1) {
                col_x += col.width;
                let pdf_top = ph - slice.start_y;
                let pdf_bottom = ph - table_bottom_y;
                page.objects_mut()
                    .create_path_object_line(
                        PdfPoints::new(col_x),
                        PdfPoints::new(pdf_top),
                        PdfPoints::new(col_x),
                        PdfPoints::new(pdf_bottom),
                        border_color,
                        stroke_w,
                    )
                    .map_err(|e| TableError::Render(e.to_string()))?;
            }
        }
    }

    document
        .save_to_bytes()
        .map_err(|e| TableError::Render(e.to_string()))
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum TableError {
    #[error("Render request not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Render error: {0}")]
    Render(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
