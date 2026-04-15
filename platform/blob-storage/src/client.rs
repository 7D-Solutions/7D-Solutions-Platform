//! S3-compatible blob storage client with tenant-scoped operations per ADR-018.

use std::time::Duration;

use aws_config::BehaviorVersion;
use aws_sdk_s3::{
    config::{Credentials, Region},
    presigning::PresigningConfig,
    Client,
};
use tracing::{debug, info};

use crate::BlobError;

/// Runtime configuration for the blob storage client.
///
/// Load from environment with [`BlobStorageConfig::from_env`] or construct directly
/// for testing.
#[derive(Debug, Clone)]
pub struct BlobStorageConfig {
    /// S3-compatible provider identifier (informational; always "s3" in practice).
    pub provider: String,
    /// AWS region or equivalent (e.g. `"us-east-1"`, `"auto"` for R2).
    pub region: String,
    /// Optional custom endpoint URL — required for MinIO, Cloudflare R2, etc.
    pub endpoint: Option<String>,
    /// Target bucket name.
    pub bucket: String,
    /// Static access key ID (dev/break-glass only in production).
    pub access_key_id: String,
    /// Static secret access key (dev/break-glass only in production).
    pub secret_access_key: String,
    /// Default presigned URL lifetime in seconds (ADR-018 default: 900, max: 3600).
    pub presign_ttl_seconds: u64,
    /// Maximum upload size in bytes (ADR-018 default: 26 214 400 = 25 MiB).
    pub max_upload_bytes: u64,
}

