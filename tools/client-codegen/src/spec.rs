//! Parse OpenAPI 3.x JSON into an intermediate representation.

use serde_json::Value;
use std::collections::BTreeMap;

pub struct ParsedSpec {
    pub service_title: String,
    pub crate_name: String,
    pub types: Vec<TypeDef>,
    pub endpoints: Vec<Endpoint>,
}

pub enum TypeDef {
    Struct {
        name: String,
        fields: Vec<Field>,
        doc: Option<String>,
    },
    Enum {
        name: String,
        variants: Vec<String>,
        doc: Option<String>,
    },
}

pub struct Field {
    pub name: String,
    pub rust_type: String,
    pub required: bool,
    pub doc: Option<String>,
}

pub struct Endpoint {
    pub method: HttpMethod,
    pub path: String,
    pub operation_id: String,
    pub tag: String,
    pub path_params: Vec<Param>,
    pub query_params: Vec<Param>,
    pub request_body: Option<String>,
    pub response: ResponseKind,
}

#[derive(Clone)]
pub struct Param {
    pub name: String,
    pub rust_type: String,
    pub required: bool,
}

pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

pub enum ResponseKind {
    Json(String),
    Empty,
}

impl ParsedSpec {
    pub fn from_openapi(doc: &Value) -> Self {
        let title = doc["info"]["title"].as_str().unwrap_or("Unknown");
        let crate_name = title_to_crate_name(title);

        let mut types = Vec::new();
        let mut wrapper_names: Vec<String> = Vec::new();

        // Parse schemas
        if let Some(schemas) = doc["components"]["schemas"].as_object() {
            for (name, schema) in schemas {
                // Detect PaginatedResponse_X / DataResponse_X wrappers
                if name.starts_with("PaginatedResponse_") || name.starts_with("DataResponse_") {
                    wrapper_names.push(name.clone());
                    continue;
                }
                // Skip well-known types we don't generate
                if name == "ApiError" || name == "FieldError" || name == "PaginationMeta"
                    || name == "BTreeMap" || name == "HashMap"
                {
                    continue;
                }
                if let Some(td) = parse_type_def(name, schema) {
                    types.push(td);
                }
            }
        }

        // Parse endpoints
        let mut endpoints = Vec::new();
        if let Some(paths) = doc["paths"].as_object() {
            for (path, methods) in paths {
                if let Some(obj) = methods.as_object() {
                    for (method_str, detail) in obj {
                        if let Some(ep) = parse_endpoint(method_str, path, detail) {
                            endpoints.push(ep);
                        }
                    }
                }
            }
        }

        ParsedSpec {
            service_title: title.to_string(),
            crate_name,
            types,
            endpoints,
        }
    }

    /// Check if a schema name is a wrapper type the generator handles generically.
    #[allow(dead_code)]
    pub fn is_wrapper_schema(name: &str) -> bool {
        name.starts_with("PaginatedResponse_")
            || name.starts_with("DataResponse_")
            || name == "PaginationMeta"
            || name == "ApiError"
            || name == "FieldError"
    }
}

fn title_to_crate_name(title: &str) -> String {
    let module: String = title
        .to_lowercase()
        .replace("service", "")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    let module = module.trim_matches('-');
    format!("platform-client-{module}")
}

