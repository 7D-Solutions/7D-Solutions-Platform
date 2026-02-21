/// Platform-level tenant registry primitives
///
/// This crate provides shared tenant provisioning and lifecycle
/// management infrastructure across the platform.

pub mod registry;
pub mod lifecycle;
pub mod schema;
pub mod summary;
pub mod routes;
pub mod plans;
pub mod seed;
pub mod health;
pub mod tenant_crud;

// Re-export commonly used types
pub use schema::{
    TenantId, TenantRecord, TenantStatus, Environment,
    ModuleSchemaVersions, ProvisioningStep, ProvisioningStepStatus,
    VerificationResult,
    Bundle, BundleModule, TenantBundle, TenantBundleStatus,
};

pub use lifecycle::{
    ProvisioningStepDefinition,
    ProvisioningState,
    standard_provisioning_sequence,
    get_step_definition,
    validate_step_sequence,
    step_names,
    event_types,
    is_valid_provisioning_transition,
};

pub use registry::{
    TenantRegistry, RegistryResult, RegistryError,
    is_valid_state_transition,
    EntitlementRow, get_tenant_entitlements,
    TenantAppIdRow, get_tenant_app_id,
    TenantStatusRow, get_tenant_status_row,
};

pub use summary::{
    fetch_tenant_summary,
    ModuleUrl,
    ModuleReadiness,
    ReadinessStatus,
    TenantSummary,
    SummaryError,
    MODULE_READINESS_TIMEOUT,
};

pub use routes::{
    SummaryState,
    summary_router,
    entitlements_router,
    app_id_router,
    status_router,
};

pub use plans::plans_router;

pub use tenant_crud::{
    derive_name,
    tenant_list_router,
    tenant_detail_router,
    TenantListResponse,
    TenantSummaryDto,
    TenantDetailDto,
};

pub use seed::{
    seed_gl_module,
    seed_ar_module,
    seed_subscriptions_module,
    seed_identity_module,
    seed_all_modules,
    SeedError,
    SeedResult,
};

pub use health::{
    check_all_modules_ready,
    activate_tenant_atomic,
    verify_and_activate_tenant,
    HealthCheckResult,
    ActivationError,
};
