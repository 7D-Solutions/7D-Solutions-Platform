//! Audit writer API
//!
//! Provides a single writer interface for modules to record audit events

use crate::schema::{AuditEvent, WriteAuditRequest};
use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AuditWriterError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Invalid audit request: {0}")]
    InvalidRequest(String),
}

pub type Result<T> = std::result::Result<T, AuditWriterError>;

/// Audit writer for appending events to the audit log
pub struct AuditWriter {
    pool: PgPool,
}

impl AuditWriter {
    /// Create a new audit writer with the given database pool
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Write a single audit event (atomically)
    ///
    /// Returns the audit_id of the inserted event
    #[tracing::instrument(skip(self, request), fields(
        action = %request.action,
        entity_type = %request.entity_type,
        entity_id = %request.entity_id
    ))]
    pub async fn write(&self, request: WriteAuditRequest) -> Result<Uuid> {
        let audit_id = self.write_impl(&self.pool, request).await?;
        tracing::debug!(audit_id = %audit_id, "Audit event written");
        Ok(audit_id)
    }

    /// Write an audit event within an existing transaction
    ///
    /// Use this for transactional consistency with module mutations
    #[tracing::instrument(skip(tx, request), fields(
        action = %request.action,
        entity_type = %request.entity_type,
        entity_id = %request.entity_id
    ))]
    pub async fn write_in_tx(
        tx: &mut Transaction<'_, Postgres>,
        request: WriteAuditRequest,
    ) -> Result<Uuid> {
        let audit_id = Self::write_impl_tx(tx, request).await?;
        tracing::debug!(audit_id = %audit_id, "Audit event written in transaction");
        Ok(audit_id)
    }

    /// Internal implementation for pool-based writes
    async fn write_impl<'e, E>(&self, executor: E, request: WriteAuditRequest) -> Result<Uuid>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let audit_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO audit_events (
                actor_id, actor_type, action, mutation_class,
                entity_type, entity_id,
                before_snapshot, after_snapshot,
                before_hash, after_hash,
                causation_id, correlation_id, trace_id,
                metadata
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14
            )
            RETURNING audit_id
            "#,
        )
        .bind(request.actor_id)
        .bind(request.actor_type)
        .bind(request.action)
        .bind(request.mutation_class)
        .bind(request.entity_type)
        .bind(request.entity_id)
        .bind(request.before_snapshot)
        .bind(request.after_snapshot)
        .bind(request.before_hash)
        .bind(request.after_hash)
        .bind(request.causation_id)
        .bind(request.correlation_id)
        .bind(request.trace_id)
        .bind(request.metadata)
        .fetch_one(executor)
        .await?;

        Ok(audit_id)
    }

    /// Internal implementation for transaction-based writes
    async fn write_impl_tx(
        tx: &mut Transaction<'_, Postgres>,
        request: WriteAuditRequest,
    ) -> Result<Uuid> {
        let audit_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO audit_events (
                actor_id, actor_type, action, mutation_class,
                entity_type, entity_id,
                before_snapshot, after_snapshot,
                before_hash, after_hash,
                causation_id, correlation_id, trace_id,
                metadata
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14
            )
            RETURNING audit_id
            "#,
        )
        .bind(request.actor_id)
        .bind(request.actor_type)
        .bind(request.action)
        .bind(request.mutation_class)
        .bind(request.entity_type)
        .bind(request.entity_id)
        .bind(request.before_snapshot)
        .bind(request.after_snapshot)
        .bind(request.before_hash)
        .bind(request.after_hash)
        .bind(request.causation_id)
        .bind(request.correlation_id)
        .bind(request.trace_id)
        .bind(request.metadata)
        .fetch_one(&mut **tx)
        .await?;

        Ok(audit_id)
    }

    /// Query audit events by entity
    pub async fn get_by_entity(
        &self,
        entity_type: &str,
        entity_id: &str,
    ) -> Result<Vec<AuditEvent>> {
        let events = sqlx::query_as::<_, AuditEvent>(
            r#"
            SELECT
                audit_id, occurred_at,
                actor_id, actor_type,
                action, mutation_class,
                entity_type, entity_id,
                before_snapshot, after_snapshot,
                before_hash, after_hash,
                causation_id, correlation_id, trace_id,
                metadata
            FROM audit_events
            WHERE entity_type = $1 AND entity_id = $2
            ORDER BY occurred_at DESC
            "#,
        )
        .bind(entity_type)
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(events)
    }

    /// Query audit events by correlation ID
    pub async fn get_by_correlation(&self, correlation_id: Uuid) -> Result<Vec<AuditEvent>> {
        let events = sqlx::query_as::<_, AuditEvent>(
            r#"
            SELECT
                audit_id, occurred_at,
                actor_id, actor_type,
                action, mutation_class,
                entity_type, entity_id,
                before_snapshot, after_snapshot,
                before_hash, after_hash,
                causation_id, correlation_id, trace_id,
                metadata
            FROM audit_events
            WHERE correlation_id = $1
            ORDER BY occurred_at ASC
            "#,
        )
        .bind(correlation_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::MutationClass;

    #[test]
    fn test_audit_request_builder() {
        let actor_id = Uuid::new_v4();
        let request = WriteAuditRequest::new(
            actor_id,
            "User".to_string(),
            "UpdateCustomer".to_string(),
            MutationClass::Update,
            "Customer".to_string(),
            "cust_123".to_string(),
        )
        .with_correlation(None, Some(Uuid::new_v4()), Some("trace-456".to_string()));

        assert_eq!(request.actor_id, actor_id);
        assert_eq!(request.action, "UpdateCustomer");
        assert_eq!(request.mutation_class, MutationClass::Update);
        assert!(request.correlation_id.is_some());
        assert!(request.trace_id.is_some());
    }
}
