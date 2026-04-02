use config_validator::ConfigValidator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusType {
    Nats,
    InMemory,
}

impl BusType {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "nats" => Ok(BusType::Nats),
            "inmemory" => Ok(BusType::InMemory),
            other => Err(format!(
                "Invalid BUS_TYPE '{}'. Must be 'nats' or 'inmemory'",
                other
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub env: String,
    pub cors_origins: Vec<String>,
    pub bus_type: BusType,
    pub nats_url: Option<String>,
    pub workforce_competence_base_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let mut v = ConfigValidator::new("quality-inspection");

        let database_url = v.require("DATABASE_URL").unwrap_or_default();
        let host = v.optional("HOST").or_default("0.0.0.0");
        let port = v.optional_parse::<u16>("PORT").unwrap_or(8106);
        let env_name = v.optional("ENV").or_default("development");

        let cors_raw = v.optional("CORS_ORIGINS").or_default("*");
        let cors_origins: Vec<String> = cors_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let bus_type_str = v.optional("BUS_TYPE").or_default("inmemory");
        let bus_type = match BusType::from_str(&bus_type_str) {
            Ok(bt) => bt,
            Err(_err) => {
                // ConfigValidator doesn't have a push_error method, so we handle
                // the error by returning early after finish collects other errors.
                // The BusType::from_str error message is clear enough.
                return Err(format!(
                    "Config validation failed for module quality-inspection:\n{}",
                    _err
                ));
            }
        };

        let nats_url = v.require_when(
            "NATS_URL",
            || bus_type == BusType::Nats,
            "required when BUS_TYPE=nats",
        );

        let workforce_competence_base_url = v
            .optional("WORKFORCE_COMPETENCE_BASE_URL")
            .or_default("http://localhost:8121");

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
            bus_type,
            nats_url,
            workforce_competence_base_url,
        })
    }
}
