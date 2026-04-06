//! `[blob]` — opt-in blob storage configuration for a module.
//!
//! Modules that need blob storage declare this section in `module.toml`.
//! The bucket name comes from the manifest; credentials and endpoint come
//! from environment variables.
//!
//! ```toml
//! [blob]
//! bucket = "platform-docs"
//! # BLOB_REGION, BLOB_ACCESS_KEY_ID, BLOB_SECRET_ACCESS_KEY from env
//! # BLOB_ENDPOINT optional (for MinIO / Cloudflare R2)
//! ```

use std::collections::BTreeMap;

use serde::Deserialize;

/// `[blob]` section of `module.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct BlobSection {
    /// Target bucket name.
    pub bucket: String,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}
