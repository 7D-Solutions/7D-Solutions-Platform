use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GpsPing {
    pub id: Uuid,
    pub driver_id: Uuid,
    pub route_id: Option<Uuid>,
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_meters: Option<f64>,
    pub recorded_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateGpsPing {
    pub driver_id: Uuid,
    pub route_id: Option<Uuid>,
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_meters: Option<f64>,
    pub recorded_at: DateTime<Utc>,
}

pub struct GpsPingRepo;

impl GpsPingRepo {
    pub async fn create(pool: &PgPool, input: &CreateGpsPing) -> Result<GpsPing, sqlx::Error> {
        sqlx::query_as::<_, GpsPing>(
            r#"INSERT INTO gps_pings
                (driver_id, route_id, latitude, longitude, accuracy_meters, recorded_at)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING *"#,
        )
        .bind(input.driver_id)
        .bind(input.route_id)
        .bind(input.latitude)
        .bind(input.longitude)
        .bind(input.accuracy_meters)
        .bind(input.recorded_at)
        .fetch_one(pool)
        .await
    }

    pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<Option<GpsPing>, sqlx::Error> {
        sqlx::query_as::<_, GpsPing>("SELECT * FROM gps_pings WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_for_driver(
        pool: &PgPool,
        driver_id: Uuid,
        limit: i64,
    ) -> Result<Vec<GpsPing>, sqlx::Error> {
        sqlx::query_as::<_, GpsPing>(
            "SELECT * FROM gps_pings WHERE driver_id = $1 ORDER BY recorded_at DESC LIMIT $2",
        )
        .bind(driver_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }

    pub async fn list_for_route(
        pool: &PgPool,
        route_id: Uuid,
    ) -> Result<Vec<GpsPing>, sqlx::Error> {
        sqlx::query_as::<_, GpsPing>(
            "SELECT * FROM gps_pings WHERE route_id = $1 ORDER BY recorded_at",
        )
        .bind(route_id)
        .fetch_all(pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_gps_ping_serializes() {
        let input = CreateGpsPing {
            driver_id: Uuid::new_v4(),
            route_id: None,
            latitude: 47.6062,
            longitude: -122.3321,
            accuracy_meters: Some(5.0),
            recorded_at: Utc::now(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("latitude"));
        assert!(json.contains("longitude"));
        assert!(json.contains("driver_id"));
    }

    #[test]
    fn gps_ping_optional_fields() {
        let input = CreateGpsPing {
            driver_id: Uuid::new_v4(),
            route_id: None,
            latitude: 0.0,
            longitude: 0.0,
            accuracy_meters: None,
            recorded_at: Utc::now(),
        };
        assert!(input.route_id.is_none());
        assert!(input.accuracy_meters.is_none());
    }
}
