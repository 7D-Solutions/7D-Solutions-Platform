pub const QUALITY_INSPECTION_EVENT_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

#[derive(Debug, Clone, Copy)]
pub enum QualityInspectionEventType {
    InspectionPlanCreated,
    InspectionRecorded,
    DispositionDecided,
}

impl QualityInspectionEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InspectionPlanCreated => "quality_inspection.plan_created",
            Self::InspectionRecorded => "quality_inspection.inspection_recorded",
            Self::DispositionDecided => "quality_inspection.disposition_decided",
        }
    }
}
