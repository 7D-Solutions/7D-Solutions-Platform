use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const QUALITY_INSPECTION_EVENT_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

#[derive(Debug, Clone, Copy)]
pub enum QualityInspectionEventType {
    InspectionPlanCreated,
    InspectionRecorded,
    DispositionDecided,
    InspectionHeld,
    InspectionReleased,
    InspectionAccepted,
    InspectionRejected,
}

impl QualityInspectionEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InspectionPlanCreated => "quality_inspection.plan_created",
            Self::InspectionRecorded => "quality_inspection.inspection_recorded",
            Self::DispositionDecided => "quality_inspection.disposition_decided",
            Self::InspectionHeld => "quality_inspection.held",
            Self::InspectionReleased => "quality_inspection.released",
            Self::InspectionAccepted => "quality_inspection.accepted",
            Self::InspectionRejected => "quality_inspection.rejected",
        }
    }
}

// ============================================================================
// Payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanCreatedPayload {
    pub plan_id: Uuid,
    pub tenant_id: String,
    pub part_id: Uuid,
    pub revision: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InspectionRecordedPayload {
    pub inspection_id: Uuid,
    pub tenant_id: String,
    pub receipt_id: Option<Uuid>,
    pub part_id: Option<Uuid>,
    pub part_revision: Option<String>,
    pub result: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DispositionTransitionPayload {
    pub inspection_id: Uuid,
    pub tenant_id: String,
    pub previous_disposition: String,
    pub new_disposition: String,
    pub inspector_id: Option<Uuid>,
    pub reason: Option<String>,
}

// ============================================================================
// Envelope builders
// ============================================================================

fn create_qi_envelope<T>(
    event_id: Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: T,
) -> event_bus::EventEnvelope<T> {
    event_bus::EventEnvelope::with_event_id(
        event_id,
        tenant_id,
        "quality-inspection".to_string(),
        event_type,
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_trace_id(Some(correlation_id.clone()))
    .with_correlation_id(Some(correlation_id))
    .with_causation_id(causation_id)
    .with_mutation_class(Some(MUTATION_CLASS_DATA_MUTATION.to_string()))
    .with_replay_safe(true)
}

pub fn build_plan_created_envelope(
    plan_id: Uuid,
    tenant_id: String,
    part_id: Uuid,
    revision: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<PlanCreatedPayload> {
    create_qi_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        QualityInspectionEventType::InspectionPlanCreated
            .as_str()
            .to_string(),
        correlation_id,
        causation_id,
        PlanCreatedPayload {
            plan_id,
            tenant_id,
            part_id,
            revision,
        },
    )
}

pub fn build_inspection_recorded_envelope(
    inspection_id: Uuid,
    tenant_id: String,
    receipt_id: Option<Uuid>,
    part_id: Option<Uuid>,
    part_revision: Option<String>,
    result: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<InspectionRecordedPayload> {
    create_qi_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        QualityInspectionEventType::InspectionRecorded
            .as_str()
            .to_string(),
        correlation_id,
        causation_id,
        InspectionRecordedPayload {
            inspection_id,
            tenant_id,
            receipt_id,
            part_id,
            part_revision,
            result,
        },
    )
}

pub fn build_disposition_transition_envelope(
    event_type: QualityInspectionEventType,
    inspection_id: Uuid,
    tenant_id: String,
    previous_disposition: String,
    new_disposition: String,
    inspector_id: Option<Uuid>,
    reason: Option<String>,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<DispositionTransitionPayload> {
    create_qi_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        event_type.as_str().to_string(),
        correlation_id,
        causation_id,
        DispositionTransitionPayload {
            inspection_id,
            tenant_id,
            previous_disposition,
            new_disposition,
            inspector_id,
            reason,
        },
    )
}
