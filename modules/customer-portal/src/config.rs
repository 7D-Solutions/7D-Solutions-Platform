use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub cors_origins: Vec<String>,
    pub portal_jwt_private_key: String,
    pub portal_jwt_public_key: String,
    pub access_token_ttl_minutes: i64,
    pub refresh_token_ttl_days: i64,
    pub doc_mgmt_base_url: String,
    pub doc_mgmt_bearer_token: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required".to_string())?;
        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(8110);
        let cors_origins = env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>();

        let portal_jwt_private_key = env::var("PORTAL_JWT_PRIVATE_KEY")
            .map_err(|_| "PORTAL_JWT_PRIVATE_KEY is required".to_string())?;
        let portal_jwt_public_key = env::var("PORTAL_JWT_PUBLIC_KEY")
            .map_err(|_| "PORTAL_JWT_PUBLIC_KEY is required".to_string())?;

        let access_token_ttl_minutes = env::var("ACCESS_TOKEN_TTL_MINUTES")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(15);

        let refresh_token_ttl_days = env::var("REFRESH_TOKEN_TTL_DAYS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(7);
        let doc_mgmt_base_url =
            env::var("DOC_MGMT_BASE_URL").unwrap_or_else(|_| "http://localhost:8095".to_string());
        let doc_mgmt_bearer_token = env::var("DOC_MGMT_BEARER_TOKEN").ok();

        Ok(Self {
            database_url,
            host,
            port,
            cors_origins,
            portal_jwt_private_key,
            portal_jwt_public_key,
            access_token_ttl_minutes,
            refresh_token_ttl_days,
            doc_mgmt_base_url,
            doc_mgmt_bearer_token,
        })
    }
}
