mod emit_client;
mod emit_types;
mod spec;

use spec::ParsedSpec;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::{env, fs, process};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: client-codegen <openapi.json> <output-dir>");
        eprintln!("  output-dir receives src/ files and Cargo.toml for the client crate");
        process::exit(1);
    }

    let spec_path = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(&args[2]);

    let raw = fs::read_to_string(&spec_path).unwrap_or_else(|e| {
        eprintln!("Cannot read {}: {e}", spec_path.display());
        process::exit(1);
    });

    let doc: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!("Invalid JSON in {}: {e}", spec_path.display());
        process::exit(1);
    });

    let parsed = ParsedSpec::from_openapi(&doc);

    let src_dir = out_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap_or_else(|e| {
        eprintln!("Cannot create {}: {e}", src_dir.display());
        process::exit(1);
    });

    // Clean stale generated .rs files (preserves non-generated files)
    if let Ok(entries) = fs::read_dir(&src_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let _ = fs::remove_file(&path);
            }
        }
    }

    // Emit types — split into multiple files if over 500 lines
    let types_code = emit_types::emit(&parsed);
    let types_files = if types_code.lines().count() > 500 {
        split_types(&parsed)
    } else {
        vec![("types.rs".to_string(), types_code)]
    };
    for (fname, content) in &types_files {
        write_file(&src_dir.join(fname), content);
    }

    // Group endpoints by tag → one file per tag
    let mut by_tag: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, ep) in parsed.endpoints.iter().enumerate() {
        by_tag.entry(ep.tag.clone()).or_default().push(i);
    }

    let mut mod_names: Vec<String> = Vec::new();
    for (tag, indices) in &by_tag {
        let mod_name = tag_to_mod_name(tag);
        let code = emit_client::emit(&parsed, tag, indices);
        let chunks = split_if_needed(&code, &mod_name);
        for (file_name, chunk) in &chunks {
            write_file(&src_dir.join(file_name), chunk);
        }
        if chunks.len() == 1 {
            mod_names.push(mod_name);
        } else {
            for (file_name, _) in &chunks {
                let m = file_name.trim_end_matches(".rs");
                mod_names.push(m.to_string());
            }
        }
    }

    // Collect type module names for lib.rs
    let type_mod_names: Vec<String> = types_files
        .iter()
        .map(|(f, _)| f.trim_end_matches(".rs").to_string())
        .collect();

    // Emit lib.rs
    let lib_code = emit_lib(&parsed, &mod_names, &type_mod_names);
    write_file(&src_dir.join("lib.rs"), &lib_code);

    // Emit Cargo.toml
    let cargo_code = emit_cargo_toml(&parsed, &out_dir);
    write_file(&out_dir.join("Cargo.toml"), &cargo_code);

    let type_count = parsed.types.len();
    let ep_count = parsed.endpoints.len();
    let file_count = mod_names.len() + 2; // +types.rs +lib.rs
    eprintln!(
        "Generated {} types, {} endpoints across {} files for {}",
        type_count, ep_count, file_count, parsed.crate_name
    );
}

fn tag_to_mod_name(tag: &str) -> String {
    let mut s: String = tag
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    s.trim_matches('_').to_string()
}

fn write_file(path: &Path, content: &str) {
    fs::write(path, content).unwrap_or_else(|e| {
        eprintln!("Cannot write {}: {e}", path.display());
        process::exit(1);
    });
}

