//! Subscriptions Module Configuration
//!
//! Uses ConfigValidator to report ALL missing/invalid env vars at once.
//! Invariant: Subscriptions service never starts with missing/invalid configuration.

use config_validator::ConfigValidator;

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

#[derive(Debug, Clone)]
pub struct Config {
    pub bus_type: BusType,
    pub database_url: String,
    pub nats_url: Option<String>,
    pub env: String,
    /// Comma-separated list of allowed CORS origins. "*" means allow any.
    pub cors_origins: Vec<String>,
}

impl Config {
    /// Load configuration from environment variables with structured validation.
    ///
    /// All errors are collected and reported at once via ConfigValidator.
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("subscriptions");

        let database_url = v.require("DATABASE_URL").unwrap_or_default();
        let env_name = v.optional("ENV").or_default("development");

        let cors_raw = v.optional("CORS_ORIGINS").or_default("*");
        let cors_origins: Vec<String> = cors_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let bus_type_str = v.optional("BUS_TYPE").or_default("inmemory");
        let bus_type = BusType::from_str(&bus_type_str).unwrap_or(BusType::InMemory);

        let nats_url = v.require_when(
            "NATS_URL",
            || bus_type == BusType::Nats,
            "required when BUS_TYPE=nats",
        );

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
            bus_type,
            database_url,
            nats_url,
            env: env_name,
            cors_origins,
        })
    }

    /// Validate configuration contract
    ///
    /// Ensures all required fields are set and valid.
    /// Called automatically during from_env(), but exposed for testing.
    pub fn validate(&self) -> Result<(), String> {
        if self.database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        if self.bus_type == BusType::Nats && self.nats_url.is_none() {
            return Err("NATS_URL is required when BUS_TYPE=nats".to_string());
        }

        if let Some(ref url) = self.nats_url {
            if url.trim().is_empty() {
                return Err("NATS_URL cannot be empty".to_string());
            }
        }

        Ok(())
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
        assert_eq!(BusType::from_str("InMemory").unwrap(), BusType::InMemory);

        let err = BusType::from_str("invalid").unwrap_err();
        assert!(err.contains("Invalid BUS_TYPE"));
        assert!(err.contains("invalid"));
    }

    #[test]
    fn test_validate_empty_database_url() {
        let config = Config {
            database_url: "".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
        };

        let err = config.validate().unwrap_err();
        assert!(err.contains("DATABASE_URL cannot be empty"));
    }

    #[test]
    fn test_validate_nats_requires_url() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::Nats,
            nats_url: None,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
        };

        let err = config.validate().unwrap_err();
        assert!(err.contains("NATS_URL is required when BUS_TYPE=nats"));
    }

    #[test]
    fn test_validate_success() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
        };

        assert!(config.validate().is_ok());

        let config_nats = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::Nats,
            nats_url: Some("nats://localhost:4222".to_string()),
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
        };

        assert!(config_nats.validate().is_ok());
    }
}
