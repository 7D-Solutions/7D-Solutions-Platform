use platform_client_inventory::ItemsClient;
use platform_sdk::{ClientError, PlatformClient, VerifiedClaims};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use super::bom_service::BomError;
use crate::domain::models::ItemDetails;

/// Per-item on-hand availability returned by the Inventory service.
#[derive(Debug, Clone, Default)]
pub struct AvailabilityInfo {
    pub on_hand_qty: i64,
    pub expired_qty: i64,
    pub quarantine_qty: i64,
    pub available_qty: i64,
}

/// Wire-format response from Inventory's on-hand summary endpoint.
#[derive(Debug, Deserialize)]
struct OnHandSummaryResponse {
    available_qty: i64,
    #[serde(default)]
    on_hand_qty: i64,
    #[serde(default)]
    expired_qty: i64,
    #[serde(default)]
    quarantine_qty: i64,
}

// ---------------------------------------------------------------------------
// Public client type
// ---------------------------------------------------------------------------

pub struct InventoryClient {
    mode: Mode,
}

enum Mode {
    /// Production: calls the Inventory service via SDK-wired platform client.
    Platform { client: PlatformClient },
    /// Legacy: calls the Inventory service over HTTP via typed client (manual URL).
    Http { base_url: String },
    /// Test / direct: queries the Inventory database directly.
    Direct { pool: PgPool },
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

impl platform_sdk::PlatformService for InventoryClient {
    const SERVICE_NAME: &'static str = "inventory";
    fn from_platform_client(client: PlatformClient) -> Self {
        Self {
            mode: Mode::Platform { client },
        }
    }
}

impl InventoryClient {
    /// Build a client that calls the Inventory HTTP service.
    pub fn http(base_url: String) -> Self {
        Self {
            mode: Mode::Http { base_url },
        }
    }

    /// Build a client that queries the Inventory database directly.
    ///
    /// Used in integration tests where the HTTP server is not running.
    pub fn direct(pool: PgPool) -> Self {
        Self {
            mode: Mode::Direct { pool },
        }
    }

    // -----------------------------------------------------------------------
    // Fetch item details
    // -----------------------------------------------------------------------

    /// Look up a single inventory item by its UUID.
    ///
    /// Returns `Ok(None)` when the item does not exist — callers embed `item: null`
    /// rather than returning an error.
    pub async fn fetch_item_details(
        &self,
        claims: &VerifiedClaims,
        tenant_id: &str,
        item_id: Uuid,
    ) -> Result<Option<ItemDetails>, BomError> {
        match &self.mode {
            Mode::Platform { client } => fetch_via_http(client.clone(), claims, item_id).await,
            Mode::Http { base_url } => {
                let platform = PlatformClient::new(base_url.clone());
                fetch_via_http(platform, claims, item_id).await
            }
            Mode::Direct { pool } => fetch_direct(pool, tenant_id, item_id).await,
        }
    }

    // -----------------------------------------------------------------------
    // Fetch on-hand availability
    // -----------------------------------------------------------------------

