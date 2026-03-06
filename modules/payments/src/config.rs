//! Payments Module Configuration
//!
//! Validates required environment variables at startup with clear error messages.
//! Invariant: Payments service never starts with missing/invalid configuration.

use std::env;

/// Payment provider selection
#[derive(Debug, Clone, PartialEq)]
pub enum PaymentsProvider {
    Mock,
    Tilled,
}

impl PaymentsProvider {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "mock" => Ok(PaymentsProvider::Mock),
            "tilled" => Ok(PaymentsProvider::Tilled),
            _ => Err(format!(
                "Invalid PAYMENTS_PROVIDER '{}'. Must be 'mock' or 'tilled'",
                s
            )),
        }
    }
}

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

/// Payments application configuration
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
    /// Payment provider: 'mock' or 'tilled' (default: mock)
    pub payments_provider: PaymentsProvider,
    /// Tilled API key (required when PAYMENTS_PROVIDER=tilled)
    pub tilled_api_key: Option<String>,
    /// Tilled account ID (required when PAYMENTS_PROVIDER=tilled)
    pub tilled_account_id: Option<String>,
    /// Tilled webhook secret for signature verification (required when PAYMENTS_PROVIDER=tilled)
    pub tilled_webhook_secret: Option<String>,
    /// Previous Tilled webhook secret — present only during rotation overlap window.
    /// Set `TILLED_WEBHOOK_SECRET_PREV` to the retiring secret, deploy, then clear it
    /// once Tilled is no longer sending webhooks signed with the old secret.
    pub tilled_webhook_secret_prev: Option<String>,
}

impl Config {
    /// Load configuration from environment variables with strict validation
    ///
    /// ## Required Environment Variables
    /// - `DATABASE_URL`: PostgreSQL connection string
    ///
    /// ## Optional Environment Variables (with defaults)
    /// - `BUS_TYPE`: 'nats' or 'inmemory' (default: 'inmemory')
    /// - `NATS_URL`: NATS server URL (default: 'nats://localhost:4222', required if BUS_TYPE=nats)
    /// - `HOST`: Bind host (default: '0.0.0.0')
    /// - `PORT`: HTTP port (default: '8088')
    /// - `PAYMENTS_PROVIDER`: 'mock' or 'tilled' (default: 'mock')
    /// - `TILLED_API_KEY`: Tilled API key (required if PAYMENTS_PROVIDER=tilled)
    /// - `TILLED_ACCOUNT_ID`: Tilled account ID (required if PAYMENTS_PROVIDER=tilled)
    /// - `TILLED_WEBHOOK_SECRET`: Tilled webhook HMAC secret (required if PAYMENTS_PROVIDER=tilled)
    ///
    /// ## Failure Modes
    /// - Missing DATABASE_URL: Service cannot persist payment data
    /// - Invalid BUS_TYPE: Service cannot communicate with other modules
    /// - Missing NATS_URL when BUS_TYPE=nats: Service cannot connect to event bus
    /// - Invalid PORT: Service cannot bind to network interface
    /// - PAYMENTS_PROVIDER=tilled without Tilled credentials: service refuses to start
    pub fn from_env() -> Result<Self, String> {
        // Required: DATABASE_URL
        let database_url = env::var("DATABASE_URL").map_err(|_| {
            "DATABASE_URL is required but not set. \
             Example: postgresql://payments_user:payments_pass@localhost:5436/payments_db"
                .to_string()
        })?;

        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        // Optional: BUS_TYPE (default: inmemory)
        let bus_type_str = env::var("BUS_TYPE").unwrap_or_else(|_| "inmemory".to_string());
        let bus_type = BusType::from_str(&bus_type_str)?;

        // Conditional: NATS_URL (required if BUS_TYPE=nats)
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

        // Optional: HOST (default: 0.0.0.0)
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        // Optional: PORT (default: 8088)
        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "8088".to_string())
            .parse()
            .map_err(|_| {
                format!(
                    "PORT must be a valid u16 (0-65535), got: '{}'",
                    env::var("PORT").unwrap_or_default()
                )
            })?;

        // Optional: PAYMENTS_PROVIDER (default: mock)
        let provider_str = env::var("PAYMENTS_PROVIDER").unwrap_or_else(|_| "mock".to_string());
        let payments_provider = PaymentsProvider::from_str(&provider_str)?;

