use chrono::Utc;
use platform_sdk::PlatformClient;
use serde::Deserialize;
use uuid::Uuid;

use super::service::QiError;

const ARTIFACT_CODE: &str = "quality_inspection";

#[derive(Debug, Deserialize)]
struct AuthorizationResult {
    pub authorized: bool,
}

pub async fn verify_inspector_authorized(
    wc_client: &PlatformClient,
    tenant_id: &str,
    inspector_id: Uuid,
) -> Result<(), QiError> {
    let now = Utc::now().to_rfc3339();
    let path = format!(
        "/api/workforce-competence/authorization?operator_id={}&artifact_code={}&at_time={}",
        inspector_id, ARTIFACT_CODE, now
    );

    let tenant_uuid = uuid::Uuid::parse_str(tenant_id).map_err(|e| {
        QiError::Validation(format!("Invalid tenant_id '{}': {}", tenant_id, e))
    })?;
    let claims = PlatformClient::service_claims(tenant_uuid);

    let resp = wc_client.get(&path, &claims).await.map_err(|e| {
        QiError::ServiceUnavailable(format!(
            "Workforce-Competence authorization check failed (fail-closed): {}",
            e
        ))
    })?;

    if resp.status() == reqwest::StatusCode::OK {
        let result: AuthorizationResult = resp.json().await.map_err(|e| {
            QiError::ServiceUnavailable(format!(
                "Workforce-Competence returned invalid response: {}",
                e
            ))
        })?;

        if result.authorized {
            Ok(())
        } else {
            Err(QiError::Unauthorized(format!(
                "Inspector {} is not authorized for quality inspection disposition",
                inspector_id
            )))
        }
    } else {
        Err(QiError::ServiceUnavailable(format!(
            "Workforce-Competence authorization check failed (fail-closed): HTTP {}",
            resp.status()
        )))
    }
}