/// If a file exceeds 500 lines, split its impl blocks into numbered files.
fn split_if_needed(code: &str, mod_name: &str) -> Vec<(String, String)> {
    let line_count = code.lines().count();
    if line_count <= 500 {
        return vec![(format!("{mod_name}.rs"), code.to_string())];
    }

    // Find split points at `impl` boundaries
    let lines: Vec<&str> = code.lines().collect();
    let mut chunks: Vec<(String, String)> = Vec::new();
    let mut header = String::new();
    let mut impls: Vec<(usize, usize)> = Vec::new();
    let mut in_impl = false;
    let mut brace_depth: i32 = 0;
    let mut impl_start = 0;

    for (i, line) in lines.iter().enumerate() {
        if !in_impl && line.starts_with("impl ") {
            impl_start = i;
            in_impl = true;
            brace_depth = 0;
        }
        if in_impl {
            brace_depth += line.matches('{').count() as i32;
            brace_depth -= line.matches('}').count() as i32;
            if brace_depth == 0 {
                impls.push((impl_start, i));
                in_impl = false;
            }
        }
        if impls.is_empty() && !in_impl {
            header.push_str(line);
            header.push('\n');
        }
    }

    if impls.len() <= 1 {
        return vec![(format!("{mod_name}.rs"), code.to_string())];
    }

    // Split impls into groups that fit under 500 lines
    let mut part = 1;
    let mut current_lines: Vec<&str> = Vec::new();
    let max = 450; // leave room for header

    for (start, end) in &impls {
        let block: Vec<&str> = lines[*start..=*end].to_vec();
        if !current_lines.is_empty()
            && current_lines.len() + block.len() + header.lines().count() > max
        {
            let mut content = header.clone();
            for l in &current_lines {
                content.push_str(l);
                content.push('\n');
            }
            chunks.push((format!("{mod_name}_{part}.rs"), content));
            part += 1;
            current_lines.clear();
        }
        current_lines.extend(block);
    }

    if !current_lines.is_empty() {
        let mut content = header.clone();
        for l in &current_lines {
            content.push_str(l);
            content.push('\n');
        }
        chunks.push((format!("{mod_name}_{part}.rs"), content));
    }

    chunks
}

fn emit_lib(parsed: &ParsedSpec, mod_names: &[String], type_mod_names: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "//! Generated typed client for the {} service.\n",
        parsed.service_title
    ));
    out.push_str("//!\n//! Auto-generated by client-codegen — do not edit.\n\n");

    // Declare type modules
    for m in type_mod_names {
        out.push_str(&format!("pub mod {m};\n"));
    }

    // Declare endpoint modules
    for m in mod_names {
        out.push_str(&format!("pub mod {m};\n"));
    }

    // Re-export all types
    out.push('\n');
    for m in type_mod_names {
        out.push_str(&format!("pub use {m}::*;\n"));
    }

    // Collect all client struct names for re-export
    let mut tags: BTreeSet<String> = BTreeSet::new();
    for ep in &parsed.endpoints {
        tags.insert(ep.tag.clone());
    }
    for tag in &tags {
        let struct_name = tag_to_client_name(tag);
        let mod_name = tag_to_mod_name(tag);
        // Use first matching mod_name (even if split)
        let actual_mod = mod_names
            .iter()
            .find(|m| m == &&mod_name || m.starts_with(&format!("{mod_name}_")))
            .cloned()
            .unwrap_or(mod_name.clone());
        out.push_str(&format!("pub use {actual_mod}::{struct_name};\n"));

        // Re-export public query structs from this tag's module
        let tag_eps: Vec<&spec::Endpoint> = parsed
            .endpoints
            .iter()
            .filter(|ep| ep.tag == *tag)
            .collect();
        for ep in tag_eps {
            if ep.query_params.len() >= emit_client::QUERY_STRUCT_THRESHOLD {
                let query_name = emit_client::op_id_to_query_struct(&ep.operation_id);
                out.push_str(&format!("pub use {actual_mod}::{query_name};\n"));
            }
        }
    }

    // Emit PlatformService trait impls so verticals can use
    // ctx.platform_client::<T>() from the SDK's VerticalBuilder.
    let service_name = parsed
        .crate_name
        .strip_prefix("platform-client-")
        .unwrap_or(&parsed.crate_name);
    out.push_str("\n// -- PlatformService trait impls (connects to SDK VerticalBuilder) --\n\n");
    for tag in &tags {
        let struct_name = tag_to_client_name(tag);
        out.push_str(&format!(
            "impl platform_sdk::PlatformService for {struct_name} {{\n"
        ));
        out.push_str(&format!(
            "    const SERVICE_NAME: &'static str = \"{service_name}\";\n"
        ));
        out.push_str(
            "    fn from_platform_client(client: platform_sdk::PlatformClient) -> Self {\n",
        );
        out.push_str("        Self::new(client)\n");
        out.push_str("    }\n");
        out.push_str("}\n\n");
    }

    out
}

