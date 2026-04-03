//! Notifications Module Configuration
//!
//! Validates required environment variables at startup with clear error messages.
//! Invariant: Notifications service never starts with missing/invalid configuration.

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

#[derive(Debug, Clone, PartialEq)]
pub enum EmailSenderType {
    Logging,
    Http,
    SendGrid,
}

impl EmailSenderType {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "logging" => Ok(EmailSenderType::Logging),
            "http" => Ok(EmailSenderType::Http),
            "sendgrid" => Ok(EmailSenderType::SendGrid),
            _ => Err(format!(
                "Invalid EMAIL_SENDER_TYPE '{}'. Must be 'logging', 'http', or 'sendgrid'",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SmsSenderType {
    Logging,
    Http,
}

impl SmsSenderType {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "logging" => Ok(SmsSenderType::Logging),
            "http" => Ok(SmsSenderType::Http),
            _ => Err(format!(
                "Invalid SMS_SENDER_TYPE '{}'. Must be 'logging' or 'http'",
                s
            )),
        }
    }
}

/// Notifications application configuration
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
    pub email_sender_type: EmailSenderType,
    pub email_http_endpoint: Option<String>,
    pub email_from: String,
    pub email_api_key: Option<String>,
    pub sendgrid_api_key: Option<String>,
    pub sms_sender_type: SmsSenderType,
    pub sms_http_endpoint: Option<String>,
    pub sms_from_number: String,
    pub sms_api_key: Option<String>,
    pub retry_max_attempts: i32,
    pub retry_backoff_base_secs: i64,
    pub retry_backoff_multiplier: f64,
    pub retry_backoff_max_secs: i64,
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
    /// - `PORT`: HTTP port (default: '8089')
    ///
    /// ## Failure Modes
    /// - Missing DATABASE_URL: Service cannot persist notification data
    /// - Invalid BUS_TYPE: Service cannot communicate with other modules
    /// - Missing NATS_URL when BUS_TYPE=nats: Service cannot connect to event bus
    /// - Invalid PORT: Service cannot bind to network interface
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("notifications");

        // Required: DATABASE_URL
        let database_url = v
            .require("DATABASE_URL")
            .unwrap_or_default();

        // Optional: BUS_TYPE (default: inmemory)
        let bus_type_str = v.optional("BUS_TYPE").or_default("inmemory");
        let bus_type = BusType::from_str(&bus_type_str)?;

        // Conditional: NATS_URL (required if BUS_TYPE=nats)
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

        // Optional: HOST (default: 0.0.0.0)
        let host = v.optional("HOST").or_default("0.0.0.0");

        // Optional: PORT (default: 8089)
        let port: u16 = v.optional_parse::<u16>("PORT").unwrap_or(8089);

        let env_val = v.optional("ENV").or_default("development");

        let cors_origins: Vec<String> = v
            .optional("CORS_ORIGINS")
            .or_default("*")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let email_sender_type = EmailSenderType::from_str(
            &v.optional("EMAIL_SENDER_TYPE").or_default("logging"),
        )?;
        let email_http_endpoint = env::var("EMAIL_HTTP_ENDPOINT").ok();
        let email_from = v
            .optional("EMAIL_FROM")
            .or_default("no-reply@notifications.local");
        let email_api_key = env::var("EMAIL_API_KEY").ok();
        let sendgrid_api_key = env::var("SENDGRID_API_KEY").ok();
        let sms_sender_type = SmsSenderType::from_str(
            &v.optional("SMS_SENDER_TYPE").or_default("logging"),
        )?;
        let sms_http_endpoint = env::var("SMS_HTTP_ENDPOINT").ok();
        let sms_from_number = v
            .optional("SMS_FROM_NUMBER")
            .or_default("+10000000000");
        let sms_api_key = env::var("SMS_API_KEY").ok();
        let retry_max_attempts = v
            .optional_parse::<i32>("NOTIFICATIONS_RETRY_MAX_ATTEMPTS")
            .unwrap_or(5);
        let retry_backoff_base_secs = v
            .optional_parse::<i64>("NOTIFICATIONS_RETRY_BACKOFF_BASE_SECS")
            .unwrap_or(300);
        let retry_backoff_multiplier = v
            .optional_parse::<f64>("NOTIFICATIONS_RETRY_BACKOFF_MULTIPLIER")
            .unwrap_or(1.0);
        let retry_backoff_max_secs = v
            .optional_parse::<i64>("NOTIFICATIONS_RETRY_BACKOFF_MAX_SECS")
            .unwrap_or(3600);

        v.finish().map_err(|e| e.to_string())?;

