//! compliance-export - Compliance Export Tool
//!
//! **Purpose:** Standalone CLI tool for exporting audit logs and ledger data
//! for compliance, regulatory, and data retention purposes.
//!
//! **Commands:**
//! - export: Export audit and ledger data for a tenant
//! - evidence-pack: Generate evidence pack for a closed period
//!
//! **Usage:**
//! ```bash
//! cargo run -p compliance-export -- --help
//! cargo run -p compliance-export -- export --tenant t1 --output ./export/
//! cargo run -p compliance-export -- evidence-pack --tenant t1 --period-id <uuid> --output ./pack/
//! ```

use anyhow::Result;
use clap::{Parser, Subcommand};
use compliance_export::export_compliance_data;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

// ============================================================================
// CLI Definition
// ============================================================================

/// compliance-export - Compliance Export Tool
#[derive(Parser, Debug)]
#[command(name = "compliance-export")]
#[command(about = "Export audit logs and ledger data for compliance", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Export audit and ledger data for a tenant
    Export {
        /// Tenant ID to export data for
        #[arg(long)]
        tenant: String,

        /// Output directory for exported data
        #[arg(long)]
        output: String,

        /// Export format (json, csv, parquet)
        #[arg(long, default_value = "json")]
        format: String,

        /// Date range start (YYYY-MM-DD)
        #[arg(long)]
        from: Option<String>,

        /// Date range end (YYYY-MM-DD)
        #[arg(long)]
        to: Option<String>,
    },

    /// Generate evidence pack for a closed accounting period
    EvidencePack {
        /// Tenant ID
        #[arg(long)]
        tenant: String,

        /// Period UUID
        #[arg(long)]
        period_id: String,

        /// Output directory for the evidence pack
        #[arg(long)]
        output: String,

        /// Path to a compliance-export manifest.json to reference
        #[arg(long)]
        manifest: Option<String>,
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
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Export {
            tenant,
            output,
            format,
            from,
            to,
        } => {
            tracing::info!(
                tenant = %tenant,
                output = %output,
                format = %format,
                "Export command invoked"
            );

            // Date range filtering is not yet implemented
            if from.is_some() || to.is_some() {
                tracing::warn!(
                    "Date range filtering (--from/--to) is not yet implemented and will be ignored"
                );
            }

            export_compliance_data(&tenant, &output, &format).await?;

            Ok(())
        }
        Commands::EvidencePack {
            tenant,
            period_id,
            output,
            manifest,
        } => {
            tracing::info!(
                tenant = %tenant,
                period_id = %period_id,
                output = %output,
                "Evidence pack command invoked"
            );

            let period_uuid: uuid::Uuid = period_id
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid period UUID: {}", e))?;

            let gl_url = std::env::var("GL_DATABASE_URL")
                .map_err(|_| anyhow::anyhow!("GL_DATABASE_URL not set"))?;
            let gl_pool = sqlx::PgPool::connect(&gl_url)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to connect to GL database: {}", e))?;

            let pack = compliance_export::generate_evidence_pack(
                &gl_pool,
                &tenant,
                period_uuid,
                manifest.as_deref(),
            )
            .await?;

            std::fs::create_dir_all(&output)?;
            let output_path = std::path::Path::new(&output).join("evidence_pack.json");
            compliance_export::evidence_pack::write_evidence_pack(&pack, &output_path)?;

            gl_pool.close().await;

            println!("Evidence pack generated successfully");
            println!("  Tenant: {}", tenant);
            println!("  Period: {}", period_id);
            println!("  Closed: {}", pack.close_state.is_closed);
            if let Some(ref hash) = pack.close_state.close_hash {
                println!("  Close hash: {}...", &hash[..16.min(hash.len())]);
            }
            println!("  Reopen count: {}", pack.close_state.reopen_count);
            println!("  Reopen history: {} entries", pack.reopen_history.len());
            println!("  Pack hash: {}...", &pack.pack_hash[..16]);
            println!("  Output: {}", output_path.display());

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
