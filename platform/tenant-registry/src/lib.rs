/// Platform-level tenant registry primitives
///
/// This crate provides shared tenant provisioning and lifecycle
/// management infrastructure across the platform.

pub mod registry;
pub mod lifecycle;
pub mod schema;
pub mod summary;
pub mod routes;

// Re-export commonly used types
pub use schema::{
    TenantId, TenantRecord, TenantStatus, Environment,
    ModuleSchemaVersions, ProvisioningStep, ProvisioningStepStatus,
    VerificationResult,
};

pub use lifecycle::{
    ProvisioningStepDefinition,
    standard_provisioning_sequence,
    get_step_definition,
    validate_step_sequence,
    step_names,
};

pub use registry::{
    TenantRegistry, RegistryResult, RegistryError,
    is_valid_state_transition,
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
};
