//! Channel-agnostic notification message template engine.
//!
//! Deterministic rendering: `template_key` + `payload_json` → `RenderedMessage`.
//! Variable substitution uses `{{key}}` syntax with explicit error classification
//! for missing variables, unknown templates, and invalid payloads.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

// ── Rendered output ─────────────────────────────────────────────────

/// The product of a successful template render — subject, HTML body, and
/// plain-text body. Channel-agnostic; the delivery sender decides which
/// fields to use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderedMessage {
    pub subject: String,
    pub body_html: String,
    pub body_text: String,
}

// ── Error classification ────────────────────────────────────────────

/// Explicit render error types — every failure is classified so callers can
/// distinguish "template doesn't exist" from "payload is missing a field."
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("unknown template: {0}")]
    UnknownTemplate(String),

    #[error("missing variable '{variable}' in template '{template_key}'")]
    MissingVariable {
        template_key: String,
        variable: String,
    },

    #[error("invalid payload for template '{template_key}': {reason}")]
    InvalidPayload {
        template_key: String,
        reason: String,
    },
}

impl RenderError {
    /// Machine-readable error class for persistence in delivery attempt rows.
    pub fn class(&self) -> &'static str {
        match self {
            RenderError::UnknownTemplate(_) => "unknown_template",
            RenderError::MissingVariable { .. } => "missing_variable",
            RenderError::InvalidPayload { .. } => "invalid_payload",
        }
    }
}

// ── Template definition ─────────────────────────────────────────────

struct Template {
    subject: &'static str,
    body_html: &'static str,
    body_text: &'static str,
}

/// Look up a template definition by key. Returns `UnknownTemplate` for
/// keys that don't match a registered template.
fn lookup(template_key: &str) -> Result<Template, RenderError> {
    match template_key {
        "invoice_due_soon" => Ok(Template {
            subject: "Invoice {{invoice_id}} — payment due {{due_date}}",
            body_html: concat!(
                "<p>Your invoice <strong>{{invoice_id}}</strong> ",
                "for {{amount}} is due on {{due_date}}.</p>",
            ),
            body_text: "Your invoice {{invoice_id}} for {{amount}} is due on {{due_date}}.",
        }),

        "payment_succeeded" => Ok(Template {
            subject: "Payment received for invoice {{invoice_id}}",
            body_html: concat!(
                "<p>We received your payment of {{amount}} ",
                "for invoice <strong>{{invoice_id}}</strong>.</p>",
            ),
            body_text: "We received your payment of {{amount}} for invoice {{invoice_id}}.",
        }),

        "payment_retry" => Ok(Template {
            subject: "Action needed — payment for {{invoice_id}} failed",
            body_html: concat!(
                "<p>Payment for invoice <strong>{{invoice_id}}</strong> ",
                "failed ({{failure_code}}). Please update your payment method and retry.</p>",
            ),
            body_text: concat!(
                "Payment for invoice {{invoice_id}} failed ({{failure_code}}). ",
                "Please update your payment method and retry.",
            ),
        }),

        "low_stock_alert" => Ok(Template {
            subject: "Low stock: {{item_id}} at warehouse {{warehouse_id}}",
            body_html: concat!(
                "<p>Item <strong>{{item_id}}</strong> at warehouse ",
                "<strong>{{warehouse_id}}</strong> is below reorder point. ",
                "Available: {{available_qty}}, reorder at: {{reorder_point}}.</p>",
            ),
            body_text: concat!(
                "Item {{item_id}} at warehouse {{warehouse_id}} is below reorder point. ",
                "Available: {{available_qty}}, reorder at: {{reorder_point}}.",
            ),
        }),

        "order_shipped" => Ok(Template {
            subject: "Your order has shipped — tracking {{tracking_number}}",
            body_html: concat!(
                "<p>Hi {{recipient_name}},</p>",
                "<p>Your shipment has been handed to <strong>{{carrier}}</strong>.</p>",
                "<p>Tracking number: <strong>{{tracking_number}}</strong></p>",
                "<p>Shipped at: {{shipped_at}}</p>",
                "<p>Use your tracking number to follow your delivery status.</p>",
            ),
            body_text: concat!(
                "Hi {{recipient_name}},\n\n",
                "Your shipment has been handed to {{carrier}}.\n",
                "Tracking number: {{tracking_number}}\n",
                "Shipped at: {{shipped_at}}\n\n",
                "Use your tracking number to follow your delivery status.",
            ),
        }),

        "delivery_confirmed" => Ok(Template {
            subject: "Your order has been delivered",
            body_html: concat!(
                "<p>Hi {{recipient_name}},</p>",
                "<p>Your shipment has been delivered.</p>",
                "<p>Delivered at: {{delivered_at}}</p>",
                "<p>Thank you for your order!</p>",
            ),
            body_text: concat!(
                "Hi {{recipient_name}},\n\n",
                "Your shipment has been delivered.\n",
                "Delivered at: {{delivered_at}}\n\n",
                "Thank you for your order!",
            ),
        }),

        _ => Err(RenderError::UnknownTemplate(template_key.to_string())),
    }
}

