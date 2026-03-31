use config_validator::ConfigValidator;

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
        let mut v = ConfigValidator::new("customer-portal");

        let database_url = v.require("DATABASE_URL").unwrap_or_default();
        let host = v.optional("HOST").or_default("0.0.0.0");
        let port = v.optional_parse::<u16>("PORT").unwrap_or(8110);

        let cors_raw = v.optional("CORS_ORIGINS").or_default("*");
        let cors_origins: Vec<String> = cors_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let portal_jwt_private_key = v.require("PORTAL_JWT_PRIVATE_KEY").unwrap_or_default();
        let portal_jwt_public_key = v.require("PORTAL_JWT_PUBLIC_KEY").unwrap_or_default();

        let access_token_ttl_minutes = v.optional_parse::<i64>("ACCESS_TOKEN_TTL_MINUTES").unwrap_or(15);
        let refresh_token_ttl_days = v.optional_parse::<i64>("REFRESH_TOKEN_TTL_DAYS").unwrap_or(7);

        let doc_mgmt_base_url = v.optional("DOC_MGMT_BASE_URL").or_default("http://localhost:8095");
        let doc_mgmt_bearer_token = v.optional("DOC_MGMT_BEARER_TOKEN").present().map(String::from);

        let env_name = v.optional("ENV").or_default("development");

        if env_name == "production" && cors_origins.iter().any(|o| o == "*") {
            return Err(
                "CORS_ORIGINS=* is not allowed in production. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com)"
                    .to_string(),
            );
        }

        v.finish().map_err(|e| e.to_string())?;

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
