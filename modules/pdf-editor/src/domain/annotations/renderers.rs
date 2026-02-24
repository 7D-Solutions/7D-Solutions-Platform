//! Per-annotation-type rendering functions.
//!
//! Each function takes a mutable page reference, font tokens, and an annotation,
//! and renders the annotation onto the page using pdfium-render page-level APIs.

use pdfium_render::prelude::*;

use super::render::{FontTokens, RenderError};
use super::types::{Annotation, ShapeType, StampType};

fn to_pdf_color(hex: &str) -> Result<PdfColor, RenderError> {
    let (r, g, b) = super::render::parse_hex_color(hex)?;
    Ok(PdfColor::new(r, g, b, 255))
}

fn to_pdf_color_with_alpha(hex: &str, alpha: f32) -> Result<PdfColor, RenderError> {
    let (r, g, b) = super::render::parse_hex_color(hex)?;
    Ok(PdfColor::new(r, g, b, (alpha * 255.0) as u8))
}

fn rect(left: f32, bottom: f32, right: f32, top: f32) -> PdfRect {
    PdfRect::new_from_values(bottom, left, top, right)
}

fn resolve_color(c: Option<&str>, default: PdfColor) -> Result<PdfColor, RenderError> {
    c.map(to_pdf_color).transpose().map(|c| c.unwrap_or(default))
}

pub(crate) fn render_text(
    page: &mut PdfPage,
    fonts: &FontTokens,
    ann: &Annotation,
    pdf_y: f32,
) -> Result<(), RenderError> {
    let text = ann.text.as_deref().unwrap_or("");
    if text.is_empty() {
        return Ok(());
    }
    let font_size = ann.font_size.unwrap_or(14.0);
    let color = resolve_color(ann.color.as_deref(), PdfColor::new(0, 0, 0, 255))?;

    let mut obj = page.objects_mut().create_text_object(
        PdfPoints::new(ann.x),
        PdfPoints::new(pdf_y - font_size),
        text,
        fonts.helvetica,
        PdfPoints::new(font_size),
    )?;
    obj.set_fill_color(color)?;
    Ok(())
}

pub(crate) fn render_arrow(
    page: &mut PdfPage,
    ann: &Annotation,
    page_height: f32,
) -> Result<(), RenderError> {
    let x2 = ann.x2.unwrap_or(ann.x + 50.0);
    let y2 = ann.y2.unwrap_or(ann.y);
    let stroke_w = ann.stroke_width.unwrap_or(2.0);
    let color = resolve_color(ann.color.as_deref(), PdfColor::new(255, 0, 0, 255))?;
    let pdf_y1 = page_height - ann.y;
    let pdf_y2 = page_height - y2;

    page.objects_mut().create_path_object_line(
        PdfPoints::new(ann.x), PdfPoints::new(pdf_y1),
        PdfPoints::new(x2), PdfPoints::new(pdf_y2),
        color, PdfPoints::new(stroke_w),
    )?;

    let head_size = ann.arrowhead_size.unwrap_or(10.0);
    let dx = x2 - ann.x;
    let dy = y2 - ann.y;
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let (ux, uy) = (dx / len, dy / len);

    let ax1 = x2 - head_size * (ux + 0.4 * uy);
    let ay1 = y2 - head_size * (uy - 0.4 * ux);
    let ax2 = x2 - head_size * (ux - 0.4 * uy);
    let ay2 = y2 - head_size * (uy + 0.4 * ux);

    page.objects_mut().create_path_object_line(
        PdfPoints::new(x2), PdfPoints::new(pdf_y2),
        PdfPoints::new(ax1), PdfPoints::new(page_height - ay1),
        color, PdfPoints::new(stroke_w),
    )?;
    page.objects_mut().create_path_object_line(
        PdfPoints::new(x2), PdfPoints::new(pdf_y2),
        PdfPoints::new(ax2), PdfPoints::new(page_height - ay2),
        color, PdfPoints::new(stroke_w),
    )?;
    Ok(())
}

