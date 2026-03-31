//! PDF Editor Module Configuration
//!
//! Validates required environment variables at startup with clear error messages.
//! Invariant: PDF Editor service never starts with missing/invalid configuration.

use config_validator::ConfigValidator;

/// Bus type enumeration
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

/// PDF Editor application configuration
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bus_type: BusType,
    pub nats_url: Option<String>,
    pub host: String,
    pub port: u16,
    /// Comma-separated list of allowed CORS origins. "*" means allow any.
    pub cors_origins: Vec<String>,
    /// Runtime environment (e.g. "development", "staging", "production").
    pub env: String,
}

impl Config {
    /// Load configuration from environment variables with strict validation
    ///
    /// ## Required Environment Variables
    /// - `DATABASE_URL`: PostgreSQL connection string
    ///
    /// ## Optional Environment Variables (with defaults)
    /// - `BUS_TYPE`: 'nats' or 'inmemory' (default: 'inmemory')
    /// - `NATS_URL`: NATS server URL (required if BUS_TYPE=nats)
    /// - `HOST`: Bind host (default: '0.0.0.0')
    /// - `PORT`: HTTP port (default: '8102')
    /// - `CORS_ORIGINS`: Comma-separated allowed origins (omit to deny all cross-origin requests)
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("pdf-editor");

        let database_url = v.require("DATABASE_URL").unwrap_or_default();

        let bus_type_str = v.optional("BUS_TYPE").or_default("inmemory");
        let bus_type = BusType::from_str(&bus_type_str)?;

        let nats_url = match bus_type {
            BusType::Nats => {
                let url = v.optional("NATS_URL").or_default("nats://localhost:4222");
                if url.trim().is_empty() {
                    return Err("NATS_URL cannot be empty when BUS_TYPE=nats".to_string());
                }
                Some(url)
            }
            BusType::InMemory => None,
        };

        let host = v.optional("HOST").or_default("0.0.0.0");
        let port = v.optional_parse::<u16>("PORT").unwrap_or(8102);

        let cors_origins: Vec<String> = v
            .optional("CORS_ORIGINS")
            .or_default("")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if cors_origins.iter().any(|o| o == "*") {
            return Err(
                "CORS_ORIGINS=* (wildcard) is not allowed. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com). \
                 Omit CORS_ORIGINS to deny all cross-origin requests."
                    .to_string(),
            );
        }

        let env_name = v
            .optional("ENV")
            .or_default("development")
            .to_lowercase();

        v.finish().map_err(|e| e.to_string())?;

        Ok(Config {
            database_url,
            bus_type,
            nats_url,
            host,
            port,
            cors_origins,
            env: env_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bus_type_from_str() {
        assert_eq!(BusType::from_str("nats").unwrap(), BusType::Nats);
        assert_eq!(BusType::from_str("inmemory").unwrap(), BusType::InMemory);
        assert_eq!(BusType::from_str("NATS").unwrap(), BusType::Nats);
        assert!(BusType::from_str("invalid").is_err());
    }
}
