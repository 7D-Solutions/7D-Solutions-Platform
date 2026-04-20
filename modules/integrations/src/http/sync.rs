//! HTTP handlers for the sync API.
//!
//! Each handler is a stub that returns 501 Not Implemented.  Downstream beads
//! wire in the real domain logic while keeping auth gating unchanged.
//!
//! Permission matrix:
//!   POST /api/integrations/sync/authority            → integrations.sync.authority.flip
//!   POST /api/integrations/sync/conflicts/{id}/resolve → integrations.sync.conflict.resolve
//!   POST /api/integrations/sync/push/{entity_type}   → integrations.sync.push
//!   GET  /api/integrations/sync/conflicts            → integrations.sync.read
//!   GET  /api/integrations/sync/dlq                  → integrations.sync.read
//!   GET  /api/integrations/sync/push-attempts        → integrations.sync.read
//!   GET  /api/integrations/sync/jobs                 → integrations.sync.read

use axum::{extract::Path, http::StatusCode};

pub async fn flip_authority() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn resolve_conflict(Path(_id): Path<uuid::Uuid>) -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn push_entity(Path(_entity_type): Path<String>) -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn list_conflicts() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn list_dlq() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn list_push_attempts() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn list_jobs() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}
