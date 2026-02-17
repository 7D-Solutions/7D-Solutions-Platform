//! projection-rebuild - Projection Rebuild Tool
//!
//! **Purpose:** Standalone CLI tool for rebuilding projections from event sources
//! with deterministic blue/green swap capability.
//!
//! **Commands:**
//! - rebuild: Rebuild a specific projection using blue/green swap
//! - status: Check projection rebuild status
//! - verify: Verify projection integrity
//! - list: List available projections
//!
//! **Usage:**
//! ```bash
//! cargo run --bin projection-rebuild -- --help
//! cargo run --bin projection-rebuild -- rebuild <projection-name>
//! cargo run --bin projection-rebuild -- status <projection-name>
//! cargo run --bin projection-rebuild -- verify <projection-name>
//! cargo run --bin projection-rebuild -- list
//! ```

mod swap;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use security::{RbacPolicy, Role, Operation};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

// ============================================================================
// CLI Definition
// ============================================================================

/// projection-rebuild - Projection Rebuild Tool
#[derive(Parser, Debug)]
#[command(name = "projection-rebuild")]
#[command(about = "Rebuild projections from event sources", long_about = None)]
#[command(version)]
struct Cli {
    /// Role for authorization (admin, operator, auditor)
    /// Can also be set via PROJECTION_REBUILD_ROLE environment variable
    #[arg(long, env = "PROJECTION_REBUILD_ROLE")]
    role: Option<String>,

    /// Actor identifier (e.g., username, service account)
    /// Can also be set via PROJECTION_REBUILD_ACTOR environment variable
    #[arg(long, env = "PROJECTION_REBUILD_ACTOR", default_value = "unknown")]
    actor: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Rebuild a specific projection from events
    Rebuild {
        /// Projection name to rebuild
        projection: String,
        /// Optional tenant ID (default: all tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },
    /// Check projection rebuild status
    Status {
        /// Projection name to check
        projection: String,
    },
    /// Verify projection integrity
    Verify {
        /// Projection name to verify
        projection: String,
        /// Optional tenant ID to verify
        #[arg(long)]
        tenant_id: Option<String>,
    },
    /// List available projections
    List,
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

    // Parse role for operations that require authorization
    // Default to Admin if not specified (for now)
    let role = if let Some(role_str) = &cli.role {
        Role::from_str(role_str)
            .context(format!("Invalid role: '{}'. Valid roles: admin, operator, auditor", role_str))?
    } else {
        Role::Admin
    };

    let actor = &cli.actor;

    match cli.command {
        Commands::Rebuild { projection, tenant_id } => {
            // Authorize rebuild operation
            let resource = format!("projection:{}", projection);
            RbacPolicy::authorize(role, Operation::ProjectionRebuild, actor, &resource)?;

            tracing::info!(
                projection = %projection,
                tenant_id = ?tenant_id,
                actor = actor,
                role = ?role,
                "Rebuild command invoked"
            );

            // Note: Actual rebuild implementation requires:
            // 1. Database connection pool
            // 2. Event replay function specific to the projection
            // 3. Configuration for table DDL and ordering
            //
            // This would typically be provided via a configuration file or
            // registration system for different projection types.

            println!("Rebuild projection: {}", projection);
            if let Some(tid) = tenant_id {
                println!("  → Tenant filter: {}", tid);
            }
            println!("\n⚠️  Full rebuild requires:");
            println!("  1. Database connection configuration");
            println!("  2. Event replay function for '{}'", projection);
            println!("  3. Table schema and ordering specification");
            println!("\nSee e2e-tests/tests/projection_rebuild_blue_green_e2e.rs for example usage.");

            Ok(())
        }
        Commands::Status { projection } => {
            // Authorize status check operation
            let resource = format!("projection:{}", projection);
            RbacPolicy::authorize(role, Operation::ProjectionStatus, actor, &resource)?;

            tracing::info!(
                projection = %projection,
                actor = actor,
                role = ?role,
                "Status command invoked"
            );

            println!("Check status for projection: {}", projection);
            println!("\n⚠️  Status checking requires database connection.");
            println!("This would query projection_cursors table for cursor position.");

            Ok(())
        }
        Commands::Verify { projection, tenant_id } => {
            // Authorize verify operation
            let resource = format!("projection:{}", projection);
            RbacPolicy::authorize(role, Operation::ProjectionVerify, actor, &resource)?;

            tracing::info!(
                projection = %projection,
                tenant_id = ?tenant_id,
                actor = actor,
                role = ?role,
                "Verify command invoked"
            );

            println!("Verify projection: {}", projection);
            if let Some(tid) = tenant_id {
                println!("  → Tenant filter: {}", tid);
            }
            println!("\n⚠️  Verification requires:");
            println!("  1. Database connection");
            println!("  2. Expected digest for comparison");

            Ok(())
        }
        Commands::List => {
            // Authorize list operation
            RbacPolicy::authorize(role, Operation::ProjectionList, actor, "all")?;

            tracing::info!(
                actor = actor,
                role = ?role,
                "List command invoked"
            );

            println!("List available projections:");
            println!("\n⚠️  Listing requires:");
            println!("  1. Projection registry or configuration");
            println!("  2. Database connection to query projection_cursors");

            Ok(())
        }
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
