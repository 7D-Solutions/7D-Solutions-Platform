//! Maintenance Module Configuration
//!
//! Validates required environment variables at startup with clear error messages.
//! Invariant: Maintenance service never starts with missing/invalid configuration.

use std::env;

/// Event bus type
#[derive(Debug, Clone, PartialEq)]
pub enum BusType {
    Nats,
    InMemory,
}

impl BusType {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "nats" => Ok(BusType::Nats),
            "inmemory" => Ok(BusType::InMemory),
            _ => Err(format!(
                "Invalid BUS_TYPE '{}'. Must be 'nats' or 'inmemory'",
                s
            )),
        }
    }
}

/// Maintenance application configuration
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bus_type: BusType,
    pub nats_url: Option<String>,
    pub host: String,
    pub port: u16,
    pub env: String,
    /// Comma-separated list of allowed CORS origins. "*" means allow any.
    pub cors_origins: Vec<String>,
    /// Scheduler poll interval in seconds (default: 60)
    pub scheduler_interval_secs: u64,
}

impl Config {
    /// Load configuration from environment variables with strict validation.
    ///
    /// ## Required
    /// - `DATABASE_URL`: PostgreSQL connection string
    ///
    /// ## Optional (with defaults)
    /// - `BUS_TYPE`: 'nats' or 'inmemory' (default: 'inmemory')
    /// - `NATS_URL`: NATS server URL (required when BUS_TYPE=nats)
    /// - `HOST`: Bind host (default: '0.0.0.0')
    /// - `PORT`: HTTP port (default: '8101')
    /// - `MAINTENANCE_SCHED_INTERVAL_SECS`: Scheduler poll interval (default: '60')
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL").map_err(|_| {
            "DATABASE_URL is required but not set. \
             Example: postgresql://postgres:postgres@localhost:5432/maintenance_db"
                .to_string()
        })?;

        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        let bus_type_str = env::var("BUS_TYPE").unwrap_or_else(|_| "inmemory".to_string());
        let bus_type = BusType::from_str(&bus_type_str)?;

        let nats_url = match bus_type {
            BusType::Nats => {
                let url =
                    env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
                if url.trim().is_empty() {
                    return Err("NATS_URL cannot be empty when BUS_TYPE=nats".to_string());
                }
                Some(url)
            }
            BusType::InMemory => None,
        };

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "8101".to_string())
            .parse()
            .map_err(|_| "PORT must be a valid u16 (0-65535)".to_string())?;

        let scheduler_interval_secs: u64 = env::var("MAINTENANCE_SCHED_INTERVAL_SECS")
            .unwrap_or_else(|_| "60".to_string())
            .parse()
            .map_err(|_| "MAINTENANCE_SCHED_INTERVAL_SECS must be a valid u64".to_string())?;

        let env = env::var("ENV").unwrap_or_else(|_| "development".to_string());

        let cors_origins: Vec<String> = env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Config {
            database_url,
            bus_type,
            nats_url,
            host,
            port,
            env,
            cors_origins,
            scheduler_interval_secs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_type_from_str_valid() {
        assert_eq!(BusType::from_str("nats").unwrap(), BusType::Nats);
        assert_eq!(BusType::from_str("inmemory").unwrap(), BusType::InMemory);
        assert_eq!(BusType::from_str("NATS").unwrap(), BusType::Nats);
        assert_eq!(BusType::from_str("InMemory").unwrap(), BusType::InMemory);
    }

    #[test]
    fn bus_type_from_str_invalid() {
        let err = BusType::from_str("kafka").unwrap_err();
        assert!(err.contains("Invalid BUS_TYPE"));
    }
}