// ── Variable substitution ───────────────────────────────────────────

/// Replace `{{key}}` placeholders with values from `vars`. Returns an
/// explicit error for missing keys or non-scalar values — never silently
/// leaves unreplaced placeholders.
fn substitute(
    template_str: &str,
    vars: &serde_json::Map<String, Value>,
    template_key: &str,
) -> Result<String, RenderError> {
    let mut result = String::with_capacity(template_str.len());
    let mut rest = template_str;

    while let Some(start) = rest.find("{{") {
        result.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];

        let end = after_open.find("}}").ok_or_else(|| RenderError::InvalidPayload {
            template_key: template_key.to_string(),
            reason: "unclosed '{{' in template".to_string(),
        })?;

        let var_name = after_open[..end].trim();

        let value = vars.get(var_name).ok_or_else(|| RenderError::MissingVariable {
            template_key: template_key.to_string(),
            variable: var_name.to_string(),
        })?;

        match value {
            Value::String(s) => result.push_str(s),
            Value::Number(n) => result.push_str(&n.to_string()),
            Value::Bool(b) => result.push_str(if *b { "true" } else { "false" }),
            Value::Null => {
                return Err(RenderError::MissingVariable {
                    template_key: template_key.to_string(),
                    variable: var_name.to_string(),
                });
            }
            _ => {
                return Err(RenderError::InvalidPayload {
                    template_key: template_key.to_string(),
                    reason: format!("variable '{}' is not a scalar value", var_name),
                });
            }
        }

        rest = &after_open[end + 2..];
    }

    result.push_str(rest);
    Ok(result)
}

// ── Public render entry point ───────────────────────────────────────

/// Render a notification template. Pure, deterministic, no I/O.
///
/// Given a `template_key` and a JSON `payload`, produces a `RenderedMessage`
/// with subject, HTML body, and plain-text body. The same inputs always
/// produce the same outputs.
pub fn render(template_key: &str, payload: &Value) -> Result<RenderedMessage, RenderError> {
    let tpl = lookup(template_key)?;

    let vars = payload.as_object().ok_or_else(|| RenderError::InvalidPayload {
        template_key: template_key.to_string(),
        reason: "payload must be a JSON object".to_string(),
    })?;

    Ok(RenderedMessage {
        subject: substitute(tpl.subject, vars, template_key)?,
        body_html: substitute(tpl.body_html, vars, template_key)?,
        body_text: substitute(tpl.body_text, vars, template_key)?,
    })
}

