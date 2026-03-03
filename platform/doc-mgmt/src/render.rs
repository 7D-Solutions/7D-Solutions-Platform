use sha2::{Digest, Sha256};

/// Recursively walk the template JSON, replacing `{{key}}` placeholders
/// with values from `input_data`. Produces deterministic output.
pub fn apply_template(
    template: &serde_json::Value,
    input: &serde_json::Value,
) -> serde_json::Value {
    match template {
        serde_json::Value::String(s) => {
            let mut result = s.clone();
            if let serde_json::Value::Object(map) = input {
                for (key, val) in map {
                    let placeholder = format!("{{{{{}}}}}", key);
                    let replacement = match val {
                        serde_json::Value::String(v) => v.clone(),
                        other => other.to_string(),
                    };
                    result = result.replace(&placeholder, &replacement);
                }
            }
            serde_json::Value::String(result)
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), apply_template(v, input));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| apply_template(v, input)).collect())
        }
        other => other.clone(),
    }
}

/// Compute SHA-256 hash of canonicalized JSON.
pub fn compute_hash(value: &serde_json::Value) -> String {
    let canonical = canonical_json(value);
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Produce canonical JSON (sorted keys) for deterministic hashing.
fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let entries: Vec<String> = keys
                .iter()
                .map(|k| {
                    let key_str = serde_json::to_string(k).unwrap_or_else(|_| format!("\"{}\"", k));
                    format!("{}:{}", key_str, canonical_json(&map[*k]))
                })
                .collect();
            format!("{{{}}}", entries.join(","))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", items.join(","))
        }
        // serde_json::to_string on primitives (null, bool, number, string) is infallible
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_template_replaces_placeholders() {
        let template = serde_json::json!({
            "greeting": "Hello, {{name}}!",
            "details": {
                "order": "Order #{{order_id}}",
                "items": ["{{item1}}", "{{item2}}"]
            }
        });
        let input = serde_json::json!({
            "name": "Alice",
            "order_id": "12345",
            "item1": "Widget",
            "item2": "Gadget"
        });

        let result = apply_template(&template, &input);

        assert_eq!(result["greeting"], "Hello, Alice!");
        assert_eq!(result["details"]["order"], "Order #12345");
        assert_eq!(result["details"]["items"][0], "Widget");
        assert_eq!(result["details"]["items"][1], "Gadget");
    }

    #[test]
    fn apply_template_preserves_non_string_values() {
        let template = serde_json::json!({
            "label": "Count: {{count}}",
            "flag": true,
            "number": 42
        });
        let input = serde_json::json!({"count": "5"});

        let result = apply_template(&template, &input);

        assert_eq!(result["label"], "Count: 5");
        assert_eq!(result["flag"], true);
        assert_eq!(result["number"], 42);
    }

    #[test]
    fn compute_hash_is_deterministic() {
        let v1 = serde_json::json!({"b": 2, "a": 1});
        let v2 = serde_json::json!({"a": 1, "b": 2});
        assert_eq!(compute_hash(&v1), compute_hash(&v2));
    }

    #[test]
    fn compute_hash_differs_for_different_values() {
        let v1 = serde_json::json!({"a": 1});
        let v2 = serde_json::json!({"a": 2});
        assert_ne!(compute_hash(&v1), compute_hash(&v2));
    }

    #[test]
    fn canonical_json_sorts_keys() {
        let v = serde_json::json!({"z": 1, "a": 2, "m": 3});
        let result = canonical_json(&v);
        assert_eq!(result, r#"{"a":2,"m":3,"z":1}"#);
    }
}
