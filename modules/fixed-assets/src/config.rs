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
            other => Err(format!(
                "Invalid BUS_TYPE '{}'. Must be 'nats' or 'inmemory'",
                other
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
    /// Comma-separated list of allowed CORS origins. "*" means allow any.
    pub cors_origins: Vec<String>,
}

impl Config {
    /// Load from environment variables.
    ///
    /// Required: `DATABASE_URL` — must follow `fa_{app_id}_db` naming convention.
    /// Optional: `BUS_TYPE` (default: inmemory), `NATS_URL`, `HOST`, `PORT` (default: 8104).
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("fixed-assets");

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
        let port: u16 = v.optional_parse::<u16>("PORT").unwrap_or(8104);

        let env_val = v.optional("ENV").or_default("development");

        let cors_origins: Vec<String> = v
            .optional("CORS_ORIGINS")
            .or_default("*")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        v.finish().map_err(|e| e.to_string())?;

        if env_val == "production" && cors_origins.iter().any(|o| o == "*") {
            return Err("CORS_ORIGINS=* is not allowed in production. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com)"
                .to_string());
        }
        Ok(Config {
            database_url,
            bus_type,
            nats_url,
            host,
            port,
            env: env_val,
            cors_origins,
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
        assert!(BusType::from_str("bad").is_err());
    }

    #[test]
    fn test_bus_type_case_insensitive() {
        assert_eq!(BusType::from_str("NATS").unwrap(), BusType::Nats);
        assert_eq!(BusType::from_str("InMemory").unwrap(), BusType::InMemory);
    }
}
