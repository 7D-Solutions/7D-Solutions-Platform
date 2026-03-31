//! GL Module Configuration
//!
//! Validates required environment variables at startup with clear error messages.
//! Invariant: GL service never starts with missing/invalid configuration.

use std::collections::HashMap;

use config_validator::ConfigValidator;
use serde::{Deserialize, Serialize};

// ============================================================================
// Currency Configuration
// ============================================================================

/// Per-tenant currency configuration.
///
/// Every tenant (identified by `app_id`) has a **reporting currency** — the
/// currency in which consolidated financial statements are produced. Individual
/// transactions may be denominated in any **transaction currency** (ISO 4217);
/// FX events translate between the two.
///
/// ## Semantics
///
/// - **transaction_currency**: The currency of the original business event
///   (invoice, payment, journal entry). Carried on every GL line.
/// - **reporting_currency**: The tenant's functional / presentation currency.
///   All FX gain/loss calculations convert *from* transaction currency *to*
///   reporting currency.
///
/// ## Example
///
/// ```rust
/// use gl_rs::config::CurrencyConfig;
///
/// let cfg = CurrencyConfig::new("USD");
/// assert_eq!(cfg.reporting_currency, "USD");
/// assert!(cfg.is_valid());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CurrencyConfig {
    /// ISO 4217 currency code for the tenant's reporting (functional) currency.
    /// All FX revaluation and realized gain/loss is measured against this.
    pub reporting_currency: String,
}

impl CurrencyConfig {
    /// Create a new currency config with the given reporting currency.
    pub fn new(reporting_currency: &str) -> Self {
        Self {
            reporting_currency: reporting_currency.to_uppercase(),
        }
    }

    /// Validate the currency config.
    ///
    /// Rules:
    /// - reporting_currency must be exactly 3 uppercase ASCII letters (ISO 4217)
    pub fn is_valid(&self) -> bool {
        let rc = &self.reporting_currency;
        rc.len() == 3 && rc.chars().all(|c| c.is_ascii_uppercase())
    }

    /// Returns true if the transaction currency differs from reporting currency.
    pub fn requires_fx(&self, transaction_currency: &str) -> bool {
        transaction_currency.to_uppercase() != self.reporting_currency
    }
}

/// Default reporting currency when none is configured for a tenant.
pub const DEFAULT_REPORTING_CURRENCY: &str = "USD";

/// In-memory registry of per-tenant currency configs.
///
/// In production this would be loaded from the tenant registry database.
/// For now it provides a programmatic API that downstream beads (bd-104+)
/// will back with persistent storage.
#[derive(Debug, Clone, Default)]
pub struct CurrencyConfigRegistry {
    configs: HashMap<String, CurrencyConfig>,
}

impl CurrencyConfigRegistry {
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
        }
    }

    /// Register a reporting currency for a tenant (app_id).
    pub fn set(&mut self, app_id: &str, config: CurrencyConfig) {
        self.configs.insert(app_id.to_string(), config);
    }

    /// Look up the currency config for a tenant. Falls back to USD.
    pub fn get(&self, app_id: &str) -> CurrencyConfig {
        self.configs
            .get(app_id)
            .cloned()
            .unwrap_or_else(|| CurrencyConfig::new(DEFAULT_REPORTING_CURRENCY))
    }

    /// Check if a tenant has an explicit currency config.
    pub fn has(&self, app_id: &str) -> bool {
        self.configs.contains_key(app_id)
    }
}

// ============================================================================
// Application Configuration
// ============================================================================

