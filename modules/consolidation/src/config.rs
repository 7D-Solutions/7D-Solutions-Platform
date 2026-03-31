use config_validator::ConfigValidator;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub gl_base_url: String,
    pub env: String,
    /// Comma-separated list of allowed CORS origins. "*" means allow any.
    pub cors_origins: Vec<String>,
}

impl Config {
    /// Load from environment variables.
    ///
    /// Required: `DATABASE_URL` — must follow `consolidation_{app_id}_db` naming convention.
    /// Optional: `HOST` (default: 0.0.0.0), `PORT` (default: 8105).
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("consolidation");

        let database_url = v.require("DATABASE_URL").unwrap_or_default();

        let gl_base_url = v
            .optional("GL_BASE_URL")
            .or_default("http://localhost:8080");

        let host = v.optional("HOST").or_default("0.0.0.0");
        let port: u16 = v.optional_parse::<u16>("PORT").unwrap_or(8105);

        let env_val = v.optional("ENV").or_default("development");

        let cors_origins: Vec<String> = v
            .optional("CORS_ORIGINS")
            .or_default("*")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        v.finish().map_err(|e| e.to_string())?;

        if env_val == "production" && cors_origins.iter().any(|o| o == "*") {
            return Err("CORS_ORIGINS=* is not allowed in production. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com)"
                .to_string());
        }
        Ok(Config {
            database_url,
            host,
            port,
            gl_base_url,
            env: env_val,
            cors_origins,
        })
    }
}
