use sqlx::PgPool;

use super::models::{CreateTemplate, NotificationTemplate, TemplateVersionSummary};

/// Publish a new template version. Auto-increments version per (tenant_id, template_key).
pub async fn publish_template(
    pool: &PgPool,
    tenant_id: &str,
    input: &CreateTemplate,
    created_by: Option<&str>,
) -> Result<NotificationTemplate, sqlx::Error> {
    let next_version: i32 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(MAX(version), 0) + 1
        FROM notification_templates
        WHERE tenant_id = $1 AND template_key = $2
        "#,
    )
    .bind(tenant_id)
    .bind(&input.template_key)
    .fetch_one(pool)
    .await?;

    let required_vars_json = serde_json::to_value(&input.required_vars)
        .unwrap_or_else(|_| serde_json::Value::Array(vec![]));

    let row = sqlx::query_as::<_, NotificationTemplate>(
        r#"
        INSERT INTO notification_templates
            (tenant_id, template_key, version, channel, subject, body, required_vars, created_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id, tenant_id, template_key, version, channel, subject, body,
                  required_vars, created_at, created_by
        "#,
    )
    .bind(tenant_id)
    .bind(&input.template_key)
    .bind(next_version)
    .bind(&input.channel)
    .bind(&input.subject)
    .bind(&input.body)
    .bind(&required_vars_json)
    .bind(created_by)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Get the latest version of a template for the given tenant and key.
pub async fn get_latest(
    pool: &PgPool,
    tenant_id: &str,
    template_key: &str,
) -> Result<Option<NotificationTemplate>, sqlx::Error> {
    sqlx::query_as::<_, NotificationTemplate>(
        r#"
        SELECT id, tenant_id, template_key, version, channel, subject, body,
               required_vars, created_at, created_by
        FROM notification_templates
        WHERE tenant_id = $1 AND template_key = $2
        ORDER BY version DESC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(template_key)
    .fetch_optional(pool)
    .await
}

/// Get a specific version of a template.
pub async fn get_version(
    pool: &PgPool,
    tenant_id: &str,
    template_key: &str,
    version: i32,
) -> Result<Option<NotificationTemplate>, sqlx::Error> {
    sqlx::query_as::<_, NotificationTemplate>(
        r#"
        SELECT id, tenant_id, template_key, version, channel, subject, body,
               required_vars, created_at, created_by
        FROM notification_templates
        WHERE tenant_id = $1 AND template_key = $2 AND version = $3
        "#,
    )
    .bind(tenant_id)
    .bind(template_key)
    .bind(version)
    .fetch_optional(pool)
    .await
}

/// Get version history summaries for a template.
pub async fn list_versions(
    pool: &PgPool,
    tenant_id: &str,
    template_key: &str,
) -> Result<Vec<TemplateVersionSummary>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i32, chrono::DateTime<chrono::Utc>, Option<String>)>(
        r#"
        SELECT version, created_at, created_by
        FROM notification_templates
        WHERE tenant_id = $1 AND template_key = $2
        ORDER BY version DESC
        "#,
    )
    .bind(tenant_id)
    .bind(template_key)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(version, created_at, created_by)| TemplateVersionSummary {
            version,
            created_at,
            created_by,
        })
        .collect())
}

/// Render a template by substituting `{{var}}` placeholders from payload.
/// Returns (rendered_subject, rendered_body) or an error describing the missing var.
pub fn render_template(
    template: &NotificationTemplate,
    payload: &serde_json::Value,
) -> Result<(String, String), String> {
    let vars = payload
        .as_object()
        .ok_or_else(|| "payload must be a JSON object".to_string())?;

    // Check required vars are present
    if let Some(required) = template.required_vars.as_array() {
        for var in required {
            if let Some(var_name) = var.as_str() {
                if !vars.contains_key(var_name) {
                    return Err(format!("missing required variable: {}", var_name));
                }
                if vars.get(var_name).map_or(true, |v| v.is_null()) {
                    return Err(format!("required variable is null: {}", var_name));
                }
            }
        }
    }

    let subject = substitute(&template.subject, vars)?;
    let body = substitute(&template.body, vars)?;
    Ok((subject, body))
}

fn substitute(
    template_str: &str,
    vars: &serde_json::Map<String, serde_json::Value>,
) -> Result<String, String> {
    let mut result = String::with_capacity(template_str.len());
    let mut rest = template_str;

    while let Some(start) = rest.find("{{") {
        result.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let end = after_open
            .find("}}")
            .ok_or_else(|| "unclosed '{{' in template".to_string())?;
        let var_name = after_open[..end].trim();

        let value = vars
            .get(var_name)
            .ok_or_else(|| format!("missing variable: {}", var_name))?;

        match value {
            serde_json::Value::String(s) => result.push_str(s),
            serde_json::Value::Number(n) => result.push_str(&n.to_string()),
            serde_json::Value::Bool(b) => result.push_str(if *b { "true" } else { "false" }),
            serde_json::Value::Null => {
                return Err(format!("variable '{}' is null", var_name));
            }
            _ => {
                return Err(format!("variable '{}' is not a scalar", var_name));
            }
        }

        rest = &after_open[end + 2..];
    }

    result.push_str(rest);
    Ok(result)
}
