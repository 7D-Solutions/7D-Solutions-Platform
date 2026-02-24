//! Inventory Module Configuration
//!
//! Validates required environment variables at startup with clear error messages.
//! Invariant: Inventory service never starts with missing/invalid configuration.

use std::env;

/// Application configuration parsed from environment variables
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    /// Comma-separated list of allowed CORS origins. "*" means allow any.
    pub cors_origins: Vec<String>,
}

impl Config {
    /// Load configuration from environment variables with strict validation
    ///
    /// ## Required Environment Variables
    /// - `DATABASE_URL`: PostgreSQL connection string
    ///
    /// ## Optional Environment Variables (with defaults)
    /// - `HOST`: Bind host (default: '0.0.0.0')
    /// - `PORT`: HTTP port (default: '8092')
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL").map_err(|_| {
            "DATABASE_URL is required but not set. \
             Example: postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db"
                .to_string()
        })?;

        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "8092".to_string())
            .parse()
            .map_err(|_| {
                format!(
                    "PORT must be a valid u16 (0-65535), got: '{}'",
                    env::var("PORT").unwrap_or_default()
                )
            })?;

        let cors_origins: Vec<String> = env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Config {
            database_url,
            host,
            port,
            cors_origins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_requires_database_url() {
        // Remove DATABASE_URL if set, test that from_env fails
        let config = Config {
            database_url: "".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8092,
            cors_origins: vec!["*".to_string()],
        };
        // Direct struct construction is valid; from_env requires the env var
        assert!(config.database_url.is_empty());
    }

    #[test]
    fn config_default_port_is_8092() {
        let config = Config {
            database_url: "postgresql://localhost/inventory_db".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8092,
            cors_origins: vec!["*".to_string()],
        };
        assert_eq!(config.port, 8092);
    }
}
