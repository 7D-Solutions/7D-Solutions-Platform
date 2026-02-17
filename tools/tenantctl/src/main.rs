//! tenantctl - Tenant Operations Tool
//!
//! **Purpose:** Standalone CLI tool for tenant lifecycle management operations.
//!
//! **Commands:**
//! - create: Provision a new tenant (databases, migrations, initial data)
//! - activate: Activate a provisioned tenant
//! - verify: Verify tenant health via module endpoints
//! - fleet: Manage fleet-wide operations
//!
//! **Usage:**
//! ```bash
//! cargo run -p tenantctl -- --help
//! cargo run -p tenantctl -- tenant create --tenant t1
//! cargo run -p tenantctl -- tenant activate --tenant t1
//! cargo run -p tenantctl -- tenant verify --tenant t1
//! cargo run -p tenantctl -- fleet migrate --tenants 10 --parallel 4
//! ```

mod fleet_migrate;
mod provision;
mod verify;

use anyhow::Result;
use clap::{Parser, Subcommand};
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
    // Initialize tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Tenant { operation } => match operation {
            TenantOperation::Create { tenant } => {
                let result = provision::create_tenant(&tenant).await?;
                if result.success {
                    println!("\n✅ Tenant {} created successfully!", result.tenant_id);
                    std::process::exit(0);
                } else {
                    eprintln!("\n❌ Tenant {} creation failed: {}",
                              result.tenant_id,
                              result.error_message.unwrap_or_else(|| "Unknown error".to_string()));
                    std::process::exit(1);
                }
            }
            TenantOperation::Activate { tenant } => {
                provision::activate_tenant(&tenant).await?;
                println!("\n✅ Tenant {} activated!", tenant);
                Ok(())
            }
            TenantOperation::Verify { tenant } => {
                let result = verify::verify_tenant(&tenant).await?;
                if result.all_passed {
                    println!("\n✅ Tenant {} verification passed!", result.tenant_id);
                    std::process::exit(0);
                } else {
                    eprintln!("\n❌ Tenant {} verification failed", result.tenant_id);
                    std::process::exit(1);
                }
            }
        },
        Commands::Fleet { operation } => match operation {
            FleetOperation::Status => {
                println!("[PLACEHOLDER] Fleet status:");
                println!("  → Fleet operations not yet implemented (bd-17s4 scaffolding only)");
                Ok(())
            }
            FleetOperation::List => {
                println!("[PLACEHOLDER] List tenants:");
                println!("  → Tenant listing not yet implemented (bd-17s4 scaffolding only)");
                Ok(())
            }
            FleetOperation::Migrate { tenants, parallel } => {
                fleet_migrate::run_fleet_migration(tenants, parallel).await?;
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
        // Ensures CLI structure compiles and basic parsing works
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
