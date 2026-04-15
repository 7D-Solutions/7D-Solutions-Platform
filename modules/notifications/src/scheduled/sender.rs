use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::Value;
use thiserror::Error;

use super::models::ScheduledNotification;

/// Errors that can occur when delivering a notification.
#[derive(Debug, Error)]
pub enum NotificationError {
    #[error("transient delivery failure: {0}")]
    Transient(String),
    #[error("invalid recipient: {0}")]
    InvalidRecipient(String),
    #[error("provider auth/config failure: {0}")]
    ProviderAuth(String),
    #[error("rate limited by provider: {0}")]
    RateLimited(String),
    #[error("permanent delivery failure: {0}")]
    Permanent(String),
    #[error("render failure: {0}")]
    RenderFailure(String),
}

impl NotificationError {
    pub fn class(&self) -> &'static str {
        match self {
            NotificationError::Transient(_) => "transient",
            NotificationError::InvalidRecipient(_) => "invalid_recipient",
            NotificationError::ProviderAuth(_) => "provider_auth",
            NotificationError::RateLimited(_) => "rate_limited",
            NotificationError::Permanent(_) => "permanent",
            NotificationError::RenderFailure(_) => "render_failure",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            NotificationError::Transient(_) | NotificationError::RateLimited(_)
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct SendReceipt {
    pub provider_message_id: Option<String>,
}

/// Abstraction over actual notification delivery (email, SMS, webhook, etc.).
///
/// Implementations receive a single `ScheduledNotification` and are responsible
/// for dispatching it to the appropriate channel.  They must be `Send + Sync`
/// so they can be shared across tokio tasks.
#[async_trait]
pub trait NotificationSender: Send + Sync {
    async fn send(&self, notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError>;
}

/// Production stub: logs the notification and always succeeds.
///
/// Replace with a real implementation (email provider, SMS gateway, etc.)
/// that satisfies the `NotificationSender` trait.
pub struct LoggingSender;

#[async_trait]
impl NotificationSender for LoggingSender {
    async fn send(&self, notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError> {
        tracing::info!(
            id = %notif.id,
            recipient = %notif.recipient_ref,
            channel = %notif.channel,
            template = %notif.template_key,
            "dispatching scheduled notification"
        );
        Ok(SendReceipt::default())
    }
}

/// Provider-backed sender that performs real HTTP delivery to an email gateway.
pub struct HttpEmailSender {
    client: reqwest::Client,
    endpoint: String,
    from: String,
    api_key: Option<String>,
}

impl HttpEmailSender {
    pub fn new(endpoint: String, from: String, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
            from,
            api_key,
        }
    }

    pub fn resolve_recipient(notif: &ScheduledNotification) -> Result<String, NotificationError> {
        if let Some(email) = notif.payload_json.get("email").and_then(|v| v.as_str()) {
            if email.contains('@') {
                return Ok(email.to_string());
            }
        }

        if notif.recipient_ref.contains('@') {
            return Ok(notif.recipient_ref.clone());
        }

        Err(NotificationError::InvalidRecipient(format!(
            "recipient_ref '{}' is not an email and payload has no valid email field",
            notif.recipient_ref
        )))
    }
}

#[async_trait]
impl NotificationSender for HttpEmailSender {
    async fn send(&self, notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError> {
        let to = Self::resolve_recipient(notif)?;
        let body = serde_json::json!({
            "to": to,
            "from": self.from,
            "template_key": notif.template_key,
            "payload": notif.payload_json,
            "notification_id": notif.id,
        });

        let mut req = self.client.post(&self.endpoint).json(&body);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| NotificationError::Transient(e.to_string()))?;

        let status = resp.status();
        let payload: Option<Value> = resp.json::<Value>().await.ok();
        let provider_message_id = payload
            .as_ref()
            .and_then(|v| v.get("message_id"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);

        match status {
            s if s.is_success() => Ok(SendReceipt {
                provider_message_id,
            }),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(
                NotificationError::ProviderAuth(format!("provider returned status {status}")),
            ),
            StatusCode::TOO_MANY_REQUESTS => Err(NotificationError::RateLimited(
                "provider returned 429".to_string(),
            )),
            StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::UNPROCESSABLE_ENTITY => {
                Err(NotificationError::InvalidRecipient(format!(
                    "provider rejected recipient/status {status}"
                )))
            }
            s if s.is_server_error() => Err(NotificationError::Transient(format!(
                "provider server error status {s}"
            ))),
            _ => Err(NotificationError::Permanent(format!(
                "provider returned unexpected status {status}"
            ))),
        }
    }
}

/// SendGrid v3/mail/send adapter.
///
/// Transforms the platform notification format into SendGrid's expected
/// `{personalizations, from, subject, content}` shape and POSTs to SendGrid's
/// API.  Supports two modes:
///
/// 1. **Dynamic templates** — if `payload_json.sendgrid_template_id` is present,
///    the adapter sends a dynamic template request with `payload_json` as the
///    template data.
/// 2. **Direct content** — otherwise, `payload_json.subject` and
///    `payload_json.body` are used as the email subject and HTML body.
pub struct SendGridEmailSender {
    client: reqwest::Client,
    from: String,
    api_key: String,
}

impl SendGridEmailSender {
    pub fn new(from: String, api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            from,
            api_key,
        }
    }
}

#[async_trait]
impl NotificationSender for SendGridEmailSender {
    async fn send(&self, notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError> {
        let to = HttpEmailSender::resolve_recipient(notif)?;

        let personalizations = if let Some(template_id) = notif
            .payload_json
            .get("sendgrid_template_id")
            .and_then(|v| v.as_str())
        {
            // Dynamic template mode — pass payload as template data.
            let mut data = notif.payload_json.clone();
            // Strip the meta key so it doesn't leak into the template context.
            if let Some(obj) = data.as_object_mut() {
                obj.remove("sendgrid_template_id");
            }
            serde_json::json!({
                "personalizations": [{
                    "to": [{"email": to}],
                    "dynamic_template_data": data,
                }],
                "from": {"email": self.from},
                "template_id": template_id,
            })
        } else {
            // Direct content mode.
            let subject = notif
                .payload_json
                .get("subject")
                .and_then(|v| v.as_str())
                .unwrap_or(&notif.template_key);
            let body = notif
                .payload_json
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            serde_json::json!({
                "personalizations": [{
                    "to": [{"email": to}],
                }],
                "from": {"email": self.from},
                "subject": subject,
                "content": [{"type": "text/html", "value": body}],
            })
        };

        let resp = self
            .client
            .post("https://api.sendgrid.com/v3/mail/send")
            .bearer_auth(&self.api_key)
            .json(&personalizations)
            .send()
            .await
            .map_err(|e| NotificationError::Transient(e.to_string()))?;

        let status = resp.status();

        // SendGrid returns 202 Accepted on success with an empty body.
        // The x-message-id header carries the provider message ID.
        let provider_message_id = resp
            .headers()
            .get("x-message-id")
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);

        match status {
            s if s.is_success() => Ok(SendReceipt {
                provider_message_id,
            }),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(
                NotificationError::ProviderAuth(format!("SendGrid returned status {status}")),
            ),
            StatusCode::TOO_MANY_REQUESTS => Err(NotificationError::RateLimited(
                "SendGrid returned 429".to_string(),
            )),
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                let body_text = resp.text().await.unwrap_or_default();
                Err(NotificationError::Permanent(format!(
                    "SendGrid rejected request (status {status}): {body_text}"
                )))
            }
            s if s.is_server_error() => Err(NotificationError::Transient(format!(
                "SendGrid server error status {s}"
            ))),
            _ => Err(NotificationError::Permanent(format!(
                "SendGrid returned unexpected status {status}"
            ))),
        }
    }
}

/// Channel-routing sender that delegates to the appropriate backend based on
/// `notif.channel` (e.g., "email" → email sender, "sms" → SMS sender).
///
/// Unknown channels are rejected with a permanent error.
pub struct ChannelRouter {
    pub email: Arc<dyn NotificationSender>,
    pub sms: Arc<dyn NotificationSender>,
}

use std::sync::Arc;

#[async_trait]
impl NotificationSender for ChannelRouter {
    async fn send(&self, notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError> {
        match notif.channel.as_str() {
            "email" => self.email.send(notif).await,
            "sms" => self.sms.send(notif).await,
            other => Err(NotificationError::Permanent(format!(
                "unsupported notification channel: {other}"
            ))),
        }
    }
}

/// Provider-backed sender that performs real HTTP delivery to an SMS gateway.
pub struct HttpSmsSender {
    client: reqwest::Client,
    endpoint: String,
    from_number: String,
    api_key: Option<String>,
}

impl HttpSmsSender {
    pub fn new(endpoint: String, from_number: String, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
            from_number,
            api_key,
        }
    }

    fn resolve_recipient(notif: &ScheduledNotification) -> Result<String, NotificationError> {
        // Try payload_json.phone first
        if let Some(phone) = notif.payload_json.get("phone").and_then(|v| v.as_str()) {
            if phone.starts_with('+') && phone.len() >= 8 {
                return Ok(phone.to_string());
            }
        }

        // Fall back to recipient_ref if it looks like a phone number
        let ref_part = notif.recipient_ref.split(':').next_back().unwrap_or("");
        if ref_part.starts_with('+') && ref_part.len() >= 8 {
            return Ok(ref_part.to_string());
        }

        Err(NotificationError::InvalidRecipient(format!(
            "recipient_ref '{}' is not a phone number and payload has no valid phone field",
            notif.recipient_ref
        )))
    }
}

#[async_trait]
impl NotificationSender for HttpSmsSender {
    async fn send(&self, notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError> {
        let to = Self::resolve_recipient(notif)?;
        let body = serde_json::json!({
            "to": to,
            "from": self.from_number,
            "body": notif.payload_json.get("body").and_then(|v| v.as_str()).unwrap_or(""),
            "template_key": notif.template_key,
            "notification_id": notif.id,
        });

        let mut req = self.client.post(&self.endpoint).json(&body);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| NotificationError::Transient(e.to_string()))?;

        let status = resp.status();
        let payload: Option<Value> = resp.json::<Value>().await.ok();
        let provider_message_id = payload
            .as_ref()
            .and_then(|v| v.get("message_id"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);

        match status {
            s if s.is_success() => Ok(SendReceipt {
                provider_message_id,
            }),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(
                NotificationError::ProviderAuth(format!("SMS provider returned status {status}")),
            ),
            StatusCode::TOO_MANY_REQUESTS => Err(NotificationError::RateLimited(
                "SMS provider returned 429".to_string(),
            )),
            StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::UNPROCESSABLE_ENTITY => {
                Err(NotificationError::InvalidRecipient(format!(
                    "SMS provider rejected recipient/status {status}"
                )))
            }
            s if s.is_server_error() => Err(NotificationError::Transient(format!(
                "SMS provider server error status {s}"
            ))),
            _ => Err(NotificationError::Permanent(format!(
                "SMS provider returned unexpected status {status}"
            ))),
        }
    }
}

/// Test-only sender that fails for the first `fail_count` calls, then succeeds.
///
/// Uses an atomic counter so the struct is `Send + Sync` without a Mutex.
/// Only compiled during unit tests (`cargo test` within the crate).
#[cfg(test)]
pub struct FailingSender {
    remaining: std::sync::atomic::AtomicI32,
}

#[cfg(test)]
impl FailingSender {
    pub fn new(fail_count: i32) -> Self {
        Self {
            remaining: std::sync::atomic::AtomicI32::new(fail_count),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl NotificationSender for FailingSender {
    async fn send(&self, _notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError> {
        use std::sync::atomic::Ordering;
        let prev = self.remaining.fetch_sub(1, Ordering::SeqCst);
        if prev > 0 {
            Err(NotificationError::Transient(
                "simulated failure".to_string(),
            ))
        } else {
            Ok(SendReceipt::default())
        }
    }
}
