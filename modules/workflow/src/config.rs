//! Workflow Module Configuration
//!
//! Validates required environment variables at startup with clear error messages.

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
    pub database_url: String,
    pub bus_type: BusType,
    pub nats_url: Option<String>,
    pub host: String,
    pub port: u16,
    pub env: String,
    pub cors_origins: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("workflow");

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
        let port = v.optional_parse::<u16>("PORT").unwrap_or(8110);
        let env = v.optional("ENV").or_default("development");

        let cors_origins: Vec<String> = v
            .optional("CORS_ORIGINS")
            .or_default("*")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if env == "production" && cors_origins.iter().any(|o| o == "*") {
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
            env,
            cors_origins,
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
    }

    #[test]
    fn bus_type_from_str_invalid() {
        let err = BusType::from_str("kafka").unwrap_err();
        assert!(err.contains("Invalid BUS_TYPE"));
    }
}
