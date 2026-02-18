use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
}

impl Config {
    /// Load from environment variables.
    ///
    /// Required: `DATABASE_URL` — must follow `reporting_{app_id}_db` naming convention.
    /// Optional: `HOST`, `PORT` (default: 8096).
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL").map_err(|_| {
            "DATABASE_URL is required. Example: postgres://user:pass@localhost/reporting_default_db"
                .to_string()
        })?;

        if database_url.trim().is_empty() {
            return Err("DATABASE_URL cannot be empty".to_string());
        }

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "8096".to_string())
            .parse()
            .map_err(|_| "PORT must be a valid u16".to_string())?;

        Ok(Config {
            database_url,
            host,
            port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_requires_database_url() {
        // Remove DATABASE_URL to confirm error
        unsafe { std::env::remove_var("DATABASE_URL"); }
        let result = Config::from_env();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("DATABASE_URL is required"));
    }

    #[test]
    fn test_config_default_port_is_8096() {
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://user:pass@localhost/reporting_test_db");
            std::env::remove_var("PORT");
        }
        let config = Config::from_env().unwrap();
        assert_eq!(config.port, 8096);
        unsafe { std::env::remove_var("DATABASE_URL"); }
    }
}
