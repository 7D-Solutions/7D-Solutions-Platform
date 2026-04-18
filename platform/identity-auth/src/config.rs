use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Config {
    pub database_url: String,
    pub nats_url: String,
    pub host: String,
    pub port: u16,

    pub jwt_private_key_pem: String,
    pub jwt_public_key_pem: String,
    pub jwt_kid: String,
    /// Previous (retiring) JWT public key — present only during rotation overlap.
    pub jwt_prev_public_key_pem: Option<String>,
    /// key ID for the previous JWT key.
    pub jwt_prev_kid: Option<String>,

    pub access_token_ttl_minutes: i64,
    pub refresh_token_ttl_days: i64,

    // Sliding-expiry refresh sessions (cookie flow)
    pub refresh_idle_minutes: i64,
    pub refresh_absolute_max_days: i64,

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

    // Concurrent seat limit per tenant (DB-backed session leases)
    // Used as a local fallback when TENANT_REGISTRY_URL is not configured.
    pub max_concurrent_sessions: i64,

    // Tenant-registry entitlement client
    /// Base URL of the tenant-registry / control-plane HTTP API.
    /// e.g. http://localhost:8092
    /// Leave blank to disable live entitlement fetching (uses max_concurrent_sessions).
    pub tenant_registry_url: Option<String>,
    /// TTL in seconds for the per-tenant entitlement cache. Default: 60.
    pub entitlement_ttl_secs: u64,

    // Password reset
    pub password_reset_ttl_minutes: i64,
    pub forgot_per_min_per_email: u32,
    pub forgot_per_min_per_ip: u32,
    pub reset_per_min_per_ip: u32,

    // CORS
    pub env: String,
    pub cors_origins: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        dotenvy::dotenv().ok();

        Ok(Self {
            database_url: env::var("DATABASE_URL")?,
            nats_url: env::var("NATS_URL")?,

            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()?,

            jwt_private_key_pem: env::var("JWT_PRIVATE_KEY_PEM")?,
            jwt_public_key_pem: env::var("JWT_PUBLIC_KEY_PEM")?,
            jwt_kid: env::var("JWT_KID").unwrap_or_else(|_| "auth-key-1".to_string()),
            jwt_prev_public_key_pem: env::var("JWT_PREV_PUBLIC_KEY_PEM")
                .ok()
                .filter(|s| !s.is_empty()),
            jwt_prev_kid: env::var("JWT_PREV_KID").ok().filter(|s| !s.is_empty()),

            access_token_ttl_minutes: env::var("ACCESS_TOKEN_TTL_MINUTES")
                .unwrap_or_else(|_| "15".to_string())
                .parse()?,
            refresh_token_ttl_days: env::var("REFRESH_TOKEN_TTL_DAYS")
                .unwrap_or_else(|_| "14".to_string())
                .parse()?,

            refresh_idle_minutes: env::var("REFRESH_IDLE_MINUTES")
                .unwrap_or_else(|_| "480".to_string())
                .parse()?,
            refresh_absolute_max_days: env::var("REFRESH_ABSOLUTE_MAX_DAYS")
                .unwrap_or_else(|_| "30".to_string())
                .parse()?,

            argon_memory_kb: env::var("ARGON_MEMORY_KB")
                .unwrap_or_else(|_| "65536".to_string())
                .parse()?,
            argon_iterations: env::var("ARGON_ITERATIONS")
                .unwrap_or_else(|_| "3".to_string())
                .parse()?,
            argon_parallelism: env::var("ARGON_PARALLELISM")
                .unwrap_or_else(|_| "1".to_string())
                .parse()?,

            lockout_threshold: env::var("LOCKOUT_THRESHOLD")
                .unwrap_or_else(|_| "10".to_string())
                .parse()?,
            lockout_minutes: env::var("LOCKOUT_MINUTES")
                .unwrap_or_else(|_| "15".to_string())
                .parse()?,

            login_per_min_per_email: env::var("LOGIN_PER_MIN_PER_EMAIL")
                .unwrap_or_else(|_| "5".to_string())
                .parse()?,
            register_per_min_per_email: env::var("REGISTER_PER_MIN_PER_EMAIL")
                .unwrap_or_else(|_| "5".to_string())
                .parse()?,
            refresh_per_min_per_token: env::var("REFRESH_PER_MIN_PER_TOKEN")
                .unwrap_or_else(|_| "20".to_string())
                .parse()?,

            ip_rl_per_second: env::var("IP_RL_PER_SECOND")
                .unwrap_or_else(|_| "10".to_string())
                .parse()?,
            ip_rl_burst: env::var("IP_RL_BURST")
                .unwrap_or_else(|_| "20".to_string())
                .parse()?,

            max_concurrent_hashes: env::var("MAX_CONCURRENT_HASHES")
                .unwrap_or_else(|_| "50".to_string())
                .parse()?,
            hash_acquire_timeout_ms: env::var("HASH_ACQUIRE_TIMEOUT_MS")
                .unwrap_or_else(|_| "5000".to_string())
                .parse()?,

            max_concurrent_sessions: env::var("MAX_CONCURRENT_SESSIONS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()?,

            tenant_registry_url: env::var("TENANT_REGISTRY_URL")
                .ok()
                .filter(|s| !s.is_empty()),
            entitlement_ttl_secs: env::var("ENTITLEMENT_TTL_SECS")
                .unwrap_or_else(|_| "60".to_string())
                .parse()?,

            password_reset_ttl_minutes: env::var("PASSWORD_RESET_TTL_MINUTES")
                .unwrap_or_else(|_| "30".to_string())
                .parse()?,
            forgot_per_min_per_email: env::var("FORGOT_PER_MIN_PER_EMAIL")
                .unwrap_or_else(|_| "3".to_string())
                .parse()?,
            forgot_per_min_per_ip: env::var("FORGOT_PER_MIN_PER_IP")
                .unwrap_or_else(|_| "10".to_string())
                .parse()?,
            reset_per_min_per_ip: env::var("RESET_PER_MIN_PER_IP")
                .unwrap_or_else(|_| "5".to_string())
                .parse()?,

            env: env::var("ENV").unwrap_or_else(|_| "development".to_string()),
            cors_origins: {
                let origins: Vec<String> = env::var("CORS_ORIGINS")
                    .unwrap_or_else(|_| "*".to_string())
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let env_name = env::var("ENV").unwrap_or_else(|_| "development".to_string());
                if env_name == "production" && origins.iter().any(|o| o == "*") {
                    return Err("CORS_ORIGINS=* is not allowed in production. \
                         Set CORS_ORIGINS to a comma-separated list of allowed origins \
                         (e.g. https://app.example.com)"
                        .into());
                }
                origins
            },
        })
    }

    /// Parse CORS origins and validate against environment.
    /// Extracted for testability without requiring full env setup.
    pub fn parse_cors_origins(raw: &str, env_name: &str) -> Result<Vec<String>, String> {
        let origins: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if env_name == "production" && origins.iter().any(|o| o == "*") {
            return Err("CORS_ORIGINS=* is not allowed in production. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com)"
                .to_string());
        }
        Ok(origins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cors_wildcard_rejected_in_production() {
        let result = Config::parse_cors_origins("*", "production");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("CORS_ORIGINS=*"));
        assert!(err.contains("not allowed in production"));
    }

    #[test]
    fn cors_wildcard_allowed_in_development() {
        let origins = Config::parse_cors_origins("*", "development")
            .expect("wildcard should be allowed in development");
        assert_eq!(origins, vec!["*"]);
    }

    #[test]
    fn cors_specific_origins_allowed_in_production() {
        let origins = Config::parse_cors_origins(
            "https://app.example.com,https://admin.example.com",
            "production",
        )
        .expect("specific origins should be allowed in production");
        assert_eq!(origins.len(), 2);
        assert_eq!(origins[0], "https://app.example.com");
        assert_eq!(origins[1], "https://admin.example.com");
    }

    #[test]
    fn cors_trims_whitespace() {
        let origins = Config::parse_cors_origins(" https://a.com , https://b.com ", "production")
            .expect("trimmed origins should parse successfully");
        assert_eq!(origins[0], "https://a.com");
        assert_eq!(origins[1], "https://b.com");
    }
}
