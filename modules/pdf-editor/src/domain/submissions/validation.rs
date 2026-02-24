//! Field-level validation engine for form submissions.
//!
//! Validates field_data (JSONB) against template field definitions and their
//! validation_rules. Deterministic: same input always produces same output.
//!
//! Supported rules per field_type:
//!   text:     required, pattern (regex)
//!   number:   required, min, max
//!   date:     required
//!   dropdown: required, options (allowed values)
//!   checkbox: required (must be boolean)

use crate::domain::forms::FormField;
use serde_json::Value;

/// Validate all fields against the submission's field_data.
/// Returns Ok(()) or a list of all validation errors.
pub fn validate_submission(
    fields: &[FormField],
    field_data: &Value,
) -> Result<(), Vec<String>> {
    let data = field_data.as_object();
    let mut errors = Vec::new();

    for field in fields {
        let rules = &field.validation_rules;
        let value = data.and_then(|d| d.get(&field.field_key));

        // Check required
        let is_required = rules.get("required").and_then(|v| v.as_bool()).unwrap_or(false);
        if is_required && is_empty_value(value) {
            errors.push(format!("'{}' is required", field.field_key));
            continue; // Skip further checks on missing required field
        }

        // Skip further validation if value is absent/null (and not required)
        if is_empty_value(value) {
            continue;
        }

        let value = value.unwrap();

        match field.field_type.as_str() {
            "text" => validate_text(field, rules, value, &mut errors),
            "number" => validate_number(field, rules, value, &mut errors),
            "date" => validate_date(field, value, &mut errors),
            "dropdown" => validate_dropdown(field, rules, value, &mut errors),
            "checkbox" => validate_checkbox(field, value, &mut errors),
            _ => {} // Unknown types pass — validated at field creation
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn is_empty_value(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        _ => false,
    }
}

fn validate_text(
    field: &FormField,
    rules: &Value,
    value: &Value,
    errors: &mut Vec<String>,
) {
    let Some(s) = value.as_str() else {
        errors.push(format!("'{}' must be a string", field.field_key));
        return;
    };

    if let Some(pattern) = rules.get("pattern").and_then(|v| v.as_str()) {
        match regex::Regex::new(pattern) {
            Ok(re) => {
                if !re.is_match(s) {
                    errors.push(format!(
                        "'{}' does not match pattern '{}'",
                        field.field_key, pattern
                    ));
                }
            }
            Err(_) => {
                errors.push(format!(
                    "'{}' has invalid validation pattern '{}'",
                    field.field_key, pattern
                ));
            }
        }
    }
}

fn validate_number(
    field: &FormField,
    rules: &Value,
    value: &Value,
    errors: &mut Vec<String>,
) {
    let Some(n) = value.as_f64() else {
        errors.push(format!("'{}' must be a number", field.field_key));
        return;
    };

    if let Some(min) = rules.get("min").and_then(|v| v.as_f64()) {
        if n < min {
            errors.push(format!("'{}' must be >= {}", field.field_key, min));
        }
    }
    if let Some(max) = rules.get("max").and_then(|v| v.as_f64()) {
        if n > max {
            errors.push(format!("'{}' must be <= {}", field.field_key, max));
        }
    }
}

fn validate_date(
    field: &FormField,
    value: &Value,
    errors: &mut Vec<String>,
) {
    let Some(s) = value.as_str() else {
        errors.push(format!("'{}' must be a date string", field.field_key));
        return;
    };

    // Accept ISO 8601 date format (YYYY-MM-DD)
    if chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_err() {
        errors.push(format!(
            "'{}' must be a valid date (YYYY-MM-DD)",
            field.field_key
        ));
    }
}

fn validate_dropdown(
    field: &FormField,
    rules: &Value,
    value: &Value,
    errors: &mut Vec<String>,
) {
    let Some(s) = value.as_str() else {
        errors.push(format!("'{}' must be a string", field.field_key));
        return;
    };

    if let Some(options) = rules.get("options").and_then(|v| v.as_array()) {
        let allowed: Vec<&str> = options.iter().filter_map(|o| o.as_str()).collect();
        if !allowed.contains(&s) {
            errors.push(format!(
                "'{}' must be one of: {}",
                field.field_key,
                allowed.join(", ")
            ));
        }
    }
}

fn validate_checkbox(
    field: &FormField,
    value: &Value,
    errors: &mut Vec<String>,
) {
    if !value.is_boolean() {
        errors.push(format!("'{}' must be a boolean", field.field_key));
    }
}