        // Conditional: Tilled credentials (required if PAYMENTS_PROVIDER=tilled)
        let tilled_api_key = env::var("TILLED_API_KEY").ok();
        let tilled_account_id = env::var("TILLED_ACCOUNT_ID").ok();
        let tilled_webhook_secret = env::var("TILLED_WEBHOOK_SECRET").ok();
        let tilled_webhook_secret_prev = env::var("TILLED_WEBHOOK_SECRET_PREV").ok();

        let env = env::var("ENV").unwrap_or_else(|_| "development".to_string());

        let cors_origins: Vec<String> = env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if env == "production" && cors_origins.iter().any(|o| o == "*") {
            return Err(
                "CORS_ORIGINS=* is not allowed in production. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com)"
                    .to_string(),
            );
        }

        let config = Config {
            database_url,
            bus_type,
            nats_url,
            host,
            port,
            env,
            cors_origins,
            payments_provider,
            tilled_api_key,
            tilled_account_id,
            tilled_webhook_secret,
            tilled_webhook_secret_prev,
        };

        config.validate()?;
        Ok(config)
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

        if self.payments_provider == PaymentsProvider::Tilled {
            if self
                .tilled_api_key
                .as_deref()
                .map(str::is_empty)
                .unwrap_or(true)
            {
                return Err("TILLED_API_KEY is required when PAYMENTS_PROVIDER=tilled".to_string());
            }
            if self
                .tilled_account_id
                .as_deref()
                .map(str::is_empty)
                .unwrap_or(true)
            {
                return Err(
                    "TILLED_ACCOUNT_ID is required when PAYMENTS_PROVIDER=tilled".to_string(),
                );
            }
            if self
                .tilled_webhook_secret
                .as_deref()
                .map(str::is_empty)
                .unwrap_or(true)
            {
                return Err(
                    "TILLED_WEBHOOK_SECRET is required when PAYMENTS_PROVIDER=tilled".to_string(),
                );
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

    fn base_config() -> Config {
        Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8088,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            payments_provider: PaymentsProvider::Mock,
            tilled_api_key: None,
            tilled_account_id: None,
            tilled_webhook_secret: None,
            tilled_webhook_secret_prev: None,
        }
    }

    #[test]
    fn test_validate_empty_database_url() {
        let config = Config {
            database_url: "".to_string(),
            ..base_config()
        };

        let err = config.validate().unwrap_err();
        assert!(err.contains("DATABASE_URL cannot be empty"));
    }

    #[test]
    fn test_validate_nats_requires_url() {
        let config = Config {
            bus_type: BusType::Nats,
            nats_url: None,
            ..base_config()
        };

        let err = config.validate().unwrap_err();
        assert!(err.contains("NATS_URL is required when BUS_TYPE=nats"));
    }

    #[test]
    fn test_validate_success() {
        let config = base_config();
        assert!(config.validate().is_ok());

        let config_nats = Config {
            bus_type: BusType::Nats,
            nats_url: Some("nats://localhost:4222".to_string()),
            ..base_config()
        };
        assert!(config_nats.validate().is_ok());
    }

    #[test]
    fn test_payments_provider_from_str() {
        assert_eq!(
            PaymentsProvider::from_str("mock").unwrap(),
            PaymentsProvider::Mock
        );
        assert_eq!(
            PaymentsProvider::from_str("tilled").unwrap(),
            PaymentsProvider::Tilled
        );
        assert_eq!(
            PaymentsProvider::from_str("MOCK").unwrap(),
            PaymentsProvider::Mock
        );
        assert!(PaymentsProvider::from_str("stripe").is_err());
    }

    #[test]
    fn test_validate_tilled_requires_credentials() {
        let config = Config {
            payments_provider: PaymentsProvider::Tilled,
            ..base_config()
        };
        let err = config.validate().unwrap_err();
        assert!(err.contains("TILLED_API_KEY"));

        let config2 = Config {
            payments_provider: PaymentsProvider::Tilled,
            tilled_api_key: Some("sk_test_key".to_string()),
            ..base_config()
        };
        let err2 = config2.validate().unwrap_err();
        assert!(err2.contains("TILLED_ACCOUNT_ID"));

        let config3 = Config {
            payments_provider: PaymentsProvider::Tilled,
            tilled_api_key: Some("sk_test_key".to_string()),
            tilled_account_id: Some("acct_test".to_string()),
            tilled_webhook_secret: Some("whsec_test".to_string()),
            ..base_config()
        };
        assert!(config3.validate().is_ok());
    }
}
