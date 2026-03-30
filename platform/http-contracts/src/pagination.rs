use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Pagination metadata for list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PaginationMeta {
    pub page: i64,
    pub page_size: i64,
    pub total_items: i64,
    pub total_pages: i64,
}

/// Generic paginated response envelope.
///
/// Every list endpoint returns this wrapper so consumers get consistent
/// pagination metadata regardless of the underlying entity type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PaginatedResponse<T: ToSchema> {
    pub data: Vec<T>,
    pub pagination: PaginationMeta,
}

impl PaginationMeta {
    pub fn new(page: i64, page_size: i64, total_items: i64) -> Self {
        let total_pages = if page_size > 0 {
            (total_items + page_size - 1) / page_size
        } else {
            0
        };
        Self {
            page,
            page_size,
            total_items,
            total_pages,
        }
    }
}

impl<T: ToSchema> PaginatedResponse<T> {
    pub fn new(data: Vec<T>, page: i64, page_size: i64, total_items: i64) -> Self {
        Self {
            data,
            pagination: PaginationMeta::new(page, page_size, total_items),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
    struct TestItem {
        id: i64,
        name: String,
    }

    #[test]
    fn paginated_response_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let resp = PaginatedResponse::new(
            vec![
                TestItem { id: 1, name: "alpha".into() },
                TestItem { id: 2, name: "beta".into() },
            ],
            1,
            10,
            25,
        );

        let json = serde_json::to_string(&resp)?;
        let deser: PaginatedResponse<TestItem> = serde_json::from_str(&json)?;

        assert_eq!(deser.data.len(), 2);
        assert_eq!(deser.data[0].name, "alpha");
        assert_eq!(deser.pagination.page, 1);
        assert_eq!(deser.pagination.page_size, 10);
        assert_eq!(deser.pagination.total_items, 25);
        assert_eq!(deser.pagination.total_pages, 3);
        Ok(())
    }

    #[test]
    fn total_pages_rounds_up() {
        let meta = PaginationMeta::new(1, 10, 21);
        assert_eq!(meta.total_pages, 3);
    }

    #[test]
    fn zero_page_size_gives_zero_pages() {
        let meta = PaginationMeta::new(1, 0, 100);
        assert_eq!(meta.total_pages, 0);
    }

    #[test]
    fn empty_data_serializes_correctly() -> Result<(), Box<dyn std::error::Error>> {
        let resp: PaginatedResponse<TestItem> = PaginatedResponse::new(vec![], 1, 10, 0);
        let json = serde_json::to_string(&resp)?;
        assert!(json.contains("\"data\":[]"));
        assert!(json.contains("\"total_items\":0"));
        assert!(json.contains("\"total_pages\":0"));
        Ok(())
    }
}
