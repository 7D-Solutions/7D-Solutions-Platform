//! Standardized command output for tenantctl
//!
//! All commands produce a `CommandOutput` which is rendered either as
//! human-readable text (default) or JSON (--json flag).

use serde::Serialize;

/// Standardized output envelope for all tenantctl commands.
#[derive(Debug, Serialize)]
pub struct CommandOutput {
    pub success: bool,
    pub action: String,
    pub tenant_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl CommandOutput {
    pub fn ok(action: &str, tenant_id: &str) -> Self {
        Self {
            success: true,
            action: action.to_string(),
            tenant_id: tenant_id.to_string(),
            state: None,
            message: None,
            data: None,
        }
    }

    pub fn fail(action: &str, tenant_id: &str, message: &str) -> Self {
        Self {
            success: false,
            action: action.to_string(),
            tenant_id: tenant_id.to_string(),
            state: None,
            message: Some(message.to_string()),
            data: None,
        }
    }

    pub fn with_state(mut self, state: &str) -> Self {
        self.state = Some(state.to_string());
        self
    }

    pub fn with_message(mut self, msg: &str) -> Self {
        self.message = Some(msg.to_string());
        self
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Render output and exit with appropriate code.
pub fn render_and_exit(output: CommandOutput, json: bool) -> ! {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&output).expect("serialize")
        );
    } else {
        render_human(&output);
    }

    if output.success {
        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}

/// Render output without exiting (for commands that return Ok).
pub fn render(output: &CommandOutput, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(output).expect("serialize")
        );
    } else {
        render_human(output);
    }
}

fn render_human(output: &CommandOutput) {
    let icon = if output.success { "✅" } else { "❌" };
    let verb = &output.action;
    let tid = &output.tenant_id;

    if let Some(msg) = &output.message {
        if output.success {
            println!("\n{} Tenant {} {}: {}", icon, tid, verb, msg);
        } else {
            eprintln!("\n{} Tenant {} {} failed: {}", icon, tid, verb, msg);
        }
    } else {
        println!("\n{} Tenant {} {}!", icon, tid, verb);
    }

    if let Some(state) = &output.state {
        println!("   State: {}", state);
    }

    if let Some(data) = &output.data {
        if let Some(obj) = data.as_object() {
            for (key, val) in obj {
                match val {
                    serde_json::Value::String(s) => {
                        println!("   {}: {}", humanize_key(key), s);
                    }
                    serde_json::Value::Number(n) => {
                        println!("   {}: {}", humanize_key(key), n);
                    }
                    serde_json::Value::Bool(b) => {
                        println!("   {}: {}", humanize_key(key), b);
                    }
                    serde_json::Value::Null => {
                        println!("   {}: null", humanize_key(key));
                    }
                    _ => {
                        // Arrays/objects get compact JSON
                        println!("   {}: {}", humanize_key(key), val);
                    }
                }
            }
        }
    }
}

/// Convert snake_case key to Title Case for human output.
fn humanize_key(key: &str) -> String {
    key.split('_')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_output_ok_serializes() {
        let out = CommandOutput::ok("created", "t1").with_state("active");
        let json = serde_json::to_string(&out).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"state\":\"active\""));
    }

    #[test]
    fn command_output_fail_serializes() {
        let out = CommandOutput::fail("created", "t1", "db error");
        let json = serde_json::to_string(&out).unwrap();
        assert!(json.contains("\"success\":false"));
        assert!(json.contains("db error"));
    }

    #[test]
    fn humanize_key_works() {
        assert_eq!(humanize_key("tenant_id"), "Tenant Id");
        assert_eq!(humanize_key("created_at"), "Created At");
        assert_eq!(humanize_key("status"), "Status");
    }

    #[test]
    fn none_fields_omitted_in_json() {
        let out = CommandOutput::ok("show", "t1");
        let json = serde_json::to_string(&out).unwrap();
        assert!(!json.contains("state"));
        assert!(!json.contains("data"));
    }
}
