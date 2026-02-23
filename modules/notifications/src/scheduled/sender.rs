use async_trait::async_trait;
use thiserror::Error;

use super::models::ScheduledNotification;

/// Errors that can occur when delivering a notification.
#[derive(Debug, Error)]
pub enum NotificationError {
    #[error("delivery failed: {0}")]
    DeliveryFailed(String),
}

/// Abstraction over actual notification delivery (email, SMS, webhook, etc.).
///
/// Implementations receive a single `ScheduledNotification` and are responsible
/// for dispatching it to the appropriate channel.  They must be `Send + Sync`
/// so they can be shared across tokio tasks.
#[async_trait]
pub trait NotificationSender: Send + Sync {
    async fn send(&self, notif: &ScheduledNotification) -> Result<(), NotificationError>;
}

/// Production stub: logs the notification and always succeeds.
///
/// Replace with a real implementation (email provider, SMS gateway, etc.)
/// that satisfies the `NotificationSender` trait.
pub struct LoggingSender;

#[async_trait]
impl NotificationSender for LoggingSender {
    async fn send(&self, notif: &ScheduledNotification) -> Result<(), NotificationError> {
        tracing::info!(
            id = %notif.id,
            recipient = %notif.recipient_ref,
            channel = %notif.channel,
            template = %notif.template_key,
            "dispatching scheduled notification"
        );
        Ok(())
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
    async fn send(&self, _notif: &ScheduledNotification) -> Result<(), NotificationError> {
        use std::sync::atomic::Ordering;
        let prev = self.remaining.fetch_sub(1, Ordering::SeqCst);
        if prev > 0 {
            Err(NotificationError::DeliveryFailed("simulated failure".to_string()))
        } else {
            Ok(())
        }
    }
}
