use std::env;

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
    /// Load from environment variables.
    ///
    /// Required: `DATABASE_URL` — must follow `ap_{app_id}_db` naming convention.
    /// Optional: `BUS_TYPE` (default: inmemory), `NATS_URL`, `HOST`, `PORT` (default: 8093).
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL").map_err(|_| {
            "DATABASE_URL is required. Example: postgres://ap_user:pass@localhost/ap_default_db"
                .to_string()
        })?;

        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        let bus_type_str = env::var("BUS_TYPE").unwrap_or_else(|_| "inmemory".to_string());
        let bus_type = BusType::from_str(&bus_type_str)?;

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

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "8093".to_string())
            .parse()
            .map_err(|_| "PORT must be a valid u16".to_string())?;

        let env = env::var("ENV").unwrap_or_else(|_| "development".to_string());

        let cors_origins: Vec<String> = env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

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
    fn test_bus_type_from_str() {
        assert_eq!(BusType::from_str("nats").unwrap(), BusType::Nats);
        assert_eq!(BusType::from_str("inmemory").unwrap(), BusType::InMemory);
        assert!(BusType::from_str("bad").is_err());
    }
}
