use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
}

impl Config {
    /// Load from environment variables.
    ///
    /// Required: `DATABASE_URL` — must follow `consolidation_{app_id}_db` naming convention.
    /// Optional: `HOST` (default: 0.0.0.0), `PORT` (default: 8096).
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL").map_err(|_| {
            "DATABASE_URL is required. Example: postgres://consolidation_user:pass@localhost/consolidation_db"
                .to_string()
        })?;

        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "8096".to_string())
            .parse()
            .map_err(|_| "PORT must be a valid u16".to_string())?;

        Ok(Config {
            database_url,
            host,
            port,
        })
    }
}
