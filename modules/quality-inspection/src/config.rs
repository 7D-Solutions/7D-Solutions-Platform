use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub env: String,
    pub cors_origins: Vec<String>,
    pub nats_url: String,
    pub bus_type: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL").map_err(|_| {
            "DATABASE_URL is required but not set. \
             Example: postgresql://quality_inspection_user:quality_inspection_pass@localhost:5459/quality_inspection_db"
                .to_string()
        })?;

        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "8106".to_string())
            .parse()
            .map_err(|_| {
                format!(
                    "PORT must be a valid u16 (0-65535), got: '{}'",
                    env::var("PORT").unwrap_or_default()
                )
            })?;

        let env_name = env::var("ENV").unwrap_or_else(|_| "development".to_string());

        let cors_origins: Vec<String> = env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let nats_url =
            env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
        let bus_type = env::var("BUS_TYPE").unwrap_or_else(|_| "nats".to_string());

        Ok(Config {
            database_url,
            host,
            port,
            env: env_name,
            cors_origins,
            nats_url,
            bus_type,
        })
    }
}
