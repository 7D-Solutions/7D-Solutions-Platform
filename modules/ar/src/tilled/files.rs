use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct FileObject {
    pub id: String,
    #[serde(rename = "type", default)]
    pub file_type: Option<String>,
    pub purpose: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub size: Option<i64>,
    #[serde(default)]
    pub created_at: Option<String>,
}

impl TilledClient {
    /// Upload a file for dispute evidence via multipart/form-data.
    pub async fn upload_file(
        &self,
        file_bytes: Vec<u8>,
        filename: &str,
        mime_type: &str,
        purpose: &str,
    ) -> Result<FileObject, TilledError> {
        let url = format!("{}/v1/files", self.config.base_path);
        let part = Part::bytes(file_bytes)
            .file_name(filename.to_string())
            .mime_str(mime_type)
            .map_err(|e| TilledError::HttpError(e.to_string()))?;
        let form = Form::new()
            .text("purpose", purpose.to_string())
            .part("file", part);

        let response = self
            .http_client
            .post(&url)
            .headers(self.build_auth_headers()?)
            .multipart(form)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        self.handle_response(response).await
    }

    /// List files with optional filters.
    pub async fn list_files(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<FileObject>, TilledError> {
        self.get("/v1/files", filters).await
    }

    /// Get file metadata by ID.
    pub async fn get_file(&self, file_id: &str) -> Result<FileObject, TilledError> {
        let path = format!("/v1/files/{file_id}");
        self.get(&path, None).await
    }

    /// Get raw file bytes by file ID.
    pub async fn get_file_contents(&self, file_id: &str) -> Result<Vec<u8>, TilledError> {
        let path = format!("/v1/files/{file_id}/contents");
        let url = format!("{}{}", self.config.base_path, path);
        let response = self
            .http_client
            .get(&url)
            .headers(self.build_auth_headers()?)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        if response.status().is_success() {
            let bytes = response
                .bytes()
                .await
                .map_err(|e| TilledError::HttpError(e.to_string()))?;
            Ok(bytes.to_vec())
        } else {
            let status_code = response.status().as_u16();
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            Err(TilledError::ApiError {
                status_code,
                message,
            })
        }
    }

    /// Delete file by ID.
    /// Tilled may return 204 with empty body for successful deletes.
    pub async fn delete_file(&self, file_id: &str) -> Result<(), TilledError> {
        let path = format!("/v1/files/{file_id}");
        let url = format!("{}{}", self.config.base_path, path);
        let response = self
            .http_client
            .delete(&url)
            .headers(self.build_auth_headers()?)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status_code = response.status().as_u16();
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            Err(TilledError::ApiError {
                status_code,
                message,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FileObject;

    #[test]
    fn file_object_deserializes_type_alias() {
        let value = serde_json::json!({
            "id": "file_123",
            "type": "png",
            "purpose": "dispute_evidence",
            "size": 69
        });
        let file: FileObject = serde_json::from_value(value).unwrap();
        assert_eq!(file.id, "file_123");
        assert_eq!(file.file_type.as_deref(), Some("png"));
        assert_eq!(file.purpose, "dispute_evidence");
        assert_eq!(file.size, Some(69));
    }
}
