use chrono::{DateTime, Utc};
use platform_sdk::{parse_response, ClientError, PlatformClient, VerifiedClaims};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct TaxTenantConfigResponse {
    pub tenant_id: String,
    pub tax_calculation_source: String,
    pub provider_name: String,
    pub config_version: i64,
    pub updated_at: DateTime<Utc>,
    pub reconciliation_threshold_pct: Decimal,
}

#[derive(Debug, Serialize)]
pub struct PutTaxTenantConfigRequest {
    pub tax_calculation_source: String,
    pub provider_name: String,
}

pub struct TaxTenantConfigClient {
    client: PlatformClient,
}

impl TaxTenantConfigClient {
    pub fn new(client: PlatformClient) -> Self {
        Self { client }
    }

    pub async fn get(&self, claims: &VerifiedClaims) -> Result<TaxTenantConfigResponse, ClientError> {
        let resp = self
            .client
            .get("/api/ar/tax/tenant-config", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn put(
        &self,
        claims: &VerifiedClaims,
        body: &PutTaxTenantConfigRequest,
    ) -> Result<TaxTenantConfigResponse, ClientError> {
        let resp = self
            .client
            .put("/api/ar/tax/tenant-config", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use platform_sdk::VerifiedClaims;
    use security::claims::ActorType;
    use uuid::Uuid;

    /// Mint a full user JWT with admin role using JWT_PRIVATE_KEY_PEM from env.
    /// The PUT handler enforces tenant_admin/admin role; service JWTs carry empty
    /// roles, so we mint a user JWT that carries the role the server actually checks.
    fn mint_admin_jwt(tenant_id: Uuid, user_id: Uuid) -> String {
        let pem = std::env::var("JWT_PRIVATE_KEY_PEM")
            .expect("JWT_PRIVATE_KEY_PEM must be set for PUT test");
        let pem = pem.replace("\\n", "\n");
        let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes())
            .expect("invalid JWT_PRIVATE_KEY_PEM");
        let now = Utc::now();
        let exp = now + chrono::Duration::minutes(15);
        let claims = serde_json::json!({
            "sub": user_id.to_string(),
            "iss": "auth-rs",
            "aud": "7d-platform",
            "iat": now.timestamp(),
            "exp": exp.timestamp(),
            "jti": Uuid::new_v4().to_string(),
            "tenant_id": tenant_id.to_string(),
            "roles": ["admin"],
            "perms": ["ar.mutate", "ar.read"],
            "actor_type": "user",
            "ver": "1.0",
        });
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        jsonwebtoken::encode(&header, &claims, &encoding_key)
            .expect("failed to mint admin JWT")
    }

    #[tokio::test]
    async fn round_trip_tax_calculation_source() {
        dotenvy::dotenv().ok();

        let base_url = std::env::var("AR_SERVICE_URL")
            .unwrap_or_else(|_| "http://localhost:8086".to_string());
        let tenant_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();

        // PUT requires tenant_admin/admin role; service JWTs carry empty roles.
        // Mint a user JWT with admin role directly so the handler accepts it.
        let admin_jwt = mint_admin_jwt(tenant_id, user_id);
        let http = reqwest::Client::new();
        let put_url = format!("{base_url}/api/ar/tax/tenant-config");
        let pre_put = Utc::now();
        let put_body = serde_json::json!({
            "tax_calculation_source": "platform",
            "provider_name": "local",
        });
        let put_raw = http
            .put(&put_url)
            .bearer_auth(&admin_jwt)
            .header("x-tenant-id", tenant_id.to_string())
            .header("x-actor-id", user_id.to_string())
            .json(&put_body)
            .send()
            .await
            .expect("PUT request failed");
        assert_eq!(put_raw.status().as_u16(), 200, "PUT must return 200");
        let put_resp: TaxTenantConfigResponse =
            put_raw.json().await.expect("PUT response deserialization failed");
        assert_eq!(put_resp.tax_calculation_source, "platform");
        let _ = put_resp.reconciliation_threshold_pct;

        // GET uses the service JWT (no role check on GET) via TaxTenantConfigClient.
        let service_claims = VerifiedClaims {
            user_id,
            tenant_id,
            app_id: None,
            roles: vec![],
            perms: vec!["service.internal".to_string()],
            actor_type: ActorType::Service,
            issued_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            token_id: Uuid::new_v4(),
            version: "1.0".to_string(),
        };
        let client = TaxTenantConfigClient::new(PlatformClient::new(base_url));
        let get_resp = client.get(&service_claims).await.expect("GET failed");
        assert_eq!(get_resp.tax_calculation_source, "platform");
        assert!(get_resp.updated_at >= pre_put - chrono::Duration::seconds(5));
        assert!(get_resp.config_version > 0);
    }
}
