use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const BOM_EVENT_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

#[derive(Debug, Clone, Copy)]
pub enum BomEventType {
    BomCreated,
    RevisionCreated,
    EffectivitySet,
    LineAdded,
    LineUpdated,
    LineRemoved,
    EcoCreated,
    EcoSubmitted,
    EcoApproved,
    EcoRejected,
    EcoApplied,
    RevisionSuperseded,
    RevisionReleased,
    MrpExploded,
    KitReadinessChecked,
}

impl BomEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BomCreated => "bom.created",
            Self::RevisionCreated => "bom.revision_created",
            Self::EffectivitySet => "bom.effectivity_set",
            Self::LineAdded => "bom.line_added",
            Self::LineUpdated => "bom.line_updated",
            Self::LineRemoved => "bom.line_removed",
            Self::EcoCreated => "eco.created",
            Self::EcoSubmitted => "eco.submitted",
            Self::EcoApproved => "eco.approved",
            Self::EcoRejected => "eco.rejected",
            Self::EcoApplied => "eco.applied",
            Self::RevisionSuperseded => "bom.revision_superseded",
            Self::RevisionReleased => "bom.revision_released",
            Self::MrpExploded => "bom.mrp_exploded",
            Self::KitReadinessChecked => "bom.kit_readiness_checked",
        }
    }
}

// ============================================================================
// Payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct BomCreatedPayload {
    pub bom_id: Uuid,
    pub tenant_id: String,
    pub part_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevisionCreatedPayload {
    pub revision_id: Uuid,
    pub bom_id: Uuid,
    pub tenant_id: String,
    pub revision_label: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EffectivitySetPayload {
    pub revision_id: Uuid,
    pub bom_id: Uuid,
    pub tenant_id: String,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LineAddedPayload {
    pub line_id: Uuid,
    pub revision_id: Uuid,
    pub tenant_id: String,
    pub component_item_id: Uuid,
    pub quantity: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LineUpdatedPayload {
    pub line_id: Uuid,
    pub revision_id: Uuid,
    pub tenant_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LineRemovedPayload {
    pub line_id: Uuid,
    pub revision_id: Uuid,
    pub tenant_id: String,
    pub component_item_id: Uuid,
}

// ============================================================================
// Envelope builders
// ============================================================================

fn create_bom_envelope<T>(
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
        "bom".to_string(),
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

pub fn build_bom_created_envelope(
    bom_id: Uuid,
    tenant_id: String,
    part_id: Uuid,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<BomCreatedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::BomCreated.as_str().to_string(),
        correlation_id,
        causation_id,
        BomCreatedPayload {
            bom_id,
            tenant_id,
            part_id,
        },
    )
}

pub fn build_revision_created_envelope(
    revision_id: Uuid,
    bom_id: Uuid,
    tenant_id: String,
    revision_label: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<RevisionCreatedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::RevisionCreated.as_str().to_string(),
        correlation_id,
        causation_id,
        RevisionCreatedPayload {
            revision_id,
            bom_id,
            tenant_id,
            revision_label,
        },
    )
}

pub fn build_effectivity_set_envelope(
    revision_id: Uuid,
    bom_id: Uuid,
    tenant_id: String,
    effective_from: DateTime<Utc>,
    effective_to: Option<DateTime<Utc>>,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<EffectivitySetPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::EffectivitySet.as_str().to_string(),
        correlation_id,
        causation_id,
        EffectivitySetPayload {
            revision_id,
            bom_id,
            tenant_id,
            effective_from,
            effective_to,
        },
    )
}

pub fn build_line_added_envelope(
    line_id: Uuid,
    revision_id: Uuid,
    tenant_id: String,
    component_item_id: Uuid,
    quantity: f64,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<LineAddedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::LineAdded.as_str().to_string(),
        correlation_id,
        causation_id,
        LineAddedPayload {
            line_id,
            revision_id,
            tenant_id,
            component_item_id,
            quantity,
        },
    )
}

pub fn build_line_updated_envelope(
    line_id: Uuid,
    revision_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<LineUpdatedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::LineUpdated.as_str().to_string(),
        correlation_id,
        causation_id,
        LineUpdatedPayload {
            line_id,
            revision_id,
            tenant_id,
        },
    )
}

pub fn build_line_removed_envelope(
    line_id: Uuid,
    revision_id: Uuid,
    tenant_id: String,
    component_item_id: Uuid,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<LineRemovedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::LineRemoved.as_str().to_string(),
        correlation_id,
        causation_id,
        LineRemovedPayload {
            line_id,
            revision_id,
            tenant_id,
            component_item_id,
        },
    )
}