// ── Golden tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_invoice_due_soon() {
        let payload = serde_json::json!({
            "invoice_id": "INV-001",
            "amount": 15000,
            "due_date": "2026-03-15",
        });
        let msg = render("invoice_due_soon", &payload).expect("render should succeed");
        assert_eq!(msg.subject, "Invoice INV-001 — payment due 2026-03-15");
        assert_eq!(
            msg.body_html,
            "<p>Your invoice <strong>INV-001</strong> for 15000 is due on 2026-03-15.</p>"
        );
        assert_eq!(
            msg.body_text,
            "Your invoice INV-001 for 15000 is due on 2026-03-15."
        );
    }

    #[test]
    fn golden_payment_succeeded() {
        let payload = serde_json::json!({
            "invoice_id": "INV-042",
            "amount": 9900,
        });
        let msg = render("payment_succeeded", &payload).expect("render should succeed");
        assert_eq!(msg.subject, "Payment received for invoice INV-042");
        assert_eq!(
            msg.body_html,
            "<p>We received your payment of 9900 for invoice <strong>INV-042</strong>.</p>"
        );
        assert_eq!(
            msg.body_text,
            "We received your payment of 9900 for invoice INV-042."
        );
    }

    #[test]
    fn golden_payment_retry() {
        let payload = serde_json::json!({
            "invoice_id": "INV-007",
            "failure_code": "card_declined",
        });
        let msg = render("payment_retry", &payload).expect("render should succeed");
        assert_eq!(
            msg.subject,
            "Action needed — payment for INV-007 failed"
        );
        assert!(msg.body_html.contains("card_declined"));
        assert!(msg.body_text.contains("card_declined"));
    }

    #[test]
    fn golden_low_stock_alert() {
        let payload = serde_json::json!({
            "item_id": "ITEM-500",
            "warehouse_id": "WH-1",
            "available_qty": 3,
            "reorder_point": 10,
        });
        let msg = render("low_stock_alert", &payload).expect("render should succeed");
        assert_eq!(msg.subject, "Low stock: ITEM-500 at warehouse WH-1");
        assert!(msg.body_text.contains("Available: 3, reorder at: 10"));
    }

    #[test]
    fn deterministic_same_inputs_same_outputs() {
        let payload = serde_json::json!({
            "invoice_id": "INV-DET",
            "amount": 500,
            "due_date": "2026-04-01",
        });
        let a = render("invoice_due_soon", &payload).expect("render should succeed");
        let b = render("invoice_due_soon", &payload).expect("render should succeed");
        assert_eq!(a, b);
    }

    #[test]
    fn error_unknown_template() {
        let result = render("nonexistent_template", &serde_json::json!({}));
        let err = result.unwrap_err();
        assert_eq!(err.class(), "unknown_template");
        assert!(err.to_string().contains("nonexistent_template"));
    }

    #[test]
    fn error_missing_variable() {
        let result = render("invoice_due_soon", &serde_json::json!({"invoice_id": "X"}));
        let err = result.unwrap_err();
        assert_eq!(err.class(), "missing_variable");
        assert!(err.to_string().contains("amount") || err.to_string().contains("due_date"));
    }

    #[test]
    fn error_payload_not_object() {
        let result = render("invoice_due_soon", &serde_json::json!("not an object"));
        let err = result.unwrap_err();
        assert_eq!(err.class(), "invalid_payload");
    }

    #[test]
    fn error_non_scalar_variable() {
        let payload = serde_json::json!({
            "invoice_id": ["not", "a", "scalar"],
            "amount": 100,
            "due_date": "2026-01-01",
        });
        let result = render("invoice_due_soon", &payload);
        let err = result.unwrap_err();
        assert_eq!(err.class(), "invalid_payload");
    }

    #[test]
    fn error_null_variable_treated_as_missing() {
        let payload = serde_json::json!({
            "invoice_id": null,
            "amount": 100,
            "due_date": "2026-01-01",
        });
        let result = render("invoice_due_soon", &payload);
        let err = result.unwrap_err();
        assert_eq!(err.class(), "missing_variable");
    }

    #[test]
    fn boolean_variable_renders() {
        // Not used by current templates, but validates the substitution path.
        let payload = serde_json::json!({
            "invoice_id": "INV-B",
            "amount": true,
            "due_date": "2026-05-01",
        });
        let msg = render("invoice_due_soon", &payload).expect("render should succeed");
        assert!(msg.body_text.contains("true"));
    }

    #[test]
    fn string_variable_renders_verbatim() {
        let payload = serde_json::json!({
            "invoice_id": "INV-<script>alert(1)</script>",
            "amount": 100,
            "due_date": "2026-01-01",
        });
        let msg = render("invoice_due_soon", &payload).expect("render should succeed");
        // Substitution is verbatim — HTML escaping is the sender's responsibility.
        assert!(msg.body_text.contains("INV-<script>alert(1)</script>"));
    }
}
