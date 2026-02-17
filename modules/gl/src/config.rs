//! GL Module Configuration
//!
//! Validates required environment variables at startup with clear error messages.
//! Invariant: GL service never starts with missing/invalid configuration.

use std::env;

/// Application configuration parsed from environment variables
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bus_type: String,
    pub nats_url: String,
    pub host: String,
    pub port: u16,
    /// Enable DLQ validation during period close (default: false)
    /// When enabled, period close will fail if tenant has pending DLQ entries
    /// for posting-related subjects
    pub dlq_validation_enabled: bool,
}

impl Config {
    /// Load configuration from environment variables with strict validation
    ///
    /// ## Required Environment Variables
    /// - `DATABASE_URL`: PostgreSQL connection string
    ///
    /// ## Optional Environment Variables (with defaults)
    /// - `BUS_TYPE`: 'nats' or 'inmemory' (default: 'inmemory')
    /// - `NATS_URL`: NATS server URL (default: 'nats://localhost:4222')
    /// - `HOST`: Bind host (default: '0.0.0.0')
    /// - `PORT`: HTTP port (default: '8090')
    /// - `DLQ_VALIDATION_ENABLED`: Enable DLQ validation during period close (default: 'false')
    ///
    /// ## Failure Modes
    /// - Missing DATABASE_URL: Service cannot persist GL data
    /// - Invalid BUS_TYPE: Service cannot communicate with other modules
    /// - Invalid PORT: Service cannot bind to network interface
    pub fn from_env() -> Result<Self, String> {
        // Required: DATABASE_URL
        let database_url = env::var("DATABASE_URL").map_err(|_| {
            "DATABASE_URL is required but not set. \
             Example: postgresql://gl_user:gl_pass@localhost:5438/gl_db"
                .to_string()
        })?;

        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        // Optional: BUS_TYPE (default: inmemory)
        let bus_type = env::var("BUS_TYPE").unwrap_or_else(|_| "inmemory".to_string());

        // Validate BUS_TYPE
        match bus_type.to_lowercase().as_str() {
            "nats" | "inmemory" => {}
            _ => {
                return Err(format!(
                    "Invalid BUS_TYPE '{}'. Must be 'nats' or 'inmemory'",
                    bus_type
                ));
            }
        }

        // Optional: NATS_URL (default: nats://localhost:4222)
        let nats_url = env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());

        if nats_url.trim().is_empty() {
            return Err("NATS_URL cannot be empty".to_string());
        }

        // Optional: HOST (default: 0.0.0.0)
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        // Optional: PORT (default: 8090)
        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "8090".to_string())
            .parse()
            .map_err(|_| {
                format!(
                    "PORT must be a valid u16 (0-65535), got: '{}'",
                    env::var("PORT").unwrap_or_default()
                )
            })?;

        // Optional: DLQ_VALIDATION_ENABLED (default: false)
        let dlq_validation_enabled = env::var("DLQ_VALIDATION_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            .parse()
            .unwrap_or(false);

        Ok(Config {
            database_url,
            bus_type,
            nats_url,
            host,
            port,
            dlq_validation_enabled,
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

        match self.bus_type.to_lowercase().as_str() {
            "nats" | "inmemory" => {}
            _ => {
                return Err(format!(
                    "Invalid BUS_TYPE '{}'. Must be 'nats' or 'inmemory'",
                    self.bus_type
                ));
            }
        }

        if self.nats_url.trim().is_empty() {
            return Err("NATS_URL cannot be empty".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_empty_database_url() {
        let config = Config {
            database_url: "".to_string(),
            bus_type: "inmemory".to_string(),
            nats_url: "nats://localhost:4222".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8090,
            dlq_validation_enabled: false,
        };

        let err = config.validate().unwrap_err();
        assert!(err.contains("DATABASE_URL cannot be empty"));
    }

    #[test]
    fn test_validate_invalid_bus_type() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: "invalid".to_string(),
            nats_url: "nats://localhost:4222".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8090,
            dlq_validation_enabled: false,
        };

        let err = config.validate().unwrap_err();
        assert!(err.contains("Invalid BUS_TYPE"));
        assert!(err.contains("invalid"));
    }

    #[test]
    fn test_validate_empty_nats_url() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: "nats".to_string(),
            nats_url: "".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8090,
            dlq_validation_enabled: false,
        };

        let err = config.validate().unwrap_err();
        assert!(err.contains("NATS_URL cannot be empty"));
    }

    #[test]
    fn test_validate_success() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: "inmemory".to_string(),
            nats_url: "nats://localhost:4222".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8090,
            dlq_validation_enabled: false,
        };

        assert!(config.validate().is_ok());

        let config_nats = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: "nats".to_string(),
            nats_url: "nats://localhost:4222".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8090,
            dlq_validation_enabled: true,
        };

        assert!(config_nats.validate().is_ok());
    }
}
