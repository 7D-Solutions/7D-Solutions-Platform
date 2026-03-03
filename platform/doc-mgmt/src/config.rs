use std::env;

pub struct Config {
    pub database_url: String,
    pub nats_url: String,
    pub host: String,
    pub port: u16,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        dotenvy::dotenv().ok();

        Ok(Self {
            database_url: env::var("DATABASE_URL")?,
            nats_url: env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into()),
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: env::var("PORT").unwrap_or_else(|_| "8095".into()).parse()?,
        })
    }
}
