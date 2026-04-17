use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_database_url")]
    pub database_url: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/shop_floor_gates".to_string())
}

fn default_port() -> u16 {
    8201
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://localhost/shop_floor_gates".to_string()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8201),
        }
    }
}
