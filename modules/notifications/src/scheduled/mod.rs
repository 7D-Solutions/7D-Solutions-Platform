pub mod dispatcher;
pub mod models;
pub mod repo;
pub mod sender;

pub use dispatcher::{dispatch_once, DispatchResult};
pub use models::{InsertPending, ScheduledNotification};
pub use repo::{
    claim_due_batch, insert_pending, mark_sent, record_delivery_attempt_and_mutate,
    reschedule_or_fail, reset_orphaned_claims, AttemptApplyOutcome,
};
pub use sender::{
    HttpEmailSender, LoggingSender, NotificationError, NotificationSender, SendReceipt,
};
