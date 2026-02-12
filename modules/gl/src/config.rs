use std::env;

/// Application configuration parsed from environment variables
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bus_type: String,
    pub nats_url: String,
    pub host: String,
    pub port: u16,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| "DATABASE_URL must be set".to_string())?;

        let bus_type = env::var("BUS_TYPE")
            .unwrap_or_else(|_| "inmemory".to_string());

        let nats_url = env::var("NATS_URL")
            .unwrap_or_else(|_| "nats://localhost:4222".to_string());

        let host = env::var("HOST")
            .unwrap_or_else(|_| "0.0.0.0".to_string());

        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "8090".to_string())
            .parse()
            .map_err(|_| "PORT must be a valid u16".to_string())?;

        Ok(Config {
            database_url,
            bus_type,
            nats_url,
            host,
            port,
        })
    }
}
