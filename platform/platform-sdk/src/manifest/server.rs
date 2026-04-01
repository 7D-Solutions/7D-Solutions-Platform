use std::collections::BTreeMap;

use serde::Deserialize;

/// `[server]` — HTTP listener defaults.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerSection {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_body_limit")]
    pub body_limit: String,
    #[serde(default = "default_request_timeout")]
    pub request_timeout: String,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            body_limit: default_body_limit(),
            request_timeout: default_request_timeout(),
            extra: BTreeMap::new(),
        }
    }
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_body_limit() -> String {
    "2mb".to_string()
}

fn default_request_timeout() -> String {
    "30s".to_string()
}
