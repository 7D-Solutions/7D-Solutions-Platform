use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Payload for GL entry reversal request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlEntryReverseRequestV1 {
    pub original_entry_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Payload for GL entry reversed notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlEntryReversedV1 {
    pub original_entry_id: Uuid,
    pub reversal_entry_id: Uuid,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posted_at: Option<String>,
}
