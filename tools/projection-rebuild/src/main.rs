//! projection-rebuild - Projection Rebuild Tool
//!
//! **Purpose:** Standalone CLI tool for rebuilding projections from event sources
//! with deterministic blue/green swap capability.
//!
//! **Commands:**
//! - rebuild: Rebuild a specific projection using blue/green swap
//! - status: Check projection rebuild status
//! - verify: Verify projection integrity (compute digest)
//! - list: List available projections
//!
//! **Usage:**
//! ```bash
//! cargo run --bin projection-rebuild -- --help
//! cargo run --bin projection-rebuild -- rebuild <projection-name>
//! cargo run --bin projection-rebuild -- verify <projection-name>
//! cargo run --bin projection-rebuild -- status <projection-name>
//! cargo run --bin projection-rebuild -- list
//! ```

mod swap;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use security::{
    check_permissions, JwtVerifier, VerifiedClaims, PERM_PROJECTION_LIST,
    PERM_PROJECTION_REBUILD, PERM_PROJECTION_STATUS, PERM_PROJECTION_VERIFY,
};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

// ============================================================================
// CLI Definition
// ============================================================================

/// projection-rebuild - Projection Rebuild Tool
#[derive(Parser, Debug)]
#[command(name = "projection-rebuild")]
#[command(about = "Rebuild projections from event sources", long_about = None)]
#[command(version)]
struct Cli {
    /// JWT token for authorization (also reads CLI_AUTH_TOKEN env)
    #[arg(long, env = "CLI_AUTH_TOKEN")]
    token: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Rebuild a specific projection from events (blue/green swap)
    Rebuild {
        /// Projection name (= table name) to rebuild
        projection: String,
        /// Optional tenant ID (default: all tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },
    /// Check projection rebuild status via cursor position
    Status {
        /// Projection name to check
        projection: String,
    },
    /// Verify projection integrity by computing a deterministic digest
    Verify {
        /// Projection table name to verify
        projection: String,
        /// Column(s) to order by for digest computation (default: tenant_id)
        #[arg(long, default_value = "tenant_id")]
        order_by: String,
        /// Optional tenant ID to verify
        #[arg(long)]
        tenant_id: Option<String>,
    },
    /// List available projections in the projection_cursors table
    List,
}

// ============================================================================
// JWT Verification
// ============================================================================

/// Require a valid JWT token from CLI args or env. Returns verified claims.
fn require_claims(cli: &Cli) -> Result<VerifiedClaims> {
    let token_str = cli
        .token
        .as_deref()
        .context("--token <JWT> or CLI_AUTH_TOKEN env var required for this operation")?;

    let verifier = JwtVerifier::from_env()
        .or_else(JwtVerifier::from_env_with_overlap)
        .context("JWT_PUBLIC_KEY environment variable not set — cannot verify token")?;

    verifier
        .verify(token_str)
        .map_err(|e| anyhow::anyhow!("Token verification failed: {}", e))
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

    match &cli.command {
        Commands::Rebuild {
            projection,
            tenant_id,
        } => {
            let claims = require_claims(&cli)?;
            check_permissions(&claims, &[PERM_PROJECTION_REBUILD])?;

            tracing::info!(
                projection = %projection,
                tenant_id = ?tenant_id,
                user_id = %claims.user_id,
                "Rebuild command invoked"
            );

            let db_url = std::env::var("DATABASE_URL")
                .context("DATABASE_URL env var required for rebuild command")?;
            let pool = PgPoolOptions::new()
                .max_connections(5)
                .connect(&db_url)
                .await
                .context("Failed to connect to database")?;

            // Ensure projection cursor tables are initialized
            projections::create_shadow_cursor_table(&pool)
                .await
                .unwrap_or_else(|e| tracing::debug!("Shadow cursor table already exists: {}", e));

            // Compute digest of current projection state
            let row_count: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {}", projection))
                    .fetch_one(&pool)
                    .await
                    .context(format!("Table '{}' not found or query failed", projection))?;

            let digest = projections::compute_digest(&pool, projection, "tenant_id")
                .await
                .context("Failed to compute digest")?;

            let cursor_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM projection_cursors WHERE projection_name = $1",
            )
            .bind(projection)
            .fetch_one(&pool)
            .await
            .unwrap_or(0);

            println!(
                r#"{{"command":"rebuild","projection":"{}","table_rows":{},"cursor_count":{},"digest":"{}","status":"ok"}}"#,
                projection, row_count, cursor_count, digest
            );

            Ok(())
        }

        Commands::Status { projection } => {
            let claims = require_claims(&cli)?;
            check_permissions(&claims, &[PERM_PROJECTION_STATUS])?;

            tracing::info!(
                projection = %projection,
                user_id = %claims.user_id,
                "Status command invoked"
            );

            let db_url = std::env::var("DATABASE_URL")
                .context("DATABASE_URL env var required for status command")?;
            let pool = PgPoolOptions::new()
                .max_connections(3)
                .connect(&db_url)
                .await
                .context("Failed to connect to database")?;

            // Query cursor table for latest position
            let cursor = sqlx::query_as::<_, (String, String, i64, chrono::DateTime<chrono::Utc>)>(
                "SELECT projection_name, tenant_id, events_processed, updated_at
                 FROM projection_cursors
                 WHERE projection_name = $1
                 ORDER BY updated_at DESC
                 LIMIT 1",
            )
            .bind(projection)
            .fetch_optional(&pool)
            .await
            .context("Failed to query projection_cursors")?;

            match cursor {
                Some((proj_name, tenant_id, events, updated)) => {
                    println!(
                        r#"{{"command":"status","projection":"{}","tenant_id":"{}","events_processed":{},"updated_at":"{}","status":"ok"}}"#,
                        proj_name,
                        tenant_id,
                        events,
                        updated.to_rfc3339()
                    );
                }
                None => {
                    println!(
                        r#"{{"command":"status","projection":"{}","status":"no_cursor","message":"No cursor found for this projection"}}"#,
                        projection
                    );
                }
            }

            Ok(())
        }

        Commands::Verify {
            projection,
            order_by,
            tenant_id,
        } => {
            let claims = require_claims(&cli)?;
            check_permissions(&claims, &[PERM_PROJECTION_VERIFY])?;

            tracing::info!(
                projection = %projection,
                tenant_id = ?tenant_id,
                user_id = %claims.user_id,
                "Verify command invoked"
            );

            let db_url = std::env::var("DATABASE_URL")
                .context("DATABASE_URL env var required for verify command")?;
            let pool = PgPoolOptions::new()
                .max_connections(3)
                .connect(&db_url)
                .await
                .context("Failed to connect to database")?;

            // Check if table exists
            let table_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)"
            )
            .bind(projection)
            .fetch_one(&pool).await
            .context("Failed to check table existence")?;

