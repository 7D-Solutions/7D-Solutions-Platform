//! Tenant-scoped object key construction per ADR-018.
//!
//! Key format:
//! `tenants/{tenant_id}/{service}/{artifact_type}/{entity_id}/{yyyy}/{mm}/{dd}/{object_id}-{safe_filename}`

use chrono::{Datelike, NaiveDate};

/// Builds a tenant-scoped object key per ADR-018.
pub struct BlobKeyBuilder<'a> {
    pub tenant_id: &'a str,
    pub service: &'a str,
    pub artifact_type: &'a str,
    pub entity_id: &'a str,
    pub object_id: &'a str,
    pub filename: &'a str,
}

impl<'a> BlobKeyBuilder<'a> {
    /// Construct the full object key for the given date.
    pub fn build(&self, date: NaiveDate) -> String {
        let safe = normalize_filename(self.filename);
        format!(
            "tenants/{}/{}/{}/{}/{}/{:02}/{:02}/{}-{}",
            self.tenant_id,
            self.service,
            self.artifact_type,
            self.entity_id,
            date.year(),
            date.month(),
            date.day(),
            self.object_id,
            safe,
        )
    }

    /// Convenience: build with today's UTC date.
    pub fn build_today(&self) -> String {
        self.build(chrono::Utc::now().date_naive())
    }
}

/// Normalize a filename to the ADR-018-safe character set: `[-._a-z0-9]`.
///
/// Characters outside the allowed set are replaced with `_`. Leading dots
/// (hidden files) are also replaced so keys are always visible objects.
pub fn normalize_filename(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    // Replace a leading dot so hidden-file names don't pass through.
    if out.starts_with('.') {
        out.replace_range(0..1, "_");
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    #[test]
    fn tenant_isolation_prefix() {
        let k1 = BlobKeyBuilder {
            tenant_id: "tenant-aaa",
            service: "doc-mgmt",
            artifact_type: "upload",
            entity_id: "ent-001",
            object_id: "obj-001",
            filename: "report.pdf",
        }
        .build(date(2026, 4, 6));

        let k2 = BlobKeyBuilder {
            tenant_id: "tenant-bbb",
            service: "doc-mgmt",
            artifact_type: "upload",
            entity_id: "ent-001",
            object_id: "obj-001",
            filename: "report.pdf",
        }
        .build(date(2026, 4, 6));

        assert!(k1.starts_with("tenants/tenant-aaa/"));
        assert!(k2.starts_with("tenants/tenant-bbb/"));
        assert_ne!(k1, k2, "different tenants must produce different keys");
    }

    #[test]
    fn key_structure_matches_adr018() {
        let key = BlobKeyBuilder {
            tenant_id: "t-123",
            service: "doc-mgmt",
            artifact_type: "rendered",
            entity_id: "e-456",
            object_id: "o-789",
            filename: "invoice.pdf",
        }
        .build(date(2026, 1, 5));

        assert_eq!(
            key,
            "tenants/t-123/doc-mgmt/rendered/e-456/2026/01/05/o-789-invoice.pdf"
        );
    }

    #[test]
    fn date_padding_two_digits() {
        let key = BlobKeyBuilder {
            tenant_id: "t",
            service: "s",
            artifact_type: "a",
            entity_id: "e",
            object_id: "o",
            filename: "f.txt",
        }
        .build(date(2026, 3, 7));

        assert!(
            key.contains("/2026/03/07/"),
            "month and day must be zero-padded"
        );
    }

    #[test]
    fn normalize_filename_lowercases_and_strips_unsafe() {
        assert_eq!(normalize_filename("My File (1).PDF"), "my_file__1_.pdf");
    }

    #[test]
    fn normalize_filename_allows_dash_dot_underscore() {
        assert_eq!(normalize_filename("my-file_v2.0.txt"), "my-file_v2.0.txt");
    }

    #[test]
    fn normalize_filename_no_leading_dot() {
        assert_eq!(normalize_filename(".hidden"), "_hidden");
    }

    #[test]
    fn normalize_filename_empty_becomes_placeholder() {
        assert_eq!(normalize_filename(""), "_");
    }

    #[test]
    fn normalize_filename_unicode_replaced() {
        assert_eq!(normalize_filename("résumé.pdf"), "r_sum_.pdf");
    }
}