fn parse_type_def(name: &str, schema: &Value) -> Option<TypeDef> {
    let doc = schema["description"].as_str().map(|s| s.to_string());

    // Enum
    if let Some(variants) = schema["enum"].as_array() {
        let vs: Vec<String> = variants
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        return Some(TypeDef::Enum {
            name: name.to_string(),
            variants: vs,
            doc,
        });
    }

    // allOf — flatten all sub-schemas into one struct
    if let Some(all_of) = schema["allOf"].as_array() {
        let mut fields = BTreeMap::new();
        let mut required: Vec<String> = Vec::new();
        for sub in all_of {
            if let Some(r) = sub.get("$ref").and_then(|r| r.as_str()) {
                // Flatten referenced schema — we can't resolve it here,
                // so emit a flattened serde field
                let ref_name = ref_to_type(r);
                fields.insert(
                    format!("_base_{}", ref_name.to_lowercase()),
                    Field {
                        name: format!("_base_{}", ref_name.to_lowercase()),
                        rust_type: format!("#[serde(flatten)] {ref_name}"),
                        required: true,
                        doc: None,
                    },
                );
            }
            if let Some(props) = sub["properties"].as_object() {
                let sub_req = collect_required(sub);
                required.extend(sub_req);
                for (fname, fschema) in props {
                    let rtype = schema_to_rust_type(fschema);
                    fields.insert(
                        fname.clone(),
                        Field {
                            name: fname.clone(),
                            rust_type: rtype,
                            required: true, // set below
                            doc: fschema["description"]
                                .as_str()
                                .map(|s| s.to_string()),
                        },
                    );
                }
            }
        }
        // Mark optional fields
        for f in fields.values_mut() {
            if !f.name.starts_with("_base_") {
                f.required = required.contains(&f.name);
            }
        }
        let field_list: Vec<Field> = fields.into_values().collect();
        return Some(TypeDef::Struct {
            name: name.to_string(),
            fields: field_list,
            doc,
        });
    }

    // anyOf / oneOf at top level → opaque Value
    if schema.get("anyOf").is_some() || schema.get("oneOf").is_some() {
        return Some(TypeDef::Struct {
            name: name.to_string(),
            fields: vec![Field {
                name: "value".to_string(),
                rust_type: "#[serde(flatten)] serde_json::Value".to_string(),
                required: true,
                doc: None,
            }],
            doc,
        });
    }

    // Object with properties
    if let Some(props) = schema["properties"].as_object() {
        let required_names = collect_required(schema);
        let mut fields = Vec::new();
        for (fname, fschema) in props {
            let rtype = schema_to_rust_type(fschema);
            fields.push(Field {
                name: fname.clone(),
                rust_type: rtype,
                required: required_names.contains(fname),
                doc: fschema["description"].as_str().map(|s| s.to_string()),
            });
        }
        return Some(TypeDef::Struct {
            name: name.to_string(),
            fields,
            doc,
        });
    }

    None
}

