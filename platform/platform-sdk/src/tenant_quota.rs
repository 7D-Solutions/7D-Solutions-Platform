use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::manifest::DatabaseSection;

/// In-memory per-tenant connection budget tracking.
///
/// Each tenant gets its own semaphore. The permit is held by
/// [`TenantPoolGuard`](crate::tenant::TenantPoolGuard) until the underlying
/// database connection is dropped, so the quota is released automatically.
#[derive(Debug)]
pub struct TenantQuota {
    default_max_connections: usize,
    budgets: DashMap<Uuid, Arc<Semaphore>>,
}

/// Error returned when a tenant budget cannot be acquired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantQuotaError {
    BudgetExceeded {
        tenant_id: Uuid,
        max_connections: usize,
    },
}

impl TenantQuota {
    /// Build a tenant quota from the manifest's database section.
    pub fn from_database_section(database: Option<&DatabaseSection>) -> Self {
        let default_max_connections = database
            .and_then(|db| db.tenant_quota.as_ref())
            .map(|quota| quota.max_connections as usize)
            .unwrap_or(5)
            .max(1);

        Self::new(default_max_connections)
    }

    /// Create a quota with the provided default maximum concurrent connections.
    pub fn new(default_max_connections: usize) -> Self {
        Self {
            default_max_connections: default_max_connections.max(1),
            budgets: DashMap::new(),
        }
    }

    /// Default maximum concurrent connections per tenant.
    pub fn default_max_connections(&self) -> usize {
        self.default_max_connections
    }

    fn budget_for(&self, tenant_id: Uuid) -> Arc<Semaphore> {
        self.budgets
            .entry(tenant_id)
            .or_insert_with(|| Arc::new(Semaphore::new(self.default_max_connections)))
            .clone()
    }

    /// Try to reserve one connection slot for the tenant.
    ///
    /// Returns immediately with [`TenantQuotaError::BudgetExceeded`] when the
    /// tenant has already reached the configured connection budget.
    pub fn try_acquire(&self, tenant_id: Uuid) -> Result<OwnedSemaphorePermit, TenantQuotaError> {
        let budget = self.budget_for(tenant_id);
        budget
            .try_acquire_owned()
            .map_err(|_| TenantQuotaError::BudgetExceeded {
                tenant_id,
                max_connections: self.default_max_connections,
            })
    }
}

impl Default for TenantQuota {
    fn default() -> Self {
        Self::new(5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_is_per_tenant() {
        let quota = TenantQuota::new(2);
        let tenant_a = Uuid::new_v4();
        let tenant_b = Uuid::new_v4();

        let _a1 = quota.try_acquire(tenant_a).expect("first tenant A permit");
        let _a2 = quota.try_acquire(tenant_a).expect("second tenant A permit");
        assert!(matches!(
            quota.try_acquire(tenant_a),
            Err(TenantQuotaError::BudgetExceeded {
                tenant_id,
                max_connections,
            }) if tenant_id == tenant_a && max_connections == 2
        ));

        let _b1 = quota
            .try_acquire(tenant_b)
            .expect("tenant B should still have budget");
        let _b2 = quota.try_acquire(tenant_b).expect("tenant B second permit");
    }

    #[test]
    fn permit_is_released_on_drop() {
        let quota = TenantQuota::new(1);
        let tenant = Uuid::new_v4();

        let permit = quota.try_acquire(tenant).expect("first permit");
        assert!(matches!(
            quota.try_acquire(tenant),
            Err(TenantQuotaError::BudgetExceeded { .. })
        ));

        drop(permit);

        let _permit = quota
            .try_acquire(tenant)
            .expect("permit should be released on drop");
    }
}
