//! Deterministic JSON payload generation for payroll export.
//!
//! Produces a structured JSON object with metadata + line items.
//! Keys are sorted for deterministic serialization.

use chrono::NaiveDate;
use serde_json::{json, Value};

use super::models::ExportEntry;

/// Generate a deterministic JSON export payload.
pub fn generate(
    app_id: &str,
    export_type: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
    entries: &[ExportEntry],
) -> Value {
    let total_minutes: i64 = entries.iter().map(|e| e.minutes as i64).sum();
    let total_hours = format!("{:.2}", total_minutes as f64 / 60.0);

    let line_items: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "entry_id": e.entry_id,
                "employee_id": e.employee_id,
                "employee_name": e.employee_name,
                "project_id": e.project_id,
                "project_name": e.project_name,
                "task_id": e.task_id,
                "work_date": e.work_date.to_string(),
                "minutes": e.minutes,
                "hours": format!("{:.2}", e.minutes as f64 / 60.0),
                "description": e.description,
            })
        })
        .collect();

    json!({
        "app_id": app_id,
        "export_type": export_type,
        "period_start": period_start.to_string(),
        "period_end": period_end.to_string(),
        "record_count": entries.len(),
        "total_minutes": total_minutes,
        "total_hours": total_hours,
        "line_items": line_items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn sample_entries() -> Vec<ExportEntry> {
        vec![ExportEntry {
            entry_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            employee_id: Uuid::parse_str("00000000-0000-0000-0000-000000000010").unwrap(),
            employee_name: "Alice".into(),
            project_id: None,
            project_name: None,
            task_id: None,
            work_date: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            minutes: 480,
            description: Some("Dev".into()),
        }]
    }

    #[test]
    fn json_has_required_fields() {
        let payload = generate(
            "acme",
            "payroll",
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 2, 14).unwrap(),
            &sample_entries(),
        );
        assert_eq!(payload["app_id"], "acme");
        assert_eq!(payload["export_type"], "payroll");
        assert_eq!(payload["record_count"], 1);
        assert_eq!(payload["total_minutes"], 480);
        assert!(payload["line_items"].is_array());
    }

    #[test]
    fn json_deterministic() {
        let a = generate(
            "acme",
            "payroll",
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 2, 14).unwrap(),
            &sample_entries(),
        );
        let b = generate(
            "acme",
            "payroll",
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 2, 14).unwrap(),
            &sample_entries(),
        );
        // serde_json::to_string sorts keys in Map (BTreeMap)
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn json_empty_entries() {
        let payload = generate(
            "acme",
            "payroll",
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 2, 14).unwrap(),
            &[],
        );
        assert_eq!(payload["record_count"], 0);
        assert_eq!(payload["total_minutes"], 0);
        assert_eq!(payload["line_items"].as_array().unwrap().len(), 0);
    }
}
