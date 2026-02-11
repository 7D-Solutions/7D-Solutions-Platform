use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub database_url: String,
    pub nats_url: String,
    pub host: String,
    pub port: u16,

    pub jwt_private_key_pem: String,
    pub jwt_public_key_pem: String,
    pub jwt_kid: String,

    pub access_token_ttl_minutes: i64,
    pub refresh_token_ttl_days: i64,

    pub argon_memory_kb: u32,
    pub argon_iterations: u32,
    pub argon_parallelism: u32,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        dotenvy::dotenv().ok();

        Ok(Self {
            database_url: env::var("DATABASE_URL")?,
            nats_url: env::var("NATS_URL")?,

            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("PORT").unwrap_or_else(|_| "8080".to_string()).parse()?,

            jwt_private_key_pem: env::var("JWT_PRIVATE_KEY_PEM")?,
            jwt_public_key_pem: env::var("JWT_PUBLIC_KEY_PEM")?,
            jwt_kid: env::var("JWT_KID").unwrap_or_else(|_| "auth-key-1".to_string()),

            access_token_ttl_minutes: env::var("ACCESS_TOKEN_TTL_MINUTES")
                .unwrap_or_else(|_| "15".to_string())
                .parse()?,
            refresh_token_ttl_days: env::var("REFRESH_TOKEN_TTL_DAYS")
                .unwrap_or_else(|_| "14".to_string())
                .parse()?,

            argon_memory_kb: env::var("ARGON_MEMORY_KB").unwrap_or_else(|_| "65536".to_string()).parse()?,
            argon_iterations: env::var("ARGON_ITERATIONS").unwrap_or_else(|_| "3".to_string()).parse()?,
            argon_parallelism: env::var("ARGON_PARALLELISM").unwrap_or_else(|_| "1".to_string()).parse()?,
        })
    }
}
