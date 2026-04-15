//! Payments Module Configuration
//!
//! Uses ConfigValidator to report ALL missing/invalid env vars at once.
//! Invariant: Payments service never starts with missing/invalid configuration.

use config_validator::ConfigValidator;

/// Payment provider selection.
///
/// Only real payment processors are supported. Set `PAYMENTS_PROVIDER`
/// to a supported value (currently: "tilled").
#[derive(Debug, Clone, PartialEq)]
pub enum PaymentsProvider {
    Tilled,
}

impl PaymentsProvider {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "tilled" => Ok(PaymentsProvider::Tilled),
            _ => Err(format!(
                "Invalid PAYMENTS_PROVIDER '{}'. Supported: 'tilled'",
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
    /// Payment provider (required). Supported: 'tilled'
    pub payments_provider: PaymentsProvider,
    /// Tilled API key (required when PAYMENTS_PROVIDER=tilled)
    pub tilled_api_key: Option<String>,
    /// Tilled account ID (required when PAYMENTS_PROVIDER=tilled)
    pub tilled_account_id: Option<String>,
    /// Tilled webhook secret for signature verification (required when PAYMENTS_PROVIDER=tilled)
    pub tilled_webhook_secret: Option<String>,
    /// Previous Tilled webhook secret — present only during rotation overlap window.
    pub tilled_webhook_secret_prev: Option<String>,
}

impl Config {
    /// Load configuration from environment variables with structured validation.
    ///
    /// All errors are collected and reported at once via ConfigValidator.
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("payments");

        let database_url = v.require("DATABASE_URL").unwrap_or_default();
        let host = v.optional("HOST").or_default("0.0.0.0");
        let port = v.optional_parse::<u16>("PORT").unwrap_or(8088);
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

        let provider_str = v.require("PAYMENTS_PROVIDER").unwrap_or_default();
        let payments_provider = if provider_str.is_empty() {
            // v.require already recorded the error; use placeholder
            PaymentsProvider::Tilled
        } else {
            match PaymentsProvider::from_str(&provider_str) {
                Ok(p) => p,
                Err(e) => return Err(e),
            }
        };

        let is_tilled = payments_provider == PaymentsProvider::Tilled;
        let tilled_api_key = v.require_when(
            "TILLED_API_KEY",
            || is_tilled,
            "required when PAYMENTS_PROVIDER=tilled",
        );
        let tilled_account_id = v.require_when(
            "TILLED_ACCOUNT_ID",
            || is_tilled,
            "required when PAYMENTS_PROVIDER=tilled",
        );
        let tilled_webhook_secret = v.require_when(
            "TILLED_WEBHOOK_SECRET",
            || is_tilled && env_name != "development",
            "required when PAYMENTS_PROVIDER=tilled and ENV != development",
        );
        let tilled_webhook_secret_prev = v
            .optional("TILLED_WEBHOOK_SECRET_PREV")
            .present()
            .map(String::from);

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
            env: env_name,
            cors_origins,
            payments_provider,
            tilled_api_key,
            tilled_account_id,
            tilled_webhook_secret,
            tilled_webhook_secret_prev,
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
        assert_eq!(BusType::from_str("InMemory").unwrap(), BusType::InMemory);

        let err = BusType::from_str("invalid").unwrap_err();
        assert!(err.contains("Invalid BUS_TYPE"));
        assert!(err.contains("invalid"));
    }

    #[test]
    fn test_payments_provider_from_str() {
        assert_eq!(
            PaymentsProvider::from_str("tilled").unwrap(),
            PaymentsProvider::Tilled
        );
        assert_eq!(
            PaymentsProvider::from_str("TILLED").unwrap(),
            PaymentsProvider::Tilled
        );
        assert!(PaymentsProvider::from_str("mock").is_err());
        assert!(PaymentsProvider::from_str("stripe").is_err());
    }
}
