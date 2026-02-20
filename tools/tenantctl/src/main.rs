//! tenantctl - Tenant Operations Tool
//!
//! **Purpose:** Standalone CLI tool for tenant lifecycle management operations.
//!
//! **Commands:**
//! - show: Display tenant state, mapping, and entitlements
//! - create: Provision a new tenant (databases, migrations, initial data)
//! - activate: Activate a provisioned tenant
//! - verify: Verify tenant health via module endpoints
//! - fleet: Manage fleet-wide operations
//!
//! **Usage:**
//! ```bash
//! cargo run -p tenantctl -- --help
//! cargo run -p tenantctl -- tenant show --tenant t1
//! cargo run -p tenantctl -- tenant show --tenant t1 --json
//! cargo run -p tenantctl -- tenant create --tenant t1
//! cargo run -p tenantctl -- fleet migrate --tenants 10 --parallel 4
//! ```

mod fleet_migrate;
mod lifecycle;
mod output;
mod provision;
mod retention;
mod show;
mod verify;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use output::{CommandOutput, render, render_and_exit};
use security::Role;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

// ============================================================================
// CLI Definition
// ============================================================================

/// tenantctl - Tenant Operations Tool
#[derive(Parser, Debug)]
#[command(name = "tenantctl")]
#[command(about = "Tenant lifecycle management operations", long_about = None)]
#[command(version)]
struct Cli {
    /// Role for authorization (admin, operator, auditor)
    #[arg(long, env = "TENANTCTL_ROLE")]
    role: Option<String>,

    /// Actor identifier (e.g., username, service account)
    #[arg(long, env = "TENANTCTL_ACTOR", default_value = "unknown")]
    actor: String,

    /// Output as JSON instead of human-readable text
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Tenant lifecycle operations
    Tenant {
        #[command(subcommand)]
        operation: TenantOperation,
    },
    /// Fleet-wide operations
    Fleet {
        #[command(subcommand)]
        operation: FleetOperation,
    },
}

