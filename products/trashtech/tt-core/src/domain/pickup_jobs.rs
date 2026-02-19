use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PickupJob {
    pub id: Uuid,
    pub customer_party_id: Uuid,
    pub ar_customer_id: i32,
    pub status: String,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub driver_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreatePickupJob {
    pub customer_party_id: Uuid,
    pub ar_customer_id: i32,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub driver_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub notes: Option<String>,
}

pub struct PickupJobRepo;

impl PickupJobRepo {
    pub async fn create(pool: &PgPool, input: &CreatePickupJob) -> Result<PickupJob, sqlx::Error> {
        sqlx::query_as::<_, PickupJob>(
            r#"INSERT INTO pickup_jobs
                (customer_party_id, ar_customer_id, scheduled_at, driver_id, route_id, notes)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING *"#,
        )
        .bind(input.customer_party_id)
        .bind(input.ar_customer_id)
        .bind(input.scheduled_at)
        .bind(input.driver_id)
        .bind(input.route_id)
        .bind(&input.notes)
        .fetch_one(pool)
        .await
    }

    pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<Option<PickupJob>, sqlx::Error> {
        sqlx::query_as::<_, PickupJob>("SELECT * FROM pickup_jobs WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn list(pool: &PgPool, limit: i64, offset: i64) -> Result<Vec<PickupJob>, sqlx::Error> {
        sqlx::query_as::<_, PickupJob>(
            "SELECT * FROM pickup_jobs ORDER BY created_at DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    }

    pub async fn update_status(
        pool: &PgPool,
        id: Uuid,
        status: &str,
    ) -> Result<Option<PickupJob>, sqlx::Error> {
        sqlx::query_as::<_, PickupJob>(
            r#"UPDATE pickup_jobs SET status = $1, updated_at = NOW()
               WHERE id = $2 RETURNING *"#,
        )
        .bind(status)
        .bind(id)
        .fetch_optional(pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickup_job_fields_match_schema() {
        let job = PickupJob {
            id: Uuid::new_v4(),
            customer_party_id: Uuid::new_v4(),
            ar_customer_id: 42,
            status: "pending".to_string(),
            scheduled_at: Some(Utc::now()),
            completed_at: None,
            driver_id: Some(Uuid::new_v4()),
            route_id: None,
            notes: Some("test note".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(job.status, "pending");
        assert_eq!(job.ar_customer_id, 42);
    }

    #[test]
    fn create_input_serializes() {
        let input = CreatePickupJob {
            customer_party_id: Uuid::new_v4(),
            ar_customer_id: 1,
            scheduled_at: None,
            driver_id: None,
            route_id: None,
            notes: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("customer_party_id"));
        assert!(json.contains("ar_customer_id"));
    }
}
