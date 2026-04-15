/// Service catalog handler
///
/// GET /api/service-catalog — returns module_code → base_url mappings
use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::models::ErrorBody;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ServiceCatalogResponse {
    pub services: BTreeMap<String, String>,
}

pub async fn service_catalog(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ServiceCatalogResponse>, (StatusCode, Json<ErrorBody>)> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT module_code, base_url FROM cp_service_catalog ORDER BY module_code")
            .fetch_all(&state.pool)
            .await
            .map_err(|e| {
                tracing::error!("Database error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody {
                        error: "Internal database error".to_string(),
                    }),
                )
            })?;

    let services: BTreeMap<String, String> = rows.into_iter().collect();
    Ok(Json(ServiceCatalogResponse { services }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_response_serialises_to_expected_shape() {
        let mut services = BTreeMap::new();
        services.insert("ar".to_string(), "http://7d-ar:8086".to_string());
        services.insert("gl".to_string(), "http://7d-gl:8090".to_string());

        let resp = ServiceCatalogResponse { services };
        let v = serde_json::to_value(&resp).expect("serialises");
        assert_eq!(v["services"]["ar"], "http://7d-ar:8086");
        assert_eq!(v["services"]["gl"], "http://7d-gl:8090");
    }

    #[tokio::test]
    async fn catalog_query_returns_rows_from_db() {
        let db_url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
                .to_string()
        });
        let pool = match sqlx::PgPool::connect_lazy(&db_url) {
            Ok(p) => p,
            Err(_) => return,
        };

        // Query the table — should have seed data from migration
        let rows: Result<Vec<(String, String)>, _> = sqlx::query_as(
            "SELECT module_code, base_url FROM cp_service_catalog ORDER BY module_code",
        )
        .fetch_all(&pool)
        .await;

        match rows {
            Ok(r) => {
                assert!(
                    !r.is_empty(),
                    "seed data should populate cp_service_catalog"
                );
                let map: BTreeMap<String, String> = r.into_iter().collect();
                assert_eq!(map.get("ar").map(|s| s.as_str()), Some("http://7d-ar:8086"));
            }
            Err(_) => {
                // DB not available — skip
            }
        }
    }
}
