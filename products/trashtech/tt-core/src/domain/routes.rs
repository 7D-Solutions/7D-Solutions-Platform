use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Route {
    pub id: Uuid,
    pub app_id: String,
    pub name: String,
    pub date: NaiveDate,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRoute {
    pub app_id: String,
    pub name: String,
    pub date: NaiveDate,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RouteStop {
    pub id: Uuid,
    pub route_id: Uuid,
    pub pickup_job_id: Uuid,
    pub sequence_num: i32,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRouteStop {
    pub route_id: Uuid,
    pub pickup_job_id: Uuid,
    pub sequence_num: i32,
}

pub struct RouteRepo;

impl RouteRepo {
    pub async fn create(pool: &PgPool, input: &CreateRoute) -> Result<Route, sqlx::Error> {
        sqlx::query_as::<_, Route>(
            r#"INSERT INTO routes (app_id, name, date)
               VALUES ($1, $2, $3)
               RETURNING *"#,
        )
        .bind(&input.app_id)
        .bind(&input.name)
        .bind(input.date)
        .fetch_one(pool)
        .await
    }

    pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Route>, sqlx::Error> {
        sqlx::query_as::<_, Route>("SELECT * FROM routes WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn list(pool: &PgPool, limit: i64, offset: i64) -> Result<Vec<Route>, sqlx::Error> {
        sqlx::query_as::<_, Route>(
            "SELECT * FROM routes ORDER BY date DESC, created_at DESC LIMIT $1 OFFSET $2",
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
    ) -> Result<Option<Route>, sqlx::Error> {
        sqlx::query_as::<_, Route>(
            "UPDATE routes SET status = $1 WHERE id = $2 RETURNING *",
        )
        .bind(status)
        .bind(id)
        .fetch_optional(pool)
        .await
    }
}

pub struct RouteStopRepo;

impl RouteStopRepo {
    pub async fn create(pool: &PgPool, input: &CreateRouteStop) -> Result<RouteStop, sqlx::Error> {
        sqlx::query_as::<_, RouteStop>(
            r#"INSERT INTO route_stops (route_id, pickup_job_id, sequence_num)
               VALUES ($1, $2, $3)
               RETURNING *"#,
        )
        .bind(input.route_id)
        .bind(input.pickup_job_id)
        .bind(input.sequence_num)
        .fetch_one(pool)
        .await
    }

    pub async fn list_for_route(
        pool: &PgPool,
        route_id: Uuid,
    ) -> Result<Vec<RouteStop>, sqlx::Error> {
        sqlx::query_as::<_, RouteStop>(
            "SELECT * FROM route_stops WHERE route_id = $1 ORDER BY sequence_num",
        )
        .bind(route_id)
        .fetch_all(pool)
        .await
    }

    pub async fn update_status(
        pool: &PgPool,
        id: Uuid,
        status: &str,
    ) -> Result<Option<RouteStop>, sqlx::Error> {
        sqlx::query_as::<_, RouteStop>(
            "UPDATE route_stops SET status = $1 WHERE id = $2 RETURNING *",
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
    fn route_fields_match_schema() {
        let route = Route {
            id: Uuid::new_v4(),
            app_id: "trashtech".to_string(),
            name: "North Route".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 2, 19).unwrap(),
            status: "planned".to_string(),
            created_at: Utc::now(),
        };
        assert_eq!(route.status, "planned");
        assert_eq!(route.app_id, "trashtech");
    }

    #[test]
    fn route_stop_fields_match_schema() {
        let stop = RouteStop {
            id: Uuid::new_v4(),
            route_id: Uuid::new_v4(),
            pickup_job_id: Uuid::new_v4(),
            sequence_num: 1,
            status: "pending".to_string(),
        };
        assert_eq!(stop.sequence_num, 1);
        assert_eq!(stop.status, "pending");
    }

    #[test]
    fn create_route_serializes() {
        let input = CreateRoute {
            app_id: "trashtech".to_string(),
            name: "East Route".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("app_id"));
        assert!(json.contains("East Route"));
    }
}
