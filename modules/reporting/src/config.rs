use config_validator::ConfigValidator;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub env: String,
    /// Comma-separated list of allowed CORS origins. "*" means allow any.
    pub cors_origins: Vec<String>,
}

impl Config {
    /// Load from environment variables, collecting ALL errors before failing.
    ///
    /// Required: `DATABASE_URL` — must follow `reporting_{app_id}_db` naming convention.
    /// Optional: `HOST`, `PORT` (default: 8096).
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("reporting");

        let database_url = v.require("DATABASE_URL").unwrap_or_default();
        let host = v.optional("HOST").or_default("0.0.0.0");
        let port = v.optional_parse::<u16>("PORT").unwrap_or(8096);
        let env_name = v.optional("ENV").or_default("development");

        let cors_raw = v.optional("CORS_ORIGINS").or_default("*");
        let cors_origins: Vec<String> = cors_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if env_name == "production" && cors_origins.iter().any(|o| o == "*") {
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
            host,
            port,
            env: env_name,
            cors_origins,
        })
    }
}

#[cfg(test)]
#[allow(unsafe_code)] // env var mutation in tests requires unsafe (Rust 1.83+)
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_config_requires_database_url() {
        // Remove DATABASE_URL to confirm error
        unsafe { std::env::remove_var("DATABASE_URL") };
        let result = Config::from_env();
        assert!(result.is_err());
        assert!(result.as_ref().unwrap_err().contains("DATABASE_URL"));
        unsafe { std::env::remove_var("DATABASE_URL") };
    }

    #[test]
    #[serial]
    fn test_config_default_port_is_8096() {
        unsafe {
            std::env::set_var(
                "DATABASE_URL",
                "postgres://user:pass@localhost/reporting_test_db",
            );
            std::env::remove_var("PORT");
            std::env::set_var("ENV", "development");
            std::env::set_var("CORS_ORIGINS", "*");
        }
        let config = Config::from_env().expect("from_env should succeed in test");
        assert_eq!(config.port, 8096);
        unsafe {
            std::env::remove_var("DATABASE_URL");
            std::env::remove_var("ENV");
            std::env::remove_var("CORS_ORIGINS");
        }
    }
}
