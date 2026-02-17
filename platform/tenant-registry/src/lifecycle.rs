/// Tenant lifecycle management
///
/// Defines deterministic provisioning sequences with verification checks

use serde::{Deserialize, Serialize};

/// Standard provisioning step names (deterministic sequence)
pub mod step_names {
    pub const VALIDATE_TENANT_ID: &str = "validate_tenant_id";
    pub const CREATE_TENANT_DATABASES: &str = "create_tenant_databases";
    pub const RUN_SCHEMA_MIGRATIONS: &str = "run_schema_migrations";
    pub const SEED_INITIAL_DATA: &str = "seed_initial_data";
    pub const VERIFY_DATABASE_CONNECTIVITY: &str = "verify_database_connectivity";
    pub const VERIFY_SCHEMA_VERSIONS: &str = "verify_schema_versions";
    pub const ACTIVATE_TENANT: &str = "activate_tenant";
}

/// Provisioning step definition with verification requirements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisioningStepDefinition {
    pub step_name: &'static str,
    pub step_order: i32,
    pub description: &'static str,
    pub verification_checks: Vec<&'static str>,
}

/// Standard tenant provisioning sequence
/// Returns the canonical ordered list of provisioning steps
pub fn standard_provisioning_sequence() -> Vec<ProvisioningStepDefinition> {
    vec![
        ProvisioningStepDefinition {
            step_name: step_names::VALIDATE_TENANT_ID,
            step_order: 1,
            description: "Validate tenant ID is unique and well-formed",
            verification_checks: vec!["tenant_id_format_valid", "tenant_id_not_duplicate"],
        },
        ProvisioningStepDefinition {
            step_name: step_names::CREATE_TENANT_DATABASES,
            step_order: 2,
            description: "Create per-module PostgreSQL databases for tenant",
            verification_checks: vec![
                "ar_database_exists",
                "payments_database_exists",
                "subscriptions_database_exists",
                "gl_database_exists",
                "notifications_database_exists",
            ],
        },
        ProvisioningStepDefinition {
            step_name: step_names::RUN_SCHEMA_MIGRATIONS,
            step_order: 3,
            description: "Apply latest schema migrations to all module databases",
            verification_checks: vec![
                "ar_migrations_applied",
                "payments_migrations_applied",
                "subscriptions_migrations_applied",
                "gl_migrations_applied",
                "notifications_migrations_applied",
            ],
        },
        ProvisioningStepDefinition {
            step_name: step_names::SEED_INITIAL_DATA,
            step_order: 4,
            description: "Seed required initial data (chart of accounts, default settings)",
            verification_checks: vec![
                "chart_of_accounts_seeded",
                "default_settings_created",
            ],
        },
        ProvisioningStepDefinition {
            step_name: step_names::VERIFY_DATABASE_CONNECTIVITY,
            step_order: 5,
            description: "Verify all module databases are reachable and responsive",
            verification_checks: vec![
                "ar_db_ping_success",
                "payments_db_ping_success",
                "subscriptions_db_ping_success",
                "gl_db_ping_success",
                "notifications_db_ping_success",
            ],
        },
        ProvisioningStepDefinition {
            step_name: step_names::VERIFY_SCHEMA_VERSIONS,
            step_order: 6,
            description: "Record and verify schema versions for all modules",
            verification_checks: vec!["all_module_versions_recorded"],
        },
        ProvisioningStepDefinition {
            step_name: step_names::ACTIVATE_TENANT,
            step_order: 7,
            description: "Transition tenant from provisioning to active status",
            verification_checks: vec!["tenant_status_active", "tenant_accessible"],
        },
    ]
}

/// Get provisioning step by name
pub fn get_step_definition(step_name: &str) -> Option<ProvisioningStepDefinition> {
    standard_provisioning_sequence()
        .into_iter()
        .find(|step| step.step_name == step_name)
}

/// Validate that a step sequence is complete and correctly ordered
pub fn validate_step_sequence(step_names: &[String]) -> Result<(), String> {
    let standard = standard_provisioning_sequence();

    if step_names.len() != standard.len() {
        return Err(format!(
            "Expected {} steps, got {}",
            standard.len(),
            step_names.len()
        ));
    }

    for (i, step_name) in step_names.iter().enumerate() {
        if step_name != standard[i].step_name {
            return Err(format!(
                "Step order mismatch at position {}: expected {}, got {}",
                i, standard[i].step_name, step_name
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_sequence_has_seven_steps() {
        let sequence = standard_provisioning_sequence();
        assert_eq!(sequence.len(), 7);
    }

    #[test]
    fn steps_are_ordered_sequentially() {
        let sequence = standard_provisioning_sequence();
        for (i, step) in sequence.iter().enumerate() {
            assert_eq!(step.step_order, (i + 1) as i32);
        }
    }

    #[test]
    fn all_steps_have_verification_checks() {
        let sequence = standard_provisioning_sequence();
        for step in sequence {
            assert!(!step.verification_checks.is_empty());
        }
    }

    #[test]
    fn get_step_definition_finds_existing_step() {
        let step = get_step_definition(step_names::CREATE_TENANT_DATABASES);
        assert!(step.is_some());
        assert_eq!(step.unwrap().step_order, 2);
    }

    #[test]
    fn get_step_definition_returns_none_for_unknown() {
        let step = get_step_definition("nonexistent_step");
        assert!(step.is_none());
    }

    #[test]
    fn validate_sequence_accepts_correct_order() {
        let step_names: Vec<String> = standard_provisioning_sequence()
            .iter()
            .map(|s| s.step_name.to_string())
            .collect();
        assert!(validate_step_sequence(&step_names).is_ok());
    }

    #[test]
    fn validate_sequence_rejects_wrong_count() {
        let step_names = vec!["step1".to_string(), "step2".to_string()];
        assert!(validate_step_sequence(&step_names).is_err());
    }

    #[test]
    fn validate_sequence_rejects_wrong_order() {
        let mut step_names: Vec<String> = standard_provisioning_sequence()
            .iter()
            .map(|s| s.step_name.to_string())
            .collect();
        // Swap first two steps
        step_names.swap(0, 1);
        assert!(validate_step_sequence(&step_names).is_err());
    }
}
