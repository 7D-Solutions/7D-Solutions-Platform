use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    50
}

/// URL query parameters for paginated list endpoints.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct PaginationQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}
