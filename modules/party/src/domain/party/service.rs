use crate::domain::party::{create, query, update};

pub use create::{create_company, create_individual};
pub use query::{get_party, list_parties, search_parties};
pub use update::{deactivate_party, reactivate_party, update_party};

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_APP: &str = "test-party-crud";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://party_user:party_pass@localhost:5448/party_db".to_string()
        })
    }

    async fn test_pool() -> Result<PgPool, sqlx::Error> {
        let pool = PgPool::connect(&test_db_url()).await?;
        sqlx::migrate!("./db/migrations").run(&pool).await?;
        Ok(pool)
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM party_outbox WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM party_parties WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
    }

    fn sample_company_req(name: &str) -> crate::domain::party::models::CreateCompanyRequest {
        crate::domain::party::models::CreateCompanyRequest {
            display_name: name.to_string(),
            legal_name: format!("{} LLC", name),
            trade_name: None,
            registration_number: Some("REG-001".to_string()),
            tax_id: Some("12-3456789".to_string()),
            country_of_incorporation: Some("US".to_string()),
            industry_code: Some("tech".to_string()),
            founded_date: None,
            employee_count: Some(42),
            annual_revenue_cents: Some(1_000_000),
            currency: Some("usd".to_string()),
            email: Some("ops@example.com".to_string()),
            phone: Some("+1-555-0100".to_string()),
            website: Some("https://example.com".to_string()),
            address_line1: Some("123 Main".to_string()),
            address_line2: None,
            city: Some("Austin".to_string()),
            state: Some("TX".to_string()),
            postal_code: Some("78701".to_string()),
            country: Some("US".to_string()),
            metadata: None,
        }
    }

    #[tokio::test]
    #[serial]
    async fn example_flow_compiles() {
        let pool = match test_pool().await {
            Ok(pool) => pool,
            Err(err) => {
                eprintln!("Skipping example_flow_compiles: {err}");
                return;
            }
        };
        cleanup(&pool).await;
        let req = sample_company_req("Example");
        let data = create_company(&pool, TEST_APP, &req, Uuid::new_v4().to_string())
            .await
            .expect("create failed");
        assert_eq!(data.party.app_id, TEST_APP);
    }
}
