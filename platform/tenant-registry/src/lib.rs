//! Platform-level tenant registry primitives
//!
//! This crate provides shared tenant provisioning and lifecycle
//! management infrastructure across the platform.

pub mod health;
pub mod lifecycle;
pub mod plans;
pub mod registry;
pub mod routes;
pub mod schema;
pub mod seed;
pub mod summary;
pub mod tenant_crud;

// Re-export commonly used types
pub use schema::{
    Bundle, BundleModule, Environment, ModuleSchemaVersions, ProvisioningStep,
    ProvisioningStepStatus, TenantBundle, TenantBundleStatus, TenantId, TenantRecord, TenantStatus,
    VerificationResult,
};

pub use lifecycle::{
    event_types, get_step_definition, is_valid_provisioning_transition,
    standard_provisioning_sequence, step_names, validate_step_sequence, ProvisioningState,
    ProvisioningStepDefinition,
};

pub use registry::{
    get_tenant_app_id, get_tenant_entitlements, get_tenant_status_row, is_valid_state_transition,
    EntitlementRow, RegistryError, RegistryResult, TenantAppIdRow, TenantRegistry, TenantStatusRow,
};

pub use summary::{
    fetch_tenant_summary, ModuleReadiness, ModuleUrl, ReadinessStatus, SummaryError, TenantSummary,
    MODULE_READINESS_TIMEOUT,
};

pub use routes::{app_id_router, entitlements_router, status_router, summary_router, SummaryState};

pub use plans::plans_router;

pub use tenant_crud::{
    derive_name, tenant_detail_router, tenant_list_router, TenantDetailDto, TenantListResponse,
    TenantSummaryDto,
};

pub use seed::{
    seed_all_modules, seed_ar_module, seed_gl_module, seed_identity_module,
    seed_subscriptions_module, SeedError, SeedResult,
};

pub use health::{
    activate_tenant_atomic, check_all_modules_ready, verify_and_activate_tenant, ActivationError,
    HealthCheckResult,
};
