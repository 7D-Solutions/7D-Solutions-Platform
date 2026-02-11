use jsonschema::JSONSchema;
use serde_json::Value;
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("Failed to read file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse JSON: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Failed to parse YAML: {0}")]
    YamlError(#[from] serde_yaml::Error),

    #[error("Schema compilation failed: {0}")]
    SchemaError(String),

    #[error("Validation failed: {0}")]
    ValidationError(String),
}

/// Load a JSON schema from file
pub fn load_schema(path: &Path) -> Result<JSONSchema, ContractError> {
    let contents = fs::read_to_string(path)?;
    let schema: Value = serde_json::from_str(&contents)?;

    JSONSchema::compile(&schema)
        .map_err(|e| ContractError::SchemaError(e.to_string()))
}

/// Load a JSON example from file
pub fn load_example(path: &Path) -> Result<Value, ContractError> {
    let contents = fs::read_to_string(path)?;
    let example: Value = serde_json::from_str(&contents)?;
    Ok(example)
}

/// Validate an example against a schema
pub fn validate_example(
    schema: &JSONSchema,
    example: &Value,
    example_name: &str,
) -> Result<(), ContractError> {
    match schema.validate(example) {
        Ok(_) => Ok(()),
        Err(errors) => {
            let error_messages: Vec<String> = errors
                .map(|e| format!("  - {}", e))
                .collect();
            Err(ContractError::ValidationError(format!(
                "Example '{}' failed validation:\n{}",
                example_name,
                error_messages.join("\n")
            )))
        }
    }
}

/// Validate all examples against their corresponding schemas
pub fn validate_event_contracts(
    contracts_dir: &Path,
) -> Result<Vec<(String, String)>, ContractError> {
    let schemas_dir = contracts_dir.join("events");
    let examples_dir = schemas_dir.join("examples");

    let mut validated = Vec::new();

    // Find all schema files
    for entry in fs::read_dir(&schemas_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Skip if not a JSON file or if it's in the examples directory
        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let filename = path.file_name().unwrap().to_str().unwrap();

        // Skip non-schema files (e.g., README.json if it exists)
        if !filename.contains(".v") {
            continue;
        }

        // Find corresponding example file
        let example_filename = format!("{}.example.json", filename.trim_end_matches(".json"));
        let example_path = examples_dir.join(&example_filename);

        if !example_path.exists() {
            eprintln!("Warning: No example found for schema: {}", filename);
            continue;
        }

        // Load and validate
        let schema = load_schema(&path)?;
        let example = load_example(&example_path)?;
        validate_example(&schema, &example, &example_filename)?;

        validated.push((filename.to_string(), example_filename));
    }

    Ok(validated)
}

/// Parse and validate an OpenAPI YAML file
pub fn validate_openapi_spec(path: &Path) -> Result<Value, ContractError> {
    let contents = fs::read_to_string(path)?;
    let spec: Value = serde_yaml::from_str(&contents)?;
    Ok(spec)
}

/// Check that required paths exist in an OpenAPI spec
pub fn check_required_paths(
    spec: &Value,
    required_paths: &[&str],
    spec_name: &str,
) -> Result<(), ContractError> {
    let paths = spec
        .get("paths")
        .and_then(|p| p.as_object())
        .ok_or_else(|| {
            ContractError::ValidationError(format!(
                "OpenAPI spec '{}' missing 'paths' object",
                spec_name
            ))
        })?;

    let mut missing = Vec::new();
    for required_path in required_paths {
        if !paths.contains_key(*required_path) {
            missing.push(*required_path);
        }
    }

    if !missing.is_empty() {
        return Err(ContractError::ValidationError(format!(
            "OpenAPI spec '{}' missing required paths: {:?}",
            spec_name, missing
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_example_success() {
        let schema_json = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" }
            }
        });

        let schema = JSONSchema::compile(&schema_json).unwrap();

        let example = json!({
            "name": "test"
        });

        assert!(validate_example(&schema, &example, "test").is_ok());
    }

    #[test]
    fn test_validate_example_failure() {
        let schema_json = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" }
            }
        });

        let schema = JSONSchema::compile(&schema_json).unwrap();

        let example = json!({
            "name": 123
        });

        assert!(validate_example(&schema, &example, "test").is_err());
    }
}
