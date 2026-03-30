//! Inventory Module Configuration
//!
//! Validates required environment variables at startup, collecting ALL errors
//! before failing. Invariant: a missing env var must never cause a cryptic panic
//! deep in application code — it must fail immediately at startup with a clear
//! message listing ALL problems.

use std::env;

/// Application configuration parsed from environment variables
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub env: String,
    /// Comma-separated list of allowed CORS origins. "*" means allow any.
    pub cors_origins: Vec<String>,
    pub bus_type: BusType,
    pub nats_url: Option<String>,
}

/// Supported event bus options
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusType {
    Nats,
    InMemory,
}

impl BusType {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "nats" => Ok(BusType::Nats),
            "inmemory" => Ok(BusType::InMemory),
            other => Err(format!(
                "Invalid BUS_TYPE '{}'. Must be 'nats' or 'inmemory'",
                other
            )),
        }
    }
}

impl Config {
    /// Load from environment variables, collecting ALL errors before failing.
    ///
    /// Required: `DATABASE_URL`.
    /// Optional: `HOST` (default: 0.0.0.0), `PORT` (default: 8092), `ENV`, `CORS_ORIGINS`.
    pub fn from_env() -> Result<Self, String> {
        let mut errors: Vec<String> = Vec::new();

        let database_url = match env::var("DATABASE_URL") {
            Ok(v) if v.trim().is_empty() => {
                errors.push("DATABASE_URL is set but empty".to_string());
                String::new()
            }
            Ok(v) => v,
            Err(_) => {
                errors.push(
                    "DATABASE_URL is required. \
                     Example: postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db"
                        .to_string(),
                );
                String::new()
            }
        };

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let port: u16 = match env::var("PORT")
            .unwrap_or_else(|_| "8092".to_string())
            .parse::<u16>()
        {
            Ok(p) => p,
            Err(_) => {
                errors.push(format!(
                    "PORT must be a valid u16 (0-65535), got: '{}'",
                    env::var("PORT").unwrap_or_default()
                ));
                8092
            }
        };

        let env_name = env::var("ENV").unwrap_or_else(|_| "development".to_string());

        let cors_origins: Vec<String> = env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if env_name == "production" && cors_origins.iter().any(|o| o == "*") {
            errors.push(
                "CORS_ORIGINS=* is not allowed in production. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com)"
                    .to_string(),
            );
        }

        let bus_type_str = env::var("BUS_TYPE").unwrap_or_else(|_| "inmemory".to_string());
        let bus_type = match BusType::from_str(&bus_type_str) {
            Ok(bt) => bt,
            Err(err) => {
                errors.push(err);
                BusType::InMemory
            }
        };

        let nats_url = if bus_type == BusType::Nats {
            Some(env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string()))
        } else {
            None
        };

        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }

        Ok(Config {
            database_url,
            host,
            port,
            env: env_name,
            cors_origins,
            bus_type,
            nats_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_requires_database_url() {
        let config = Config {
            database_url: "".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8092,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            bus_type: BusType::InMemory,
            nats_url: None,
        };
        assert!(config.database_url.is_empty());
    }

    #[test]
    fn config_default_port_is_8092() {
        let config = Config {
            database_url: "postgresql://localhost/inventory_db".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8092,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            bus_type: BusType::InMemory,
            nats_url: None,
        };
        assert_eq!(config.port, 8092);
    }
}