#[derive(Subcommand, Debug)]
enum TenantOperation {
    /// Show tenant state, mapping, and entitlements
    Show {
        /// Tenant ID to inspect
        #[arg(long)]
        tenant: String,
    },
    /// Provision a new tenant (create databases, run migrations)
    Create {
        /// Tenant ID to create
        #[arg(long)]
        tenant: String,
    },
    /// Activate a provisioned tenant
    Activate {
        /// Tenant ID to activate
        #[arg(long)]
        tenant: String,
    },
    /// Verify tenant health via module endpoints
    Verify {
        /// Tenant ID to verify
        #[arg(long)]
        tenant: String,
    },
    /// Suspend a tenant (disable access, retain data)
    Suspend {
        /// Tenant ID to suspend
        #[arg(long)]
        tenant: String,
    },
    /// Deprovision a tenant (soft delete, mark for cleanup)
    Deprovision {
        /// Tenant ID to deprovision
        #[arg(long)]
        tenant: String,
    },
    /// Reset tenant to a fresh demo state (drops data, re-provisions, re-seeds)
    DemoReset {
        /// Tenant ID to reset
        #[arg(long)]
        tenant: String,
        /// Deterministic RNG seed for demo data (same seed = same data)
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// AR module base URL for demo seeding
        #[arg(long, env = "AR_BASE_URL", default_value = "http://localhost:8086")]
        ar_url: String,
    },
    /// Export tenant data to a deterministic JSONL artifact (retention framework)
    Export {
        /// Tenant ID to export
        #[arg(long)]
        tenant: String,
        /// Output file path; if omitted, only the digest is printed
        #[arg(long)]
        output: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum FleetOperation {
    /// Show fleet status
    Status,
    /// List all tenants
    List,
    /// Migrate N tenants with bounded parallelism
    Migrate {
        /// Number of tenants to migrate
        #[arg(long, default_value = "10")]
        tenants: usize,
        /// Parallel migration workers
        #[arg(long, default_value = "4")]
        parallel: usize,
    },
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();

    let cli = Cli::parse();
    let json_output = cli.json;

    let role = if let Some(role_str) = &cli.role {
        Role::from_str(role_str)
            .context(format!(
                "Invalid role: '{}'. Valid roles: admin, operator, auditor",
                role_str
            ))?
    } else {
        Role::Admin
    };

    let actor = &cli.actor;

    match cli.command {
        Commands::Tenant { operation } => match operation {
            TenantOperation::Show { tenant } => {
                let out = show::show_tenant(&tenant).await?;
                render_and_exit(out, json_output);
            }
            TenantOperation::Create { tenant } => {
                let result = provision::create_tenant(&tenant).await?;
                let out = if result.success {
                    CommandOutput::ok("created", &result.tenant_id.to_string())
                        .with_state("provisioned")
                } else {
                    let msg = result
                        .error_message
                        .unwrap_or_else(|| "Unknown error".to_string());
                    CommandOutput::fail("created", &result.tenant_id.to_string(), &msg)
                };
                render_and_exit(out, json_output);
            }
            TenantOperation::Activate { tenant } => {
                provision::activate_tenant(&tenant).await?;
                let out =
                    CommandOutput::ok("activated", &tenant).with_state("active");
                render(&out, json_output);
                Ok(())
            }
            TenantOperation::Verify { tenant } => {
                let result = verify::verify_tenant(&tenant).await?;
                let checks: Vec<serde_json::Value> = result
                    .module_results
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "module": m.module_name,
                            "ready": m.ready_check,
                            "version": m.version_check,
                            "schema_version": m.schema_version,
                        })
                    })
                    .collect();
                let out = if result.all_passed {
                    CommandOutput::ok("verified", &result.tenant_id)
                        .with_data(serde_json::json!({ "modules": checks }))
                } else {
                    CommandOutput::fail("verified", &result.tenant_id, "checks failed")
                        .with_data(serde_json::json!({ "modules": checks }))
                };
                render_and_exit(out, json_output);
            }
            TenantOperation::Suspend { tenant } => {
                lifecycle::suspend_tenant(role, actor, &tenant).await?;
                let out = CommandOutput::ok("suspended", &tenant)
                    .with_state("suspended");
                render(&out, json_output);
                Ok(())
            }
            TenantOperation::Deprovision { tenant } => {
                lifecycle::deprovision_tenant(role, actor, &tenant).await?;
                let out = CommandOutput::ok("deprovisioned", &tenant)
                    .with_state("deleted");
                render(&out, json_output);
                Ok(())
            }
            TenantOperation::DemoReset {
                tenant,
                seed,
                ar_url,
            } => {
                let result =
                    lifecycle::demo_reset_tenant(&tenant, seed, &ar_url).await?;
                let out = CommandOutput::ok("demo-reset", &tenant).with_data(
                    serde_json::json!({
                        "tenant_uuid": result.tenant_id,
                        "dataset_digest": result.dataset_digest,
                        "seed": seed,
                    }),
                );
                render(&out, json_output);
                Ok(())
            }
            TenantOperation::Export { tenant, output } => {
                let result =
                    retention::export_tenant(&tenant, output.as_deref()).await?;
                let out = CommandOutput::ok("exported", &tenant).with_data(
                    serde_json::json!({
                        "artifact": result.artifact_path,
                        "sha256": result.sha256_digest,
                        "lines": result.line_count,
                    }),
                );
                render(&out, json_output);
                Ok(())
            }
        },
        Commands::Fleet { operation } => match operation {
            FleetOperation::Status => {
                let out = CommandOutput::fail("fleet-status", "-", "not yet implemented");
                render(&out, json_output);
                Ok(())
            }
            FleetOperation::List => {
                let out = CommandOutput::fail("fleet-list", "-", "not yet implemented");
                render(&out, json_output);
                Ok(())
            }
            FleetOperation::Migrate { tenants, parallel } => {
                fleet_migrate::run_fleet_migration(role, actor, tenants, parallel)
                    .await?;
                Ok(())
            }
        },
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn json_flag_is_global() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        let json_arg = cmd
            .get_arguments()
            .find(|a| a.get_id() == "json")
            .expect("--json arg should exist");
        assert!(json_arg.is_global_set());
    }
}
