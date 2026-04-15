//! AR Module Configuration
//!
//! Uses ConfigValidator to report ALL missing/invalid env vars at once.
//! Invariant: AR service never starts with missing/invalid configuration.
//!
//! PRESERVED: TILLED_WEBHOOK_SECRET_TRASHTECH → TILLED_WEBHOOK_SECRET fallback order.
//! PRESERVED: PARTY_MASTER_URL default.

use config_validator::ConfigValidator;
use std::env;

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

/// AR application configuration
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bus_type: BusType,
    pub nats_url: Option<String>,
    pub host: String,
    pub port: u16,
    pub party_master_url: String,
    pub webhook_secret: String,
    pub env: String,
    /// Comma-separated list of allowed CORS origins. "*" means allow any.
    pub cors_origins: Vec<String>,
}

impl Config {
    /// Load configuration from environment variables with structured validation.
    ///
    /// All errors are collected and reported at once via ConfigValidator.
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("ar");

        let database_url = v.require("DATABASE_URL").unwrap_or_default();
        let host = v.optional("HOST").or_default("0.0.0.0");
        let port = v.optional_parse::<u16>("PORT").unwrap_or(8086);
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

        let party_master_url = v
            .optional("PARTY_MASTER_URL")
            .or_default("http://7d-party:8098");

        // PRESERVED: TILLED_WEBHOOK_SECRET_TRASHTECH → TILLED_WEBHOOK_SECRET fallback order
        // ConfigValidator doesn't support multi-key fallback, so we resolve manually
        // and feed the result through require() by pre-setting the env var.
        let webhook_secret = env::var("TILLED_WEBHOOK_SECRET_TRASHTECH")
            .or_else(|_| env::var("TILLED_WEBHOOK_SECRET"))
            .unwrap_or_default();
        if webhook_secret.trim().is_empty() {
            // Force a require() failure so it shows up in the multi-error report
            let _ = v.require("TILLED_WEBHOOK_SECRET");
        }

        if env_name == "production" && cors_origins.iter().any(|o| o == "*") {
            return Err("CORS_ORIGINS=* is not allowed in production. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com)"
                .to_string());
        }

        v.finish().map_err(|e| e.to_string())?;

        Ok(Config {
            database_url,
            bus_type,
            nats_url,
            host,
            port,
            party_master_url,
            webhook_secret,
            env: env_name,
            cors_origins,
        })
    }

    /// Validate configuration contract
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

        if self.webhook_secret.trim().is_empty() {
            return Err("TILLED_WEBHOOK_SECRET cannot be empty".to_string());
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
            host: "0.0.0.0".to_string(),
            port: 8086,
            party_master_url: "http://7d-party:8098".to_string(),
            webhook_secret: "whsec_test".to_string(),
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
            host: "0.0.0.0".to_string(),
            port: 8086,
            party_master_url: "http://7d-party:8098".to_string(),
            webhook_secret: "whsec_test".to_string(),
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
        };

        let err = config.validate().unwrap_err();
        assert!(err.contains("NATS_URL is required when BUS_TYPE=nats"));
    }

    #[test]
    fn test_validate_webhook_secret_required() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8086,
            party_master_url: "http://7d-party:8098".to_string(),
            webhook_secret: "".to_string(),
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
        };

        let err = config.validate().unwrap_err();
        assert!(err.contains("TILLED_WEBHOOK_SECRET cannot be empty"));
    }

    #[test]
    fn test_validate_success() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8086,
            party_master_url: "http://7d-party:8098".to_string(),
            webhook_secret: "whsec_test".to_string(),
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
        };

        assert!(config.validate().is_ok());

        let config_nats = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::Nats,
            nats_url: Some("nats://localhost:4222".to_string()),
            host: "0.0.0.0".to_string(),
            port: 8086,
            party_master_url: "http://7d-party:8098".to_string(),
            webhook_secret: "whsec_test".to_string(),
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
        };

        assert!(config_nats.validate().is_ok());
    }
}
