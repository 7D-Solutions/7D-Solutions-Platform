//! Runtime configuration loaded from environment variables.

#[derive(Debug, Clone)]
pub struct Config {
    pub inventory_base_url: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            inventory_base_url: std::env::var("INVENTORY_BASE_URL").ok(),
        }
    }
}
