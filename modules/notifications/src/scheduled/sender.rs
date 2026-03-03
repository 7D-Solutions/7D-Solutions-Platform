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
}

impl NotificationError {
    pub fn class(&self) -> &'static str {
        match self {
            NotificationError::Transient(_) => "transient",
            NotificationError::InvalidRecipient(_) => "invalid_recipient",
            NotificationError::ProviderAuth(_) => "provider_auth",
            NotificationError::RateLimited(_) => "rate_limited",
            NotificationError::Permanent(_) => "permanent",
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
    async fn send(
        &self,
        notif: &ScheduledNotification,
    ) -> Result<SendReceipt, NotificationError> {
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

    fn resolve_recipient(notif: &ScheduledNotification) -> Result<String, NotificationError> {
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
    async fn send(
        &self,
        notif: &ScheduledNotification,
    ) -> Result<SendReceipt, NotificationError> {
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
            s if s.is_success() => Ok(SendReceipt { provider_message_id }),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(NotificationError::ProviderAuth(
                format!("provider returned status {status}"),
            )),
            StatusCode::TOO_MANY_REQUESTS => Err(NotificationError::RateLimited(
                "provider returned 429".to_string(),
            )),
            StatusCode::BAD_REQUEST
            | StatusCode::NOT_FOUND
            | StatusCode::UNPROCESSABLE_ENTITY => Err(NotificationError::InvalidRecipient(
                format!("provider rejected recipient/status {status}"),
            )),
            s if s.is_server_error() => Err(NotificationError::Transient(format!(
                "provider server error status {s}"
            ))),
            _ => Err(NotificationError::Permanent(format!(
                "provider returned unexpected status {status}"
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
    async fn send(
        &self,
        _notif: &ScheduledNotification,
    ) -> Result<SendReceipt, NotificationError> {
        use std::sync::atomic::Ordering;
        let prev = self.remaining.fetch_sub(1, Ordering::SeqCst);
        if prev > 0 {
            Err(NotificationError::Transient("simulated failure".to_string()))
        } else {
            Ok(SendReceipt::default())
        }
    }
}