pub(crate) fn render_highlight(
    page: &mut PdfPage,
    ann: &Annotation,
    page_height: f32,
) -> Result<(), RenderError> {
    let opacity = ann.opacity.unwrap_or(0.35);
    let color = ann.color.as_deref()
        .map(|c| to_pdf_color_with_alpha(c, opacity))
        .transpose()?
        .unwrap_or(PdfColor::new(255, 255, 0, (opacity * 255.0) as u8));

    if let Some(rects) = &ann.text_rects {
        for r in rects {
            let bottom = page_height - r.y - r.height;
            page.objects_mut().create_path_object_rect(
                rect(r.x, bottom, r.x + r.width, bottom + r.height),
                Some(color), None, None,
            )?;
        }
    } else {
        let w = ann.width.unwrap_or(100.0);
        let h = ann.height.unwrap_or(20.0);
        let bottom = page_height - ann.y - h;
        page.objects_mut().create_path_object_rect(
            rect(ann.x, bottom, ann.x + w, bottom + h),
            Some(color), None, None,
        )?;
    }
    Ok(())
}

pub(crate) fn render_stamp(
    page: &mut PdfPage,
    fonts: &FontTokens,
    ann: &Annotation,
    pdf_y: f32,
) -> Result<(), RenderError> {
    let stamp_type = ann.stamp_type.as_ref().unwrap_or(&StampType::Approved);
    let stamp_text = match stamp_type {
        StampType::Approved => "APPROVED",
        StampType::Rejected => "REJECTED",
        StampType::FaiRequired => "FAI REQUIRED",
        StampType::Hold => "HOLD",
        StampType::Reviewed => "REVIEWED",
        StampType::Verified => "VERIFIED",
        StampType::Custom => ann.text.as_deref().unwrap_or("CUSTOM"),
    };

    let mut lines = vec![stamp_text.to_string()];
    if let Some(user) = &ann.stamp_username {
        lines.push(user.clone());
    }
    if let Some(date) = &ann.stamp_date {
        let time_part = ann.stamp_time.as_deref().unwrap_or("");
        if time_part.is_empty() {
            lines.push(date.clone());
        } else {
            lines.push(format!("{date} {time_part}"));
        }
    }

    let font_size = ann.font_size.unwrap_or(12.0);
    let w = ann.width.unwrap_or(140.0);
    let h = ann.height.unwrap_or(40.0);
    let bg = resolve_color(ann.bg_color.as_deref(), PdfColor::new(255, 255, 255, 200))?;
    let border = resolve_color(ann.border_color.as_deref(), PdfColor::new(0, 128, 0, 255))?;

    page.objects_mut().create_path_object_rect(
        rect(ann.x, pdf_y - h, ann.x + w, pdf_y),
        Some(bg), Some(PdfPoints::new(2.0)), Some(border),
    )?;

    let text_color = resolve_color(ann.color.as_deref(), PdfColor::new(0, 128, 0, 255))?;
    let line_spacing = ann.font_size.unwrap_or(14.0) * 1.2;

    for (i, line) in lines.iter().enumerate() {
        let size = if i == 0 { font_size } else { font_size * 0.75 };
        let mut obj = page.objects_mut().create_text_object(
            PdfPoints::new(ann.x + 6.0),
            PdfPoints::new(pdf_y - (i as f32 + 1.0) * line_spacing),
            line,
            fonts.helvetica_bold,
            PdfPoints::new(size),
        )?;
        obj.set_fill_color(text_color)?;
    }
    Ok(())
}