// ============================================================================
// ECO event payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct EcoCreatedPayload {
    pub eco_id: Uuid,
    pub tenant_id: String,
    pub eco_number: String,
    pub title: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EcoStatusChangedPayload {
    pub eco_id: Uuid,
    pub tenant_id: String,
    pub eco_number: String,
    pub new_status: String,
    pub actor: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EcoAppliedPayload {
    pub eco_id: Uuid,
    pub tenant_id: String,
    pub eco_number: String,
    pub bom_id: Uuid,
    pub before_revision_id: Uuid,
    pub after_revision_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevisionSupersededPayload {
    pub revision_id: Uuid,
    pub bom_id: Uuid,
    pub tenant_id: String,
    pub eco_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevisionReleasedPayload {
    pub revision_id: Uuid,
    pub bom_id: Uuid,
    pub tenant_id: String,
    pub eco_id: Uuid,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
}

// ============================================================================
// ECO envelope builders
// ============================================================================

pub fn build_eco_created_envelope(
    eco_id: Uuid,
    tenant_id: String,
    eco_number: String,
    title: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<EcoCreatedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::EcoCreated.as_str().to_string(),
        correlation_id,
        causation_id,
        EcoCreatedPayload {
            eco_id,
            tenant_id,
            eco_number,
            title,
        },
    )
}

pub fn build_eco_status_changed_envelope(
    event_type: BomEventType,
    eco_id: Uuid,
    tenant_id: String,
    eco_number: String,
    new_status: String,
    actor: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<EcoStatusChangedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        event_type.as_str().to_string(),
        correlation_id,
        causation_id,
        EcoStatusChangedPayload {
            eco_id,
            tenant_id,
            eco_number,
            new_status,
            actor,
        },
    )
}

pub fn build_eco_applied_envelope(
    eco_id: Uuid,
    tenant_id: String,
    eco_number: String,
    bom_id: Uuid,
    before_revision_id: Uuid,
    after_revision_id: Uuid,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<EcoAppliedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::EcoApplied.as_str().to_string(),
        correlation_id,
        causation_id,
        EcoAppliedPayload {
            eco_id,
            tenant_id,
            eco_number,
            bom_id,
            before_revision_id,
            after_revision_id,
        },
    )
}

pub fn build_revision_superseded_envelope(
    revision_id: Uuid,
    bom_id: Uuid,
    tenant_id: String,
    eco_id: Uuid,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<RevisionSupersededPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::RevisionSuperseded.as_str().to_string(),
        correlation_id,
        causation_id,
        RevisionSupersededPayload {
            revision_id,
            bom_id,
            tenant_id,
            eco_id,
        },
    )
}

pub fn build_revision_released_envelope(
    revision_id: Uuid,
    bom_id: Uuid,
    tenant_id: String,
    eco_id: Uuid,
    effective_from: DateTime<Utc>,
    effective_to: Option<DateTime<Utc>>,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<RevisionReleasedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::RevisionReleased.as_str().to_string(),
        correlation_id,
        causation_id,
        RevisionReleasedPayload {
            revision_id,
            bom_id,
            tenant_id,
            eco_id,
            effective_from,
            effective_to,
        },
    )
}

// ============================================================================
// MRP event payload + envelope builder
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct MrpExplodedPayload {
    pub snapshot_id: Uuid,
    pub bom_id: Uuid,
    pub demand_quantity: f64,
    pub line_count: i64,
    pub net_shortage_count: i64,
}

// ============================================================================
// Kit Readiness event payload + envelope builder
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct KitReadinessCheckedPayload {
    pub snapshot_id: Uuid,
    pub bom_id: Uuid,
    pub overall_status: String,
}

pub fn build_kit_readiness_checked_envelope(
    snapshot_id: Uuid,
    bom_id: Uuid,
    overall_status: String,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<KitReadinessCheckedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::KitReadinessChecked.as_str().to_string(),
        correlation_id,
        causation_id,
        KitReadinessCheckedPayload {
            snapshot_id,
            bom_id,
            overall_status,
        },
    )
}

pub fn build_mrp_exploded_envelope(
    snapshot_id: Uuid,
    bom_id: Uuid,
    demand_quantity: f64,
    line_count: i64,
    net_shortage_count: i64,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<MrpExplodedPayload> {
    create_bom_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        BomEventType::MrpExploded.as_str().to_string(),
        correlation_id,
        causation_id,
        MrpExplodedPayload {
            snapshot_id,
            bom_id,
            demand_quantity,
            line_count,
            net_shortage_count,
        },
    )
}
