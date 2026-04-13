//! Module registry for tenant provisioning
//!
//! Loads per-module database connection details from environment variables
//! following the `{MODULE_UPPER}_POSTGRES_*` convention used in
//! docker-compose.services.yml.

use std::collections::HashMap;
use std::path::PathBuf;

/// Connection and migration configuration for a single module
#[derive(Debug, Clone)]
pub struct ModuleProvisioningConfig {
    pub module_code: String,
    pub postgres_host: String,
    pub postgres_port: u16,
    pub postgres_user: String,
    pub postgres_password: String,
    pub migrations_path: PathBuf,
}

/// Registry of all modules available for provisioning
#[derive(Debug, Clone)]
pub struct ModuleRegistry {
    modules: HashMap<String, ModuleProvisioningConfig>,
}

impl ModuleRegistry {
    /// Load module configs from environment variables.
    ///
    /// For each `module_code`, reads:
    /// - `{CODE}_POSTGRES_HOST` (default: `7d-{code}-postgres`)
    /// - `{CODE}_POSTGRES_PORT` (default: `5432`)
    /// - `{CODE}_POSTGRES_USER` (default: `{code}_user`)
    /// - `{CODE}_POSTGRES_PASSWORD` (default: `{code}_pass`)
    ///
    /// Migrations path is `{migrations_root}/modules/{code}/db/migrations`.
    /// `migrations_root` comes from `PROVISIONING_MIGRATIONS_ROOT` env var,
    /// defaulting to `.` (the working directory).
    pub fn from_env(module_codes: &[String]) -> Self {
        let migrations_root = std::env::var("PROVISIONING_MIGRATIONS_ROOT")
            .unwrap_or_else(|_| ".".to_string());

        let mut modules = HashMap::new();
        for code in module_codes {
            let upper = code.to_uppercase().replace('-', "_");
            let host_key = format!("{upper}_POSTGRES_HOST");
            let port_key = format!("{upper}_POSTGRES_PORT");
            let user_key = format!("{upper}_POSTGRES_USER");
            let pass_key = format!("{upper}_POSTGRES_PASSWORD");

            let default_host = format!("7d-{code}-postgres");
            let default_user = format!("{code}_user");
            let default_pass = format!("{code}_pass");

            let host = std::env::var(&host_key).unwrap_or(default_host);
            let port: u16 = std::env::var(&port_key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5432);
            let user = std::env::var(&user_key).unwrap_or(default_user);
            let password = std::env::var(&pass_key).unwrap_or(default_pass);

            let migrations_path =
                PathBuf::from(&migrations_root).join(format!("modules/{code}/db/migrations"));

            modules.insert(
                code.clone(),
                ModuleProvisioningConfig {
                    module_code: code.clone(),
                    postgres_host: host,
                    postgres_port: port,
                    postgres_user: user,
                    postgres_password: password,
                    migrations_path,
                },
            );
        }

        Self { modules }
    }

    /// Build a registry directly from pre-constructed configs.
    /// Used in tests and benchmarks where env-var lookup is not appropriate.
    pub fn from_configs(configs: Vec<(String, ModuleProvisioningConfig)>) -> Self {
        Self {
            modules: configs.into_iter().collect(),
        }
    }

    /// Look up a module by code
    pub fn get(&self, module_code: &str) -> Option<&ModuleProvisioningConfig> {
        self.modules.get(module_code)
    }

    /// All registered module codes
    pub fn module_codes(&self) -> Vec<&str> {
        self.modules.keys().map(|s| s.as_str()).collect()
    }

    /// Validate that all modules have reachable migration directories.
    /// Logs warnings for missing paths but does not fail startup.
    pub fn validate_migrations(&self) {
        for (code, config) in &self.modules {
            if !config.migrations_path.exists() {
                tracing::warn!(
                    module = %code,
                    path = %config.migrations_path.display(),
                    "migrations directory not found — provisioning will fail for this module"
                );
            }
        }
    }
}

impl ModuleProvisioningConfig {
    /// Build a Postgres connection URL for the module's admin database.
    /// Used to create tenant-specific databases.
    pub fn admin_url(&self) -> String {
        format!(
            "postgres://{}:{}@{}:{}/postgres",
            self.postgres_user, self.postgres_password, self.postgres_host, self.postgres_port
        )
    }

    /// Build a Postgres connection URL for a tenant-specific database.
    pub fn tenant_db_url(&self, db_name: &str) -> String {
        format!(
            "postgres://{}:{}@{}:{}/{}",
            self.postgres_user, self.postgres_password, self.postgres_host, self.postgres_port,
            db_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_uses_defaults_when_env_vars_missing() {
        let registry = ModuleRegistry::from_env(&["ar".to_string(), "gl".to_string()]);
        let ar = registry.get("ar").expect("ar should be in registry");
        assert_eq!(ar.postgres_host, "7d-ar-postgres");
        assert_eq!(ar.postgres_port, 5432);
        assert_eq!(ar.postgres_user, "ar_user");
        assert!(ar.migrations_path.ends_with("modules/ar/db/migrations"));
    }

    #[test]
    fn admin_url_format() {
        let config = ModuleProvisioningConfig {
            module_code: "ar".to_string(),
            postgres_host: "localhost".to_string(),
            postgres_port: 5434,
            postgres_user: "ar_user".to_string(),
            postgres_password: "ar_pass".to_string(),
            migrations_path: PathBuf::from("./modules/ar/db/migrations"),
        };
        assert_eq!(
            config.admin_url(),
            "postgres://ar_user:ar_pass@localhost:5434/postgres"
        );
    }

    #[test]
    fn tenant_db_url_format() {
        let config = ModuleProvisioningConfig {
            module_code: "ar".to_string(),
            postgres_host: "localhost".to_string(),
            postgres_port: 5434,
            postgres_user: "ar_user".to_string(),
            postgres_password: "ar_pass".to_string(),
            migrations_path: PathBuf::from("./modules/ar/db/migrations"),
        };
        assert_eq!(
            config.tenant_db_url("tenant_abc_ar_db"),
            "postgres://ar_user:ar_pass@localhost:5434/tenant_abc_ar_db"
        );
    }

    #[test]
    fn registry_returns_none_for_unknown_module() {
        let registry = ModuleRegistry::from_env(&["ar".to_string()]);
        assert!(registry.get("unknown").is_none());
    }
}