        if env_val == "production" && cors_origins.iter().any(|o| o == "*") {
            return Err(
                "CORS_ORIGINS=* is not allowed in production. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com)"
                    .to_string(),
            );
        }
        Ok(Config {
            database_url,
            bus_type,
            nats_url,
            host,
            port,
            env: env_val,
            cors_origins,
            email_sender_type,
            email_http_endpoint,
            email_from,
            email_api_key,
            sendgrid_api_key,
            sms_sender_type,
            sms_http_endpoint,
            sms_from_number,
            sms_api_key,
            retry_max_attempts,
            retry_backoff_base_secs,
            retry_backoff_multiplier,
            retry_backoff_max_secs,
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

        if self.email_sender_type == EmailSenderType::Http
            && self
                .email_http_endpoint
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
        {
            return Err("EMAIL_HTTP_ENDPOINT is required when EMAIL_SENDER_TYPE=http".to_string());
        }
        if self.email_sender_type == EmailSenderType::SendGrid
            && self
                .sendgrid_api_key
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
        {
            return Err(
                "SENDGRID_API_KEY is required when EMAIL_SENDER_TYPE=sendgrid".to_string(),
            );
        }
        if self.sms_sender_type == SmsSenderType::Http
            && self
                .sms_http_endpoint
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
        {
            return Err("SMS_HTTP_ENDPOINT is required when SMS_SENDER_TYPE=http".to_string());
        }
        if self.retry_max_attempts < 1 {
            return Err("NOTIFICATIONS_RETRY_MAX_ATTEMPTS must be >= 1".to_string());
        }
        if self.retry_backoff_base_secs < 1 {
            return Err("NOTIFICATIONS_RETRY_BACKOFF_BASE_SECS must be >= 1".to_string());
        }
        if self.retry_backoff_multiplier < 1.0 {
            return Err("NOTIFICATIONS_RETRY_BACKOFF_MULTIPLIER must be >= 1.0".to_string());
        }
        if self.retry_backoff_max_secs < self.retry_backoff_base_secs {
            return Err(
                "NOTIFICATIONS_RETRY_BACKOFF_MAX_SECS must be >= NOTIFICATIONS_RETRY_BACKOFF_BASE_SECS"
                    .to_string(),
            );
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
            port: 8089,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            email_sender_type: EmailSenderType::Logging,
            email_http_endpoint: None,
            email_from: "no-reply@notifications.local".to_string(),
            email_api_key: None,
            sendgrid_api_key: None,
            sms_sender_type: SmsSenderType::Logging,
            sms_http_endpoint: None,
            sms_from_number: "+10000000000".to_string(),
            sms_api_key: None,
            retry_max_attempts: 5,
            retry_backoff_base_secs: 300,
            retry_backoff_multiplier: 1.0,
            retry_backoff_max_secs: 3600,
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
            port: 8089,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            email_sender_type: EmailSenderType::Logging,
            email_http_endpoint: None,
            email_from: "no-reply@notifications.local".to_string(),
            email_api_key: None,
            sendgrid_api_key: None,
            sms_sender_type: SmsSenderType::Logging,
            sms_http_endpoint: None,
            sms_from_number: "+10000000000".to_string(),
            sms_api_key: None,
            retry_max_attempts: 5,
            retry_backoff_base_secs: 300,
            retry_backoff_multiplier: 1.0,
            retry_backoff_max_secs: 3600,
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
            host: "0.0.0.0".to_string(),
            port: 8089,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            email_sender_type: EmailSenderType::Logging,
            email_http_endpoint: None,
            email_from: "no-reply@notifications.local".to_string(),
            email_api_key: None,
            sendgrid_api_key: None,
            sms_sender_type: SmsSenderType::Logging,
            sms_http_endpoint: None,
            sms_from_number: "+10000000000".to_string(),
            sms_api_key: None,
            retry_max_attempts: 5,
            retry_backoff_base_secs: 300,
            retry_backoff_multiplier: 1.0,
            retry_backoff_max_secs: 3600,
        };

        assert!(config.validate().is_ok());

        let config_nats = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::Nats,
            nats_url: Some("nats://localhost:4222".to_string()),
            host: "0.0.0.0".to_string(),
            port: 8089,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            email_sender_type: EmailSenderType::Logging,
            email_http_endpoint: None,
            email_from: "no-reply@notifications.local".to_string(),
            email_api_key: None,
            sendgrid_api_key: None,
            sms_sender_type: SmsSenderType::Logging,
            sms_http_endpoint: None,
            sms_from_number: "+10000000000".to_string(),
            sms_api_key: None,
            retry_max_attempts: 5,
            retry_backoff_base_secs: 300,
            retry_backoff_multiplier: 1.0,
            retry_backoff_max_secs: 3600,
        };

        assert!(config_nats.validate().is_ok());
    }
}