/// Split types into multiple files that each fit under 500 lines.
fn split_types(parsed: &ParsedSpec) -> Vec<(String, String)> {
    let full_header = emit_types::header(parsed);
    let cont_header = emit_types::header_imports(parsed);
    let fragments = emit_types::emit_fragments(parsed);
    let header_lines = full_header.lines().count();
    let max_body_lines = 450_usize.saturating_sub(header_lines);

    let mut files: Vec<(String, String)> = Vec::new();
    let mut current = String::new();
    let mut current_lines = 0_usize;
    let mut part = 1;

    for frag in &fragments {
        let frag_lines = frag.lines().count();
        if !current.is_empty() && current_lines + frag_lines > max_body_lines {
            let hdr = if part == 1 {
                &full_header
            } else {
                &cont_header
            };
            let mut content = hdr.clone();
            content.push_str(&current);
            files.push((format!("types_{part}.rs"), content));
            part += 1;
            current.clear();
            current_lines = 0;
        }
        current.push_str(frag);
        current_lines += frag_lines;
    }

    if !current.is_empty() {
        let hdr = if part == 1 {
            &full_header
        } else {
            &cont_header
        };
        let mut content = hdr.clone();
        content.push_str(&current);
        files.push((format!("types_{part}.rs"), content));
    }

    files
}

fn tag_to_client_name(tag: &str) -> String {
    let pascal: String = tag
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + &c.as_str().to_lowercase(),
            }
        })
        .collect();
    format!("{pascal}Client")
}

fn emit_cargo_toml(parsed: &ParsedSpec, out_dir: &Path) -> String {
    // Calculate relative path from output dir to platform-sdk
    let sdk_path = relative_path(out_dir, "platform/platform-sdk");
    let contracts_path = relative_path(out_dir, "platform/http-contracts");

    format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2021"
description = "Generated typed client for {title}"

[dependencies]
platform-sdk = {{ path = "{sdk_path}" }}
platform-http-contracts = {{ path = "{contracts_path}" }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
uuid = {{ version = "1", features = ["v4", "serde"] }}
chrono = {{ version = "0.4", features = ["serde"] }}

[lints]
workspace = true
"#,
        crate_name = parsed.crate_name,
        title = parsed.service_title,
        sdk_path = sdk_path,
        contracts_path = contracts_path,
    )
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    if dir.is_relative() {
        dir = env::current_dir().ok()?.join(dir);
    }
    dir = dir.canonicalize().unwrap_or(dir);
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.exists() {
            if let Ok(content) = fs::read_to_string(&candidate) {
                if content.contains("[workspace]") {
                    return Some(dir);
                }
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn relative_path(from_dir: &Path, target: &str) -> String {
    // Resolve from_dir to absolute
    let abs_from = if from_dir.is_absolute() {
        from_dir.to_path_buf()
    } else {
        env::current_dir().unwrap_or_default().join(from_dir)
    };

    if let Some(ws_root) = find_workspace_root(&abs_from) {
        // Count how many components from_dir is below ws_root
        let from_canon = abs_from.canonicalize().unwrap_or(abs_from);
        if let Ok(rel) = from_canon.strip_prefix(&ws_root) {
            let depth = rel.components().count();
            let prefix = "../".repeat(depth);
            return format!("{prefix}{target}");
        }
    }

    // Fallback: assume depth 2 (clients/X or modules/X)
    format!("../../{target}")
}
