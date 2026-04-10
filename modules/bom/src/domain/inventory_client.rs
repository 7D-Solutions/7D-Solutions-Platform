use platform_client_inventory::ItemsClient;
use platform_sdk::{ClientError, PlatformClient, VerifiedClaims};
use sqlx::PgPool;
use uuid::Uuid;

use super::bom_service::BomError;
use crate::domain::models::ItemDetails;

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

    Ok(row.map(|(sku, name, description, unit_cost_minor)| ItemDetails {
        item_id,
        sku,
        name,
        description,
        unit_cost_minor,
    }))
}