impl BlobStorageConfig {
    /// Load configuration from environment variables per ADR-018.
    ///
    /// Required: `BLOB_ACCESS_KEY_ID`, `BLOB_SECRET_ACCESS_KEY`
    /// Bucket:   `BLOB_BUCKET_DOCS` (preferred) or `BLOB_BUCKET` (alias)
    /// Region:   `BLOB_REGION` (default "us-east-1"; MinIO ignores this)
    /// Optional: `BLOB_PROVIDER` (default "s3"), `BLOB_ENDPOINT`,
    ///           `BLOB_PRESIGN_TTL_SECONDS` (default 900),
    ///           `BLOB_MAX_UPLOAD_BYTES` (default 26214400)
    pub fn from_env() -> Result<Self, BlobError> {
        let require = |name: &str| {
            std::env::var(name)
                .map_err(|_| BlobError::Config(format!("{name} environment variable is required")))
        };

        // Accept BLOB_BUCKET_DOCS (canonical) or BLOB_BUCKET (compose alias).
        let bucket = std::env::var("BLOB_BUCKET_DOCS")
            .or_else(|_| std::env::var("BLOB_BUCKET"))
            .map_err(|_| {
                BlobError::Config(
                    "BLOB_BUCKET_DOCS (or BLOB_BUCKET) environment variable is required".into(),
                )
            })?;

        Ok(Self {
            provider: std::env::var("BLOB_PROVIDER").unwrap_or_else(|_| "s3".to_string()),
            // Region is optional for MinIO/R2; default matches AWS SDK expectations.
            region: std::env::var("BLOB_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
            endpoint: std::env::var("BLOB_ENDPOINT").ok(),
            bucket,
            access_key_id: require("BLOB_ACCESS_KEY_ID")?,
            secret_access_key: require("BLOB_SECRET_ACCESS_KEY")?,
            presign_ttl_seconds: std::env::var("BLOB_PRESIGN_TTL_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(900),
            max_upload_bytes: std::env::var("BLOB_MAX_UPLOAD_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(26_214_400),
        })
    }
}

/// Platform blob storage client.
///
/// Wraps the AWS S3 SDK client with tenant-aware operations. Construct with
/// [`BlobStorageClient::new`], then call [`BlobStorageClient::ensure_bucket_exists`]
/// at service startup.
pub struct BlobStorageClient {
    inner: Client,
    pub config: BlobStorageConfig,
}

impl BlobStorageClient {
    /// Create a new client from the given configuration.
    pub async fn new(config: BlobStorageConfig) -> Result<Self, BlobError> {
        let credentials = Credentials::new(
            &config.access_key_id,
            &config.secret_access_key,
            None,
            None,
            "blob-storage-static",
        );

        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(config.region.clone()))
            .credentials_provider(credentials)
            .load()
            .await;

        let mut s3_builder = aws_sdk_s3::config::Builder::from(&sdk_config);

        if let Some(ep) = &config.endpoint {
            // Path-style addressing required for MinIO and other non-AWS providers.
            s3_builder = s3_builder.endpoint_url(ep.clone()).force_path_style(true);
        }

        let inner = Client::from_conf(s3_builder.build());

        Ok(Self { inner, config })
    }

    /// Issue a presigned PUT URL for direct client upload.
    ///
    /// The URL is constrained to the exact `key` and `content_type`. The caller
    /// is responsible for ensuring the key was produced by [`BlobKeyBuilder`] and
    /// that MIME + size validation has already passed.
    ///
    /// [`BlobKeyBuilder`]: crate::BlobKeyBuilder
    pub async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<String, BlobError> {
        let ttl = ttl_seconds.unwrap_or(self.config.presign_ttl_seconds);
        let presigning = PresigningConfig::expires_in(Duration::from_secs(ttl))
            .map_err(|e| BlobError::Config(e.to_string()))?;

        let req = self
            .inner
            .put_object()
            .bucket(&self.config.bucket)
            .key(key)
            .content_type(content_type)
            .presigned(presigning)
            .await
            .map_err(|e| BlobError::S3(e.to_string()))?;

        debug!(key, ttl, "issued presigned PUT");
        Ok(req.uri().to_string())
    }

    /// Issue a presigned GET URL for short-lived download.
    pub async fn presign_get(
        &self,
        key: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<String, BlobError> {
        let ttl = ttl_seconds.unwrap_or(self.config.presign_ttl_seconds);
        let presigning = PresigningConfig::expires_in(Duration::from_secs(ttl))
            .map_err(|e| BlobError::Config(e.to_string()))?;

        let req = self
            .inner
            .get_object()
            .bucket(&self.config.bucket)
            .key(key)
            .presigned(presigning)
            .await
            .map_err(|e| BlobError::S3(e.to_string()))?;

        debug!(key, ttl, "issued presigned GET");
        Ok(req.uri().to_string())
    }

    /// Ensure the configured bucket exists, creating it if necessary.
    ///
    /// Intended for use in service startup (e.g. alongside database migration).
    /// Safe to call in dev against MinIO with path-style addressing.
    pub async fn ensure_bucket_exists(&self) -> Result<(), BlobError> {
        match self
            .inner
            .head_bucket()
            .bucket(&self.config.bucket)
            .send()
            .await
        {
            Ok(_) => {
                debug!(bucket = %self.config.bucket, "bucket already exists");
                return Ok(());
            }
            Err(sdk_err) => {
                // Distinguish "not found" (create it) from other errors (fail fast).
                let raw = sdk_err.to_string();
                // AWS and MinIO both return a 404-class error for missing buckets.
                // The SDK error message contains "404" or "NoSuchBucket".
                if !raw.contains("404") && !raw.contains("NoSuchBucket") {
                    return Err(BlobError::S3(raw));
                }
            }
        }

        info!(bucket = %self.config.bucket, "creating bucket");

        match self
            .inner
            .create_bucket()
            .bucket(&self.config.bucket)
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(sdk_err) => {
                let raw = sdk_err.to_string();
                // Race condition: another replica created the bucket first.
                if raw.contains("BucketAlreadyExists") || raw.contains("BucketAlreadyOwnedByYou") {
                    Ok(())
                } else {
                    Err(BlobError::S3(raw))
                }
            }
        }
    }
}
