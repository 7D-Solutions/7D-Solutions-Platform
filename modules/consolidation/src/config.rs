use std::env;

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
        let database_url = env::var("DATABASE_URL").map_err(|_| {
            "DATABASE_URL is required. Example: postgres://consolidation_user:pass@localhost/consolidation_db"
                .to_string()
        })?;

        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        let gl_base_url =
            env::var("GL_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "8105".to_string())
            .parse()
            .map_err(|_| "PORT must be a valid u16".to_string())?;

        let env = env::var("ENV").unwrap_or_else(|_| "development".to_string());

        let cors_origins: Vec<String> = env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if env == "production" && cors_origins.iter().any(|o| o == "*") {
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
            env,
            cors_origins,
        })
    }
}
