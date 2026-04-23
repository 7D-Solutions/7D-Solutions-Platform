use jsonschema::JSONSchema;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
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

    JSONSchema::compile(&schema).map_err(|e| ContractError::SchemaError(e.to_string()))
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
            let error_messages: Vec<String> = errors.map(|e| format!("  - {}", e)).collect();
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

/// Parse and validate an OpenAPI JSON file
pub fn validate_openapi_spec_json(path: &Path) -> Result<Value, ContractError> {
    let contents = fs::read_to_string(path)?;
    let spec: Value = serde_json::from_str(&contents)?;
    Ok(spec)
}

/// Check that no schema in an OpenAPI spec is an empty object `{}`
pub fn check_no_empty_schemas(spec: &Value, spec_name: &str) -> Result<(), ContractError> {
    let schemas = spec
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_object());

    if let Some(schemas) = schemas {
        let empty: Vec<&str> = schemas
            .iter()
            .filter(|(_, v)| v.as_object().map(|o| o.is_empty()).unwrap_or(false))
            .map(|(k, _)| k.as_str())
            .collect();

        if !empty.is_empty() {
            return Err(ContractError::ValidationError(format!(
                "OpenAPI spec '{}' has empty schemas: {:?}",
                spec_name, empty
            )));
        }
    }

    Ok(())
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

/// A single property entry in a consumer schema file
#[derive(Deserialize)]
pub struct ConsumerProperty {
    #[serde(rename = "type")]
    pub type_: String,
}

/// Deserialised consumer contract — one JSON file per endpoint expectation
#[derive(Deserialize)]
pub struct ConsumerSchema {
    pub endpoint: String,
    pub method: String,
    pub required: Vec<String>,
    pub properties: HashMap<String, ConsumerProperty>,
}

/// Validate a consumer's declared expectations against a loaded platform OpenAPI spec.
///
/// Checks:
/// 1. The endpoint+method exists and has a 200 JSON response schema.
/// 2. Every field in `consumer.required` is present in the resolved spec schema's `properties`.
/// 3. Every field in `consumer.properties` has the same `type` string in the spec (ignoring
///    `nullable`). Returns `Err` if the spec uses a nested `$ref` instead of a `type` keyword.
pub fn validate_consumer_schema(
    platform_spec: &Value,
    consumer: &ConsumerSchema,
) -> Result<(), ContractError> {
    let response_schema = platform_spec
        .get("paths")
        .and_then(|p| p.get(&consumer.endpoint))
        .and_then(|e| e.get(&consumer.method))
        .and_then(|m| m.get("responses"))
        .and_then(|r| r.get("200"))
        .and_then(|r| r.get("content"))
        .and_then(|c| c.get("application/json"))
        .and_then(|j| j.get("schema"))
        .ok_or_else(|| {
            ContractError::ValidationError(format!(
                "Cannot find 200 response schema for {} {}",
                consumer.method, consumer.endpoint
            ))
        })?;

    let resolved_schema =
        if let Some(ref_str) = response_schema.get("$ref").and_then(|v| v.as_str()) {
            let schema_name = ref_str
                .strip_prefix("#/components/schemas/")
                .ok_or_else(|| {
                    ContractError::ValidationError(format!("Unsupported $ref format: {}", ref_str))
                })?;
            platform_spec
                .get("components")
                .and_then(|c| c.get("schemas"))
                .and_then(|s| s.get(schema_name))
                .ok_or_else(|| {
                    ContractError::ValidationError(format!("Cannot resolve $ref: {}", ref_str))
                })?
        } else {
            response_schema
        };

    let spec_properties = resolved_schema
        .get("properties")
        .and_then(|p| p.as_object())
        .ok_or_else(|| {
            ContractError::ValidationError("Resolved schema has no 'properties' object".to_string())
        })?;

    for field in &consumer.required {
        if !spec_properties.contains_key(field.as_str()) {
            return Err(ContractError::ValidationError(format!(
                "Required field '{}' not found in spec schema for {} {}",
                field, consumer.method, consumer.endpoint
            )));
        }
    }

    for (field, consumer_prop) in &consumer.properties {
        if let Some(spec_prop) = spec_properties.get(field.as_str()) {
            if spec_prop.get("$ref").is_some() {
                return Err(ContractError::ValidationError(format!(
                    "Field '{}' in spec uses $ref which is unsupported for type checking",
                    field
                )));
            }
            let spec_type = spec_prop
                .get("type")
                .and_then(|t| t.as_str())
                .ok_or_else(|| {
                    ContractError::ValidationError(format!(
                        "Field '{}' in spec has no 'type' keyword",
                        field
                    ))
                })?;
            if spec_type != consumer_prop.type_ {
                return Err(ContractError::ValidationError(format!(
                    "Field '{}' type mismatch: consumer expects '{}', spec has '{}'",
                    field, consumer_prop.type_, spec_type
                )));
            }
        }
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
