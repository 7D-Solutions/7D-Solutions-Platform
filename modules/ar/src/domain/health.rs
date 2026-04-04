//! Health check repository — DB ping for readiness probe.

use sqlx::PgPool;

/// Execute a trivial query to verify DB connectivity.
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}
