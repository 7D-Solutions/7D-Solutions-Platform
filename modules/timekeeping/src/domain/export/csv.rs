//! Deterministic CSV generation from export entries.
//!
//! Rows are sorted by (work_date, employee_id, entry_id) for stable output.
//! The same input always produces the same CSV byte-for-byte.

use super::models::ExportEntry;

const CSV_HEADER: &str = "entry_id,employee_id,employee_name,project_id,\
project_name,task_id,work_date,minutes,hours,description";

/// Generate a deterministic CSV string from sorted export entries.
pub fn generate(entries: &[ExportEntry]) -> String {
    let mut buf = String::with_capacity(entries.len() * 120 + CSV_HEADER.len() + 2);
    buf.push_str(CSV_HEADER);
    buf.push('\n');

    for e in entries {
        let hours = format!("{:.2}", e.minutes as f64 / 60.0);
        buf.push_str(&e.entry_id.to_string());
        buf.push(',');
        buf.push_str(&e.employee_id.to_string());
        buf.push(',');
        push_csv_field(&mut buf, &e.employee_name);
        buf.push(',');
        buf.push_str(&opt_uuid(e.project_id));
        buf.push(',');
        push_csv_field(&mut buf, e.project_name.as_deref().unwrap_or(""));
        buf.push(',');
        buf.push_str(&opt_uuid(e.task_id));
        buf.push(',');
        buf.push_str(&e.work_date.to_string());
        buf.push(',');
        buf.push_str(&e.minutes.to_string());
        buf.push(',');
        buf.push_str(&hours);
        buf.push(',');
        push_csv_field(&mut buf, e.description.as_deref().unwrap_or(""));
        buf.push('\n');
    }

    buf
}

fn opt_uuid(u: Option<uuid::Uuid>) -> String {
    u.map(|v| v.to_string()).unwrap_or_default()
}

fn push_csv_field(buf: &mut String, val: &str) {
    if val.contains(',') || val.contains('"') || val.contains('\n') {
        buf.push('"');
        buf.push_str(&val.replace('"', "\"\""));
        buf.push('"');
    } else {
        buf.push_str(val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use uuid::Uuid;

    fn sample_entries() -> Vec<ExportEntry> {
        vec![
            ExportEntry {
                entry_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                employee_id: Uuid::parse_str("00000000-0000-0000-0000-000000000010").unwrap(),
                employee_name: "Alice Smith".into(),
                project_id: Some(
                    Uuid::parse_str("00000000-0000-0000-0000-000000000100").unwrap(),
                ),
                project_name: Some("Project Alpha".into()),
                task_id: None,
                work_date: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
                minutes: 480,
                description: Some("Development work".into()),
            },
            ExportEntry {
                entry_id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                employee_id: Uuid::parse_str("00000000-0000-0000-0000-000000000010").unwrap(),
                employee_name: "Alice Smith".into(),
                project_id: None,
                project_name: None,
                task_id: None,
                work_date: NaiveDate::from_ymd_opt(2026, 2, 2).unwrap(),
                minutes: 60,
                description: Some("Meeting, with \"quotes\"".into()),
            },
        ]
    }

    #[test]
    fn csv_header_present() {
        let csv = generate(&sample_entries());
        assert!(csv.starts_with("entry_id,"));
    }

    #[test]
    fn csv_row_count() {
        let csv = generate(&sample_entries());
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3); // header + 2 rows
    }

    #[test]
    fn csv_deterministic() {
        let a = generate(&sample_entries());
        let b = generate(&sample_entries());
        assert_eq!(a, b);
    }

    #[test]
    fn csv_escapes_quotes_and_commas() {
        let csv = generate(&sample_entries());
        // The description with quotes should be escaped
        assert!(csv.contains("\"Meeting, with \"\"quotes\"\"\""));
    }

    #[test]
    fn csv_empty_entries() {
        let csv = generate(&[]);
        assert_eq!(csv.lines().count(), 1); // header only
    }
}
