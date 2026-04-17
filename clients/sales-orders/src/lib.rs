//! Typed HTTP client for the Sales Orders Service.

pub mod blankets;
pub mod orders;
pub mod types;

pub use blankets::BlanketsClient;
pub use orders::SalesOrdersClient;
pub use types::*;

impl platform_sdk::PlatformService for SalesOrdersClient {
    const SERVICE_NAME: &'static str = "sales-orders";
    fn from_platform_client(client: platform_sdk::PlatformClient) -> Self {
        Self::new(client)
    }
}

impl platform_sdk::PlatformService for BlanketsClient {
    const SERVICE_NAME: &'static str = "sales-orders";
    fn from_platform_client(client: platform_sdk::PlatformClient) -> Self {
        Self::new(client)
    }
}
