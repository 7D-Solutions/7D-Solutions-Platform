pub mod models;
pub mod repo;

pub use models::InboxMessage;
pub use repo::{
    create_inbox_message, dismiss_message, get_message, list_messages, mark_read, mark_unread,
    undismiss_message, InboxListParams,
};
