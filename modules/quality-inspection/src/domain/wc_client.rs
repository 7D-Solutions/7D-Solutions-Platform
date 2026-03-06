use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;
use workforce_competence_rs::domain::{models::AuthorizationQuery, service as wc_service};

use super::service::QiError;

const ARTIFACT_CODE: &str = "quality_inspection";

pub async fn verify_inspector_authorized(
    wc_pool: &PgPool,
    tenant_id: &str,
    inspector_id: Uuid,
) -> Result<(), QiError> {
    let query = AuthorizationQuery {
        tenant_id: tenant_id.to_string(),
        operator_id: inspector_id,
        artifact_code: ARTIFACT_CODE.to_string(),
        at_time: Utc::now(),
    };

    match wc_service::check_authorization(wc_pool, &query).await {
        Ok(result) => {
            if result.authorized {
                Ok(())
            } else {
                Err(QiError::Unauthorized(format!(
                    "Inspector {} is not authorized for quality inspection disposition",
                    inspector_id
                )))
            }
        }
        Err(e) => Err(QiError::ServiceUnavailable(format!(
            "Workforce-Competence authorization check failed (fail-closed): {}",
            e
        ))),
    }
}