pub(crate) fn render_shape(
    page: &mut PdfPage,
    ann: &Annotation,
    page_height: f32,
) -> Result<(), RenderError> {
    let shape = ann.shape_type.as_ref().unwrap_or(&ShapeType::Rectangle);
    let stroke_w = ann.stroke_width.unwrap_or(2.0);
    let stroke_color = ann.border_color.as_deref().or(ann.color.as_deref())
        .map(to_pdf_color).transpose()?
        .unwrap_or(PdfColor::new(0, 0, 0, 255));
    let fill = ann.bg_color.as_deref().map(to_pdf_color).transpose()?;
    let w = ann.width.unwrap_or(100.0);
    let h = ann.height.unwrap_or(60.0);
    let top = page_height - ann.y;

    match shape {
        ShapeType::Rectangle | ShapeType::Polygon | ShapeType::RevisionCloud => {
            page.objects_mut().create_path_object_rect(
                rect(ann.x, top - h, ann.x + w, top),
                fill, Some(PdfPoints::new(stroke_w)), Some(stroke_color),
            )?;
        }
        ShapeType::Circle => {
            let (rx, ry) = (w / 2.0, h / 2.0);
            page.objects_mut().create_path_object_ellipse_at(
                PdfPoints::new(ann.x + rx), PdfPoints::new(top - ry),
                PdfPoints::new(rx), PdfPoints::new(ry),
                fill, Some(PdfPoints::new(stroke_w)), Some(stroke_color),
            )?;
        }
        ShapeType::Line => {
            let x2 = ann.x2.unwrap_or(ann.x + w);
            let y2 = ann.y2.unwrap_or(ann.y);
            page.objects_mut().create_path_object_line(
                PdfPoints::new(ann.x), PdfPoints::new(top),
                PdfPoints::new(x2), PdfPoints::new(page_height - y2),
                stroke_color, PdfPoints::new(stroke_w),
            )?;
        }
    }
    Ok(())
}

pub(crate) fn render_freehand(
    page: &mut PdfPage,
    ann: &Annotation,
    page_height: f32,
) -> Result<(), RenderError> {
    let points = match &ann.path {
        Some(p) if p.len() >= 2 => p,
        _ => return Ok(()),
    };
    let stroke_w = ann.stroke_width.unwrap_or(2.0);
    let color = resolve_color(ann.color.as_deref(), PdfColor::new(0, 0, 0, 255))?;

    for pair in points.windows(2) {
        page.objects_mut().create_path_object_line(
            PdfPoints::new(pair[0].x), PdfPoints::new(page_height - pair[0].y),
            PdfPoints::new(pair[1].x), PdfPoints::new(page_height - pair[1].y),
            color, PdfPoints::new(stroke_w),
        )?;
    }
    Ok(())
}

pub(crate) fn render_bubble(
    page: &mut PdfPage,
    fonts: &FontTokens,
    ann: &Annotation,
    pdf_y: f32,
) -> Result<(), RenderError> {
    let size = ann.bubble_size.unwrap_or(24.0);
    let fill = resolve_color(ann.bubble_color.as_deref(), PdfColor::new(255, 0, 0, 255))?;
    let border = resolve_color(
        ann.bubble_border_color.as_deref(), PdfColor::new(0, 0, 0, 255),
    )?;

    let radius = size / 2.0;
    let cx = ann.x + radius;
    let cy = pdf_y - radius;

    page.objects_mut().create_path_object_circle_at(
        PdfPoints::new(cx), PdfPoints::new(cy), PdfPoints::new(radius),
        Some(fill), Some(PdfPoints::new(1.5)), Some(border),
    )?;

    if let Some(num) = ann.bubble_number {
        let fs = ann.bubble_font_size.unwrap_or(12.0);
        let text_color = resolve_color(
            ann.text_color.as_deref(), PdfColor::new(255, 255, 255, 255),
        )?;
        let num_str = num.to_string();
        let char_w = fs * 0.35 * num_str.len() as f32;
        let mut obj = page.objects_mut().create_text_object(
            PdfPoints::new(cx - char_w),
            PdfPoints::new(cy - fs * 0.35),
            &num_str,
            fonts.helvetica_bold,
            PdfPoints::new(fs),
        )?;
        obj.set_fill_color(text_color)?;
    }

    if ann.has_leader_line.unwrap_or(false) {
        if let (Some(lx), Some(ly)) = (ann.leader_x, ann.leader_y) {
            let lc = resolve_color(ann.leader_color.as_deref(), border)?;
            let lw = ann.leader_stroke_width.unwrap_or(1.5);
            page.objects_mut().create_path_object_line(
                PdfPoints::new(cx), PdfPoints::new(cy),
                PdfPoints::new(lx), PdfPoints::new(pdf_y + radius - ly + ann.y),
                lc, PdfPoints::new(lw),
            )?;
        }
    }
    Ok(())
}