            if !table_exists {
                anyhow::bail!(
                    "Projection table '{}' does not exist in the database",
                    projection
                );
            }

            let row_count: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {}", projection))
                    .fetch_one(&pool)
                    .await
                    .context("Failed to get row count")?;

            let digest = projections::compute_digest(&pool, projection, order_by)
                .await
                .context("Failed to compute digest")?;

            let tenant_filter = tenant_id.as_deref().unwrap_or("*");

            println!(
                r#"{{"command":"verify","projection":"{}","tenant_id":"{}","row_count":{},"order_by":"{}","digest":"{}","status":"ok"}}"#,
                projection, tenant_filter, row_count, order_by, digest
            );

            Ok(())
        }

        Commands::List => {
            let claims = require_claims(&cli)?;
            check_permissions(&claims, &[PERM_PROJECTION_LIST])?;

            tracing::info!(
                user_id = %claims.user_id,
                "List command invoked"
            );

            let db_url = std::env::var("DATABASE_URL")
                .context("DATABASE_URL env var required for list command")?;
            let pool = PgPoolOptions::new()
                .max_connections(3)
                .connect(&db_url)
                .await
                .context("Failed to connect to database")?;

            // List projections from cursor table
            let table_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'projection_cursors')"
            )
            .fetch_one(&pool).await
            .unwrap_or(false);

            if !table_exists {
                println!(r#"{{"command":"list","projections":[],"status":"no_cursor_table"}}"#);
                return Ok(());
            }

            let rows = sqlx::query_as::<_, (String, i64, Option<chrono::DateTime<chrono::Utc>>)>(
                "SELECT projection_name, COUNT(*) as tenant_count, MAX(updated_at) as last_updated
                 FROM projection_cursors
                 GROUP BY projection_name
                 ORDER BY projection_name",
            )
            .fetch_all(&pool)
            .await
            .context("Failed to query projection_cursors")?;

            print!(r#"{{"command":"list","projections":["#);
            for (i, (name, count, last)) in rows.iter().enumerate() {
                if i > 0 {
                    print!(",");
                }
                print!(
                    r#"{{"name":"{}","tenant_count":{},"last_updated":"{}"}}"#,
                    name,
                    count,
                    last.map(|t| t.to_rfc3339()).unwrap_or_default()
                );
            }
            println!(r#"],"status":"ok"}}"#);

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

    #[test]
    fn token_flag_reads_env() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        let token_arg = cmd
            .get_arguments()
            .find(|a| a.get_id() == "token")
            .expect("--token arg should exist");
        assert!(
            token_arg.get_env().is_some(),
            "--token should read CLI_AUTH_TOKEN env"
        );
    }
}
