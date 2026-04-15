//! MIME type and upload size validation per ADR-018.

use crate::BlobError;

/// MIME types explicitly permitted by ADR-018.
pub const ALLOWED_MIME_TYPES: &[&str] = &[
    "application/pdf",
    "image/png",
    "image/jpeg",
    "text/plain",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
];

/// MIME types explicitly blocked by ADR-018.
const BLOCKED_MIME_TYPES: &[&str] = &["application/x-msdownload", "application/javascript"];

/// Validate that `mime_type` is on the ADR-018 allowlist.
///
/// Returns `BlobError::MimeTypeNotAllowed` for anything not on the allowlist,
/// including types on the explicit blocklist and unknown binary types.
pub fn validate_mime_type(mime_type: &str) -> Result<(), BlobError> {
    let lower = mime_type.to_lowercase();
    let lower = lower.trim();

    if BLOCKED_MIME_TYPES.iter().any(|b| *b == lower) {
        return Err(BlobError::MimeTypeNotAllowed(mime_type.to_string()));
    }

    if ALLOWED_MIME_TYPES.iter().any(|a| *a == lower) {
        return Ok(());
    }

    Err(BlobError::MimeTypeNotAllowed(mime_type.to_string()))
}

/// Validate that `size_bytes` does not exceed `max_bytes`.
pub fn validate_size(size_bytes: u64, max_bytes: u64) -> Result<(), BlobError> {
    if size_bytes > max_bytes {
        return Err(BlobError::FileTooLarge {
            size_bytes,
            max_bytes,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_mime_types_pass() {
        for mime in ALLOWED_MIME_TYPES {
            validate_mime_type(mime)
                .unwrap_or_else(|e| panic!("expected allowed MIME {mime} to pass, got: {e}"));
        }
    }

    #[test]
    fn blocked_mime_types_fail() {
        assert!(validate_mime_type("application/x-msdownload").is_err());
        assert!(validate_mime_type("application/javascript").is_err());
    }

    #[test]
    fn unknown_binary_blocked() {
        assert!(validate_mime_type("application/octet-stream").is_err());
        assert!(validate_mime_type("application/zip").is_err());
    }

    #[test]
    fn case_insensitive_matching() {
        assert!(validate_mime_type("Application/PDF").is_ok());
        assert!(validate_mime_type("IMAGE/PNG").is_ok());
    }

    #[test]
    fn size_within_limit_passes() {
        validate_size(1024, 26_214_400).expect("test");
    }

    #[test]
    fn size_at_limit_passes() {
        validate_size(26_214_400, 26_214_400).expect("test");
    }

    #[test]
    fn size_over_limit_fails() {
        let err = validate_size(26_214_401, 26_214_400).unwrap_err();
        assert!(matches!(err, BlobError::FileTooLarge { .. }));
    }
}