pub(crate) fn render_signature(
    page: &mut PdfPage,
    fonts: &FontTokens,
    ann: &Annotation,
    pdf_y: f32,
) -> Result<(), RenderError> {
    match ann.signature_method.as_deref().unwrap_or("TEXT") {
        "DRAW" => render_signature_draw(page, ann, pdf_y),
        "UPLOAD" => {
            if let Some(data_url) = &ann.signature_image {
                render_base64_image(page, data_url, ann, pdf_y)
            } else {
                Ok(())
            }
        }
        _ => render_signature_text(page, fonts, ann, pdf_y),
    }
}

fn render_signature_draw(
    page: &mut PdfPage,
    ann: &Annotation,
    pdf_y: f32,
) -> Result<(), RenderError> {
    let points = match &ann.signature_path {
        Some(p) => p,
        None => return Ok(()),
    };
    let color = resolve_color(ann.color.as_deref(), PdfColor::new(0, 0, 0, 255))?;
    let stroke_w = ann.stroke_width.unwrap_or(2.0);

    let mut i = 0;
    while i < points.len() {
        if i + 1 < points.len() && !points[i + 1].new_stroke.unwrap_or(false) {
            page.objects_mut().create_path_object_line(
                PdfPoints::new(ann.x + points[i].x),
                PdfPoints::new(pdf_y - points[i].y),
                PdfPoints::new(ann.x + points[i + 1].x),
                PdfPoints::new(pdf_y - points[i + 1].y),
                color, PdfPoints::new(stroke_w),
            )?;
        }
        i += 1;
    }
    Ok(())
}

fn render_signature_text(
    page: &mut PdfPage,
    fonts: &FontTokens,
    ann: &Annotation,
    pdf_y: f32,
) -> Result<(), RenderError> {
    let text = ann.signature_text.as_deref()
        .or(ann.text.as_deref())
        .unwrap_or("Signature");
    let font_size = ann.font_size.unwrap_or(18.0);
    let color = resolve_color(ann.color.as_deref(), PdfColor::new(0, 0, 128, 255))?;

    let mut obj = page.objects_mut().create_text_object(
        PdfPoints::new(ann.x),
        PdfPoints::new(pdf_y - font_size),
        text,
        fonts.helvetica_oblique,
        PdfPoints::new(font_size),
    )?;
    obj.set_fill_color(color)?;
    Ok(())
}

fn render_base64_image(
    page: &mut PdfPage,
    data_url: &str,
    ann: &Annotation,
    pdf_y: f32,
) -> Result<(), RenderError> {
    let b64_data = if let Some(pos) = data_url.find(',') {
        &data_url[pos + 1..]
    } else {
        data_url
    };

    use base64::Engine;
    let image_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64_data)
        .map_err(|e| RenderError::Base64(e.to_string()))?;
    let dyn_image = image::load_from_memory(&image_bytes)
        .map_err(|e| RenderError::ImageDecode(e.to_string()))?;

    let w = ann.width.unwrap_or(150.0);
    let aspect = dyn_image.height() as f32 / dyn_image.width() as f32;
    let h = w * aspect;

    page.objects_mut().create_image_object(
        PdfPoints::new(ann.x),
        PdfPoints::new(pdf_y - h),
        &dyn_image,
        Some(PdfPoints::new(w)),
        None,
    )?;
    Ok(())
}
