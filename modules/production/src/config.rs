use config_validator::ConfigValidator;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub env: String,
    pub cors_origins: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("production");

        let database_url = v.require("DATABASE_URL").unwrap_or_default();
        let host = v.optional("HOST").or_default("0.0.0.0");
        let port = v.optional_parse::<u16>("PORT").unwrap_or(8108);
        let env_name = v.optional("ENV").or_default("development");

        let cors_origins: Vec<String> = v
            .optional("CORS_ORIGINS")
            .or_default("*")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if env_name == "production" && cors_origins.iter().any(|o| o == "*") {
            return Err("CORS_ORIGINS=* is not allowed in production. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com)"
                .to_string());
        }

        v.finish().map_err(|e| e.to_string())?;

        Ok(Config {
            database_url,
            host,
            port,
            env: env_name,
            cors_origins,
        })
    }
}
