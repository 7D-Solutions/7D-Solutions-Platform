//! Quality gate integration for outbound shipment composite flow.
//!
//! Checks whether any final inspections tied to the shipment's source work orders
//! are in "held" disposition before allowing a shipment to be marked shipped.
//!
//! Two modes:
//! - `Platform` — calls the Quality Inspection service via typed client
//! - `Permissive` — always returns no holds (used when QI service is not configured)

use platform_client_quality_inspection::QueriesClient;
use platform_sdk::{ClientError, PlatformClient};
use thiserror::Error;
use uuid::Uuid;

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum QualityGateError {
    #[error("Quality inspection service error: {0}")]
    Client(#[from] ClientError),
}

// ── Hold record ──────────────────────────────────────────────

/// A single quality hold — one "held" inspection on a source WO.
#[derive(Debug, Clone)]
pub struct QualityGateHold {
    pub wo_id: Uuid,
    pub inspection_id: Uuid,
}

// ── Client ───────────────────────────────────────────────────

/// Quality gate integration for outbound shipments.
#[derive(Debug, Clone)]
pub struct QualityGateIntegration {
    mode: Mode,
}

#[derive(Debug, Clone)]
enum Mode {
    Platform {
        client: PlatformClient,
    },
    /// Always returns no holds — used when no QI service is configured.
    Permissive,
    /// Always returns one hold per WO ID — used in tests to exercise the hold path
    /// without requiring a live QI service. Matches the pattern of
    /// `InventoryIntegration::deterministic()`.
    AlwaysHold,
}

impl platform_sdk::PlatformService for QualityGateIntegration {
    const SERVICE_NAME: &'static str = "quality-inspection";
    fn from_platform_client(client: PlatformClient) -> Self {
        Self {
            mode: Mode::Platform { client },
        }
    }
}

impl QualityGateIntegration {
    /// Create a permissive client that always returns no holds.
    /// Used when no QI service is configured.
    pub fn permissive() -> Self {
        Self {
            mode: Mode::Permissive,
        }
    }

    /// Create a client that always returns one synthetic hold per WO ID.
    /// Used in tests to exercise the hold path without a live QI service.
    pub fn holding() -> Self {
        Self {
            mode: Mode::AlwaysHold,
        }
    }

    /// Check whether any of the given work orders have final inspections
    /// in "held" disposition.
    ///
    /// Returns an empty `Vec` if all WOs are clear to ship.
    /// Skips WOs that have no final inspections (gate passes by absence).
    pub async fn check_wo_holds(
        &self,
        tenant_id: Uuid,
        wo_ids: &[Uuid],
    ) -> Result<Vec<QualityGateHold>, QualityGateError> {
        if wo_ids.is_empty() {
            return Ok(vec![]);
        }

        match &self.mode {
            Mode::Permissive => Ok(vec![]),
            Mode::AlwaysHold => Ok(wo_ids
                .iter()
                .map(|&wo_id| QualityGateHold {
                    wo_id,
                    inspection_id: Uuid::new_v5(
                        &Uuid::NAMESPACE_OID,
                        format!("hold:{wo_id}").as_bytes(),
                    ),
                })
                .collect()),
            Mode::Platform { client } => {
                let qi = QueriesClient::new(client.clone());
                let claims = PlatformClient::service_claims(tenant_id);
                let mut holds = Vec::new();

                for &wo_id in wo_ids {
                    let inspections = qi
                        .get_inspections_by_wo_all(&claims, wo_id, Some("final"))
                        .await?;
                    for insp in &inspections {
                        if insp.disposition == "held" {
                            holds.push(QualityGateHold {
                                wo_id,
                                inspection_id: insp.id,
                            });
                        }
                    }
                }

                Ok(holds)
            }
        }
    }
}
