use jsonschema::JSONSchema;
use once_cell::sync::OnceCell;
use serde_json::Value;
use std::{collections::HashMap, fs};

static SCHEMAS: OnceCell<HashMap<String, (Value, JSONSchema)>> = OnceCell::new();

pub fn load_schemas_from_dir(schema_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut map = HashMap::new();
    for entry in fs::read_dir(schema_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path)?;
        let json: Value = serde_json::from_str(&raw)?;
        let compiled = JSONSchema::compile(&json)
            .map_err(|e| format!("Failed to compile schema: {}", e))?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown.json")
            .to_string();
        map.insert(name, (json, compiled));
    }
    SCHEMAS.set(map).map_err(|_| "schemas already loaded")?;
    Ok(())
}

pub fn validate(schema_file: &str, payload: &Value) -> Result<(), String> {
    let schemas = SCHEMAS.get().ok_or("schemas not loaded")?;
    let (_schema_value, schema) = schemas
        .get(schema_file)
        .ok_or_else(|| format!("schema not found: {schema_file}"))?;

    if let Err(errors) = schema.validate(payload) {
        let msgs: Vec<String> = errors.map(|e| e.to_string()).collect();
        return Err(msgs.join("; "));
    }
    Ok(())
}