/// Application configuration parsed from environment variables
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bus_type: String,
    pub nats_url: String,
    pub host: String,
    pub port: u16,
    pub env: String,
    /// Comma-separated list of allowed CORS origins. "*" means allow any.
    pub cors_origins: Vec<String>,
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
        let mut v = ConfigValidator::new("gl");

        // Required: DATABASE_URL
        let database_url = v.require("DATABASE_URL").unwrap_or_default();

        // Optional: BUS_TYPE (default: inmemory)
        let bus_type = v.optional("BUS_TYPE").or_default("inmemory");

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
        let nats_url = v.optional("NATS_URL").or_default("nats://localhost:4222");

        // Optional: HOST (default: 0.0.0.0)
        let host = v.optional("HOST").or_default("0.0.0.0");

        // Optional: PORT (default: 8090)
        let port = v.optional_parse::<u16>("PORT").unwrap_or(8090);

        // Optional: ENV (default: development)
        let env = v.optional("ENV").or_default("development");

        // Optional: DLQ_VALIDATION_ENABLED (default: false)
        let dlq_validation_enabled = v
            .optional_parse::<bool>("DLQ_VALIDATION_ENABLED")
            .unwrap_or(false);

        let cors_raw = v.optional("CORS_ORIGINS").or_default("*");
        let cors_origins: Vec<String> = cors_raw
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

        v.finish().map_err(|e| e.to_string())?;

        Ok(Config {
            database_url,
            bus_type,
            nats_url,
            host,
            port,
            env,
            cors_origins,
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
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
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
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
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
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
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
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            dlq_validation_enabled: false,
        };

        assert!(config.validate().is_ok());

        let config_nats = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: "nats".to_string(),
            nats_url: "nats://localhost:4222".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8090,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            dlq_validation_enabled: true,
        };

        assert!(config_nats.validate().is_ok());
    }

    // ─── CurrencyConfig ─────────────────────────────────────────────────────

    #[test]
    fn currency_config_normalizes_to_uppercase() {
        let cfg = CurrencyConfig::new("eur");
        assert_eq!(cfg.reporting_currency, "EUR");
    }

    #[test]
    fn currency_config_valid_iso_4217() {
        assert!(CurrencyConfig::new("USD").is_valid());
        assert!(CurrencyConfig::new("EUR").is_valid());
        assert!(CurrencyConfig::new("GBP").is_valid());
        assert!(CurrencyConfig::new("JPY").is_valid());
    }

    #[test]
    fn currency_config_rejects_invalid_codes() {
        // Too short
        assert!(!CurrencyConfig::new("US").is_valid());
        // Too long
        assert!(!CurrencyConfig::new("USDD").is_valid());
        // Contains digits
        assert!(!CurrencyConfig {
            reporting_currency: "U2D".to_string()
        }
        .is_valid());
        // Empty
        assert!(!CurrencyConfig {
            reporting_currency: "".to_string()
        }
        .is_valid());
    }

    #[test]
    fn currency_config_requires_fx_when_different() {
        let cfg = CurrencyConfig::new("USD");
        assert!(cfg.requires_fx("EUR"));
        assert!(cfg.requires_fx("eur")); // case-insensitive comparison
        assert!(!cfg.requires_fx("USD"));
        assert!(!cfg.requires_fx("usd")); // case-insensitive
    }

    #[test]
    fn currency_config_serializes_correctly() {
        let cfg = CurrencyConfig::new("GBP");
        let json = serde_json::to_string(&cfg).expect("serialize CurrencyConfig");
        assert!(json.contains("\"GBP\""));
        let roundtrip: CurrencyConfig = serde_json::from_str(&json).expect("deserialize CurrencyConfig");
        assert_eq!(roundtrip, cfg);
    }

    // ─── CurrencyConfigRegistry ─────────────────────────────────────────────

    #[test]
    fn registry_returns_default_for_unknown_tenant() {
        let registry = CurrencyConfigRegistry::new();
        let cfg = registry.get("unknown-tenant");
        assert_eq!(cfg.reporting_currency, DEFAULT_REPORTING_CURRENCY);
    }

    #[test]
    fn registry_stores_and_retrieves_tenant_config() {
        let mut registry = CurrencyConfigRegistry::new();
        registry.set("tenant-eu", CurrencyConfig::new("EUR"));
        registry.set("tenant-uk", CurrencyConfig::new("GBP"));

        assert_eq!(registry.get("tenant-eu").reporting_currency, "EUR");
        assert_eq!(registry.get("tenant-uk").reporting_currency, "GBP");
        assert!(registry.has("tenant-eu"));
        assert!(!registry.has("tenant-unknown"));
    }

    #[test]
    fn registry_overwrites_existing_config() {
        let mut registry = CurrencyConfigRegistry::new();
        registry.set("t1", CurrencyConfig::new("EUR"));
        assert_eq!(registry.get("t1").reporting_currency, "EUR");

        registry.set("t1", CurrencyConfig::new("GBP"));
        assert_eq!(registry.get("t1").reporting_currency, "GBP");
    }
}