    /// Query on-hand availability for a single item.
    ///
    /// Direct mode queries `item_on_hand` (test scaffold) directly.
    /// Platform/Http mode calls GET /api/inventory/items/{id}/on-hand-summary.
    /// Returns zeros if the item has no on-hand record.
    /// Returns `BomError::InventoryUnavailable` on network or unexpected errors.
    pub async fn fetch_availability(
        &self,
        claims: Option<&VerifiedClaims>,
        tenant_id: &str,
        item_id: Uuid,
    ) -> Result<AvailabilityInfo, BomError> {
        match &self.mode {
            Mode::Platform { client } => {
                let c = claims.ok_or_else(|| {
                    BomError::InventoryUnavailable("claims required for Platform mode".into())
                })?;
                fetch_availability_via_http(client.clone(), c, item_id).await
            }
            Mode::Http { base_url } => {
                let platform = PlatformClient::new(base_url.clone());
                let c = claims.ok_or_else(|| {
                    BomError::InventoryUnavailable("claims required for Http mode".into())
                })?;
                fetch_availability_via_http(platform, c, item_id).await
            }
            Mode::Direct { pool } => fetch_availability_direct(pool, tenant_id, item_id).await,
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP implementation
// ---------------------------------------------------------------------------

async fn fetch_via_http(
    client: PlatformClient,
    claims: &VerifiedClaims,
    item_id: Uuid,
) -> Result<Option<ItemDetails>, BomError> {
    let typed = ItemsClient::new(client);
    match typed.get_item(claims, item_id).await {
        Ok(item) => Ok(Some(ItemDetails {
            item_id: item.id,
            sku: item.sku,
            name: item.name,
            description: item.description,
            // unit_cost_minor is not exposed by the inventory items API in v1.
            // Populated only when the direct-DB mode is used (integration tests / future).
            unit_cost_minor: None,
        })),
        Err(ClientError::Api { status, .. }) if status == 404 => Ok(None),
        Err(ClientError::Unexpected { status, .. }) if status == 404 => Ok(None),
        Err(e) => {
            tracing::warn!(item_id = %item_id, error = %e, "inventory item lookup failed");
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Direct-DB implementation
// ---------------------------------------------------------------------------

async fn fetch_direct(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<Option<ItemDetails>, BomError> {
    let row: Option<(String, String, Option<String>, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT i.sku, i.name, i.description,
               vc.standard_cost_minor
        FROM items i
        LEFT JOIN item_valuation_configs vc
               ON vc.item_id = i.id AND vc.tenant_id = i.tenant_id
                  AND vc.method = 'standard_cost'
        WHERE i.id = $1 AND i.tenant_id = $2
        "#,
    )
    .bind(item_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(BomError::Database)?;

    Ok(
        row.map(|(sku, name, description, unit_cost_minor)| ItemDetails {
            item_id,
            sku,
            name,
            description,
            unit_cost_minor,
        }),
    )
}

// ---------------------------------------------------------------------------
// Availability — HTTP implementation
// ---------------------------------------------------------------------------

async fn fetch_availability_via_http(
    client: PlatformClient,
    claims: &VerifiedClaims,
    item_id: Uuid,
) -> Result<AvailabilityInfo, BomError> {
    let path = format!("/api/inventory/items/{}/on-hand-summary", item_id);
    let resp = client
        .get(&path, claims)
        .await
        .map_err(|e| BomError::InventoryUnavailable(e.to_string()))?;

    let status = resp.status();
    if status == 404 {
        return Ok(AvailabilityInfo::default());
    }
    if !status.is_success() {
        return Err(BomError::InventoryUnavailable(format!(
            "Inventory on-hand-summary returned HTTP {}",
            status.as_u16()
        )));
    }

    let summary: OnHandSummaryResponse = resp
        .json()
        .await
        .map_err(|e| BomError::InventoryUnavailable(e.to_string()))?;

    Ok(AvailabilityInfo {
        on_hand_qty: summary.on_hand_qty,
        expired_qty: summary.expired_qty,
        quarantine_qty: summary.quarantine_qty,
        available_qty: summary.available_qty,
    })
}

// ---------------------------------------------------------------------------
// Availability — Direct-DB implementation (integration test scaffold)
// ---------------------------------------------------------------------------

async fn fetch_availability_direct(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<AvailabilityInfo, BomError> {
    let row: Option<(i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT on_hand_qty, expired_qty, quarantine_qty, available_qty \
         FROM item_on_hand WHERE item_id = $1 AND tenant_id = $2",
    )
    .bind(item_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(BomError::Database)?;

    Ok(match row {
        Some((on_hand_qty, expired_qty, quarantine_qty, available_qty)) => AvailabilityInfo {
            on_hand_qty,
            expired_qty,
            quarantine_qty,
            available_qty,
        },
        None => AvailabilityInfo::default(),
    })
}