fn collect_required(schema: &Value) -> Vec<String> {
    schema["required"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn schema_to_rust_type(schema: &Value) -> String {
    // $ref
    if let Some(r) = schema.get("$ref").and_then(|v| v.as_str()) {
        return resolve_ref_type(r);
    }

    // oneOf with null (nullable reference)
    if let Some(one_of) = schema["oneOf"].as_array() {
        let non_null: Vec<&Value> = one_of
            .iter()
            .filter(|v| v["type"].as_str() != Some("null"))
            .collect();
        if non_null.len() == 1 && one_of.len() == 2 {
            let inner = schema_to_rust_type(non_null[0]);
            return format!("Option<{inner}>");
        }
        return "serde_json::Value".to_string();
    }

    // Type array with null (nullable primitive)
    if let Some(arr) = schema["type"].as_array() {
        let types: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        let non_null: Vec<&&str> = types.iter().filter(|t| **t != "null").collect();
        if non_null.len() == 1 {
            let inner = primitive_type(non_null[0], schema);
            return format!("Option<{inner}>");
        }
        return "serde_json::Value".to_string();
    }

    // Single type
    if let Some(t) = schema["type"].as_str() {
        return match t {
            "array" => {
                let items_type = schema
                    .get("items")
                    .map(|i| schema_to_rust_type(i))
                    .unwrap_or_else(|| "serde_json::Value".to_string());
                format!("Vec<{items_type}>")
            }
            _ => primitive_type(t, schema),
        };
    }

    // No type info — catch-all
    "serde_json::Value".to_string()
}

fn primitive_type(t: &str, schema: &Value) -> String {
    let fmt = schema["format"].as_str().unwrap_or("");
    match (t, fmt) {
        ("string", "uuid") => "uuid::Uuid".to_string(),
        ("string", "date-time") => "chrono::DateTime<chrono::Utc>".to_string(),
        ("string", "date") => "chrono::NaiveDate".to_string(),
        ("string", _) => "String".to_string(),
        ("integer", "int32") => "i32".to_string(),
        ("integer", "int64") => "i64".to_string(),
        ("integer", _) => "i64".to_string(),
        ("number", "float") => "f32".to_string(),
        ("number", "double") => "f64".to_string(),
        ("number", _) => "f64".to_string(),
        ("boolean", _) => "bool".to_string(),
        _ => "serde_json::Value".to_string(),
    }
}

fn ref_to_type(r: &str) -> String {
    r.rsplit('/').next().unwrap_or("serde_json::Value").to_string()
}

fn resolve_ref_type(r: &str) -> String {
    let name = ref_to_type(r);
    // Map well-known wrappers
    if let Some(inner) = name.strip_prefix("PaginatedResponse_") {
        return format!("PaginatedResponse<{inner}>");
    }
    if let Some(inner) = name.strip_prefix("DataResponse_") {
        return format!("DataResponse<{inner}>");
    }
    if name == "PaginationMeta" || name == "ApiError" || name == "FieldError" {
        return name;
    }
    // Rust std collection types exposed via utoipa — map to serde_json::Value
    if name == "BTreeMap" || name == "HashMap" {
        return "serde_json::Value".to_string();
    }
    name
}

fn parse_endpoint(method_str: &str, path: &str, detail: &Value) -> Option<Endpoint> {
    let method = match method_str {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        _ => return None,
    };

    let operation_id = detail["operationId"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let tag = detail["tags"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();

    let mut path_params = Vec::new();
    let mut query_params = Vec::new();

    if let Some(params) = detail["parameters"].as_array() {
        for p in params {
            let pname = p["name"].as_str().unwrap_or("").to_string();
            let loc = p["in"].as_str().unwrap_or("query");
            let required = p["required"].as_bool().unwrap_or(loc == "path");
            let rust_type = p
                .get("schema")
                .map(|s| schema_to_rust_type(s))
                .unwrap_or_else(|| "String".to_string());

            let param = Param {
                name: pname.clone(),
                rust_type,
                required,
            };

            // Only treat as path param if the path template actually has {name}
            let placeholder = format!("{{{pname}}}");
            if loc == "path" && path.contains(&placeholder) {
                path_params.push(param);
            } else {
                query_params.push(param);
            }
        }
    }

    let request_body = detail
        .get("requestBody")
        .and_then(|rb| rb["content"]["application/json"]["schema"].get("$ref"))
        .and_then(|r| r.as_str())
        .map(ref_to_type);

    let response = find_success_response(detail);

    Some(Endpoint {
        method,
        path: path.to_string(),
        operation_id,
        tag,
        path_params,
        query_params,
        request_body,
        response,
    })
}

fn find_success_response(detail: &Value) -> ResponseKind {
    let responses = match detail["responses"].as_object() {
        Some(r) => r,
        None => return ResponseKind::Empty,
    };

    // Prefer 200, then 201, then first 2xx
    for code in ["200", "201"] {
        if let Some(resp) = responses.get(code) {
            let schema = &resp["content"]["application/json"]["schema"];

            // Direct $ref (e.g. { "$ref": "#/components/schemas/PartyView" })
            if let Some(schema_ref) = schema.get("$ref").and_then(|r| r.as_str()) {
                return ResponseKind::Json(resolve_ref_type(schema_ref));
            }

            // Inline schema — arrays, objects, primitives without $ref
            // This catches list endpoints that return arrays or inline objects
            if schema.is_object() {
                if schema.as_object().map_or(true, |o| o.is_empty()) {
                    // Empty schema but JSON content type → untyped JSON
                    return ResponseKind::Json("serde_json::Value".to_string());
                }
                let rust_type = schema_to_rust_type(schema);
                return ResponseKind::Json(rust_type);
            }
        }
    }

    // 204 or DELETE with no content → empty
    for (code, _) in responses {
        if code == "204" {
            return ResponseKind::Empty;
        }
    }

    ResponseKind::Empty
}
