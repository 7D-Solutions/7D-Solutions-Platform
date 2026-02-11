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

    // Lockout
    pub lockout_threshold: i32,
    pub lockout_minutes: i64,

    // Keyed limits
    pub login_per_min_per_email: u32,
    pub register_per_min_per_email: u32,
    pub refresh_per_min_per_token: u32,

    // Per-IP governor
    pub ip_rl_per_second: u32,
    pub ip_rl_burst: u32,

    // Hash concurrency limiting
    pub max_concurrent_hashes: usize,
    pub hash_acquire_timeout_ms: u64,
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

            lockout_threshold: env::var("LOCKOUT_THRESHOLD").unwrap_or_else(|_| "10".to_string()).parse()?,
            lockout_minutes: env::var("LOCKOUT_MINUTES").unwrap_or_else(|_| "15".to_string()).parse()?,

            login_per_min_per_email: env::var("LOGIN_PER_MIN_PER_EMAIL").unwrap_or_else(|_| "5".to_string()).parse()?,
            register_per_min_per_email: env::var("REGISTER_PER_MIN_PER_EMAIL").unwrap_or_else(|_| "5".to_string()).parse()?,
            refresh_per_min_per_token: env::var("REFRESH_PER_MIN_PER_TOKEN").unwrap_or_else(|_| "20".to_string()).parse()?,

            ip_rl_per_second: env::var("IP_RL_PER_SECOND").unwrap_or_else(|_| "10".to_string()).parse()?,
            ip_rl_burst: env::var("IP_RL_BURST").unwrap_or_else(|_| "20".to_string()).parse()?,

            max_concurrent_hashes: env::var("MAX_CONCURRENT_HASHES").unwrap_or_else(|_| "50".to_string()).parse()?,
            hash_acquire_timeout_ms: env::var("HASH_ACQUIRE_TIMEOUT_MS").unwrap_or_else(|_| "5000".to_string()).parse()?,
        })
    }
}
