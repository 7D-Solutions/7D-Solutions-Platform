//! Employee repository — CRUD operations against tk_employees.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::{CreateEmployeeRequest, Employee, EmployeeError, UpdateEmployeeRequest};

pub struct EmployeeRepo;

impl EmployeeRepo {
    /// Create a new employee.
    ///
    /// Guard: validates input, then inserts. Returns DuplicateCode on
    /// (app_id, employee_code) unique constraint violation.
    pub async fn create(
        pool: &PgPool,
        req: &CreateEmployeeRequest,
    ) -> Result<Employee, EmployeeError> {
        req.validate()?;

        let currency = req.currency.as_deref().unwrap_or("USD");

        sqlx::query_as::<_, Employee>(
            r#"
            INSERT INTO tk_employees
                (app_id, employee_code, first_name, last_name, email,
                 department, external_payroll_id, hourly_rate_minor, currency)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(&req.app_id)
        .bind(req.employee_code.trim())
        .bind(req.first_name.trim())
        .bind(req.last_name.trim())
        .bind(req.email.as_deref())
        .bind(req.department.as_deref())
        .bind(req.external_payroll_id.as_deref())
        .bind(req.hourly_rate_minor)
        .bind(currency)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return EmployeeError::DuplicateCode(
                        req.employee_code.clone(),
                        req.app_id.clone(),
                    );
                }
            }
            EmployeeError::Database(e)
        })
    }

    /// Update mutable fields of an existing employee.
    /// Only fields present in the request are updated.
    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateEmployeeRequest,
    ) -> Result<Employee, EmployeeError> {
        req.validate()?;

        sqlx::query_as::<_, Employee>(
            r#"
            UPDATE tk_employees
            SET
                first_name          = COALESCE($3, first_name),
                last_name           = COALESCE($4, last_name),
                email               = CASE WHEN $5::TEXT IS NOT NULL THEN $5 ELSE email END,
                department          = CASE WHEN $6::TEXT IS NOT NULL THEN $6 ELSE department END,
                external_payroll_id = CASE WHEN $7::TEXT IS NOT NULL THEN $7 ELSE external_payroll_id END,
                hourly_rate_minor   = CASE WHEN $8::BIGINT IS NOT NULL THEN $8 ELSE hourly_rate_minor END,
                currency            = COALESCE($9, currency),
                updated_at          = NOW()
            WHERE id = $1 AND app_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.app_id)
        .bind(req.first_name.as_deref())
        .bind(req.last_name.as_deref())
        .bind(req.email.as_deref())
        .bind(req.department.as_deref())
        .bind(req.external_payroll_id.as_deref())
        .bind(req.hourly_rate_minor)
        .bind(req.currency.as_deref())
        .fetch_optional(pool)
        .await?
        .ok_or(EmployeeError::NotFound)
    }

    /// Deactivate an employee (soft delete). Idempotent.
    pub async fn deactivate(
        pool: &PgPool,
        id: Uuid,
        app_id: &str,
    ) -> Result<Employee, EmployeeError> {
        sqlx::query_as::<_, Employee>(
            r#"
            UPDATE tk_employees
            SET active = FALSE, updated_at = NOW()
            WHERE id = $1 AND app_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(app_id)
        .fetch_optional(pool)
        .await?
        .ok_or(EmployeeError::NotFound)
    }

    /// Fetch an employee by id, scoped to app_id.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        app_id: &str,
    ) -> Result<Option<Employee>, EmployeeError> {
        sqlx::query_as::<_, Employee>(
            "SELECT * FROM tk_employees WHERE id = $1 AND app_id = $2",
        )
        .bind(id)
        .bind(app_id)
        .fetch_optional(pool)
        .await
        .map_err(EmployeeError::Database)
    }

    /// List employees for a tenant, ordered by last_name, first_name.
    /// Optionally filter by active status.
    pub async fn list(
        pool: &PgPool,
        app_id: &str,
        active_only: bool,
    ) -> Result<Vec<Employee>, EmployeeError> {
        if active_only {
            sqlx::query_as::<_, Employee>(
                r#"
                SELECT * FROM tk_employees
                WHERE app_id = $1 AND active = TRUE
                ORDER BY last_name, first_name
                "#,
            )
            .bind(app_id)
            .fetch_all(pool)
            .await
            .map_err(EmployeeError::Database)
        } else {
            sqlx::query_as::<_, Employee>(
                r#"
                SELECT * FROM tk_employees
                WHERE app_id = $1
                ORDER BY last_name, first_name
                "#,
            )
            .bind(app_id)
            .fetch_all(pool)
            .await
            .map_err(EmployeeError::Database)
        }
    }
}
