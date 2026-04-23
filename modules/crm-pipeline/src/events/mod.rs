//! CRM Pipeline event contracts.
//!
//! Event type strings use dot notation WITHOUT .v1 suffix in code.
//! The .v1 suffix appears only in contract filenames.
//! source_module: "crm-pipeline"

pub mod activity;
pub mod envelope;
pub mod lead;
pub mod opportunity;

pub const CRM_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";
pub const MUTATION_CLASS_LIFECYCLE: &str = "LIFECYCLE";

pub use lead::{
    build_lead_converted_envelope, build_lead_created_envelope, build_lead_status_changed_envelope,
    LeadConvertedPayload, LeadCreatedPayload, LeadStatusChangedPayload, EVENT_TYPE_LEAD_CONVERTED,
    EVENT_TYPE_LEAD_CREATED, EVENT_TYPE_LEAD_STATUS_CHANGED,
};

pub use opportunity::{
    build_opportunity_closed_lost_envelope, build_opportunity_closed_won_envelope,
    build_opportunity_created_envelope, build_opportunity_stage_advanced_envelope,
    OpportunityClosedLostPayload, OpportunityClosedWonPayload, OpportunityCreatedPayload,
    OpportunityStageAdvancedPayload, EVENT_TYPE_OPPORTUNITY_CLOSED_LOST,
    EVENT_TYPE_OPPORTUNITY_CLOSED_WON, EVENT_TYPE_OPPORTUNITY_CREATED,
    EVENT_TYPE_OPPORTUNITY_STAGE_ADVANCED,
};

pub use activity::{
    build_activity_completed_envelope, build_activity_logged_envelope,
    build_activity_overdue_envelope, ActivityCompletedPayload, ActivityLoggedPayload,
    ActivityOverduePayload, EVENT_TYPE_ACTIVITY_COMPLETED, EVENT_TYPE_ACTIVITY_LOGGED,
    EVENT_TYPE_ACTIVITY_OVERDUE,
};

pub use envelope::EventEnvelope;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_event_types_use_crm_pipeline_prefix() {
        let events = [
            EVENT_TYPE_LEAD_CREATED,
            EVENT_TYPE_LEAD_STATUS_CHANGED,
            EVENT_TYPE_LEAD_CONVERTED,
            EVENT_TYPE_OPPORTUNITY_CREATED,
            EVENT_TYPE_OPPORTUNITY_STAGE_ADVANCED,
            EVENT_TYPE_OPPORTUNITY_CLOSED_WON,
            EVENT_TYPE_OPPORTUNITY_CLOSED_LOST,
            EVENT_TYPE_ACTIVITY_LOGGED,
            EVENT_TYPE_ACTIVITY_COMPLETED,
            EVENT_TYPE_ACTIVITY_OVERDUE,
        ];
        for et in &events {
            assert!(
                et.starts_with("crm_pipeline."),
                "Event type '{}' must start with 'crm_pipeline.'",
                et
            );
            assert!(
                !et.ends_with(".v1"),
                "Event type '{}' must not end with .v1 (no version suffix in code)",
                et
            );
        }
    }
}
