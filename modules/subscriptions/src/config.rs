use std::env;

#[derive(Debug, Clone)]
pub enum BusType {
    Nats,
    InMemory,
}

impl BusType {
    pub fn from_env() -> Self {
        match env::var("BUS_TYPE")
            .unwrap_or_else(|_| "inmemory".to_string())
            .to_lowercase()
            .as_str()
        {
            "nats" => BusType::Nats,
            "inmemory" => BusType::InMemory,
            _ => {
                tracing::warn!("Unknown BUS_TYPE, defaulting to inmemory");
                BusType::InMemory
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bus_type: BusType,
    pub database_url: String,
    pub nats_url: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let bus_type = BusType::from_env();
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| "DATABASE_URL must be set".to_string())?;

        let nats_url = match bus_type {
            BusType::Nats => Some(
                env::var("NATS_URL")
                    .unwrap_or_else(|_| "nats://localhost:4222".to_string()),
            ),
            BusType::InMemory => None,
        };

        Ok(Self {
            bus_type,
            database_url,
            nats_url,
        })
    }
}
