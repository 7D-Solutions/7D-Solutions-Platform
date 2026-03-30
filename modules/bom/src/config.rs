use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub numbering_url: String,
    pub host: String,
    pub port: u16,
    pub env: String,
    pub cors_origins: Vec<String>,
}

impl Config {
    /// Load from environment variables, collecting ALL errors before failing.
    ///
    /// Required: `DATABASE_URL`.
    /// Optional: `NUMBERING_URL`, `HOST`, `PORT` (default: 8107), `ENV`, `CORS_ORIGINS`.
    pub fn from_env() -> Result<Self, String> {
        let mut errors: Vec<String> = Vec::new();

        let database_url = match env::var("DATABASE_URL") {
            Ok(v) if v.trim().is_empty() => {
                errors.push("DATABASE_URL is set but empty".to_string());
                String::new()
            }
            Ok(v) => v,
            Err(_) => {
                errors.push(
                    "DATABASE_URL is required. \
                     Example: postgresql://bom_user:bom_pass@localhost:5450/bom_db"
                        .to_string(),
                );
                String::new()
            }
        };

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let port: u16 = match env::var("PORT")
            .unwrap_or_else(|_| "8107".to_string())
            .parse::<u16>()
        {
            Ok(p) => p,
            Err(_) => {
                errors.push(format!(
                    "PORT must be a valid u16 (0-65535), got: '{}'",
                    env::var("PORT").unwrap_or_default()
                ));
                8107
            }
        };

        let numbering_url =
            env::var("NUMBERING_URL").unwrap_or_else(|_| "http://7d-numbering:8080".to_string());

        let env_name = env::var("ENV").unwrap_or_else(|_| "development".to_string());

        let cors_origins: Vec<String> = env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if env_name == "production" && cors_origins.iter().any(|o| o == "*") {
            errors.push(
                "CORS_ORIGINS=* is not allowed in production. \
                 Set CORS_ORIGINS to a comma-separated list of allowed origins \
                 (e.g. https://app.example.com)"
                    .to_string(),
            );
        }

        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }

        Ok(Config {
            database_url,
            numbering_url,
            host,
            port,
            env: env_name,
            cors_origins,
        })
    }
}
