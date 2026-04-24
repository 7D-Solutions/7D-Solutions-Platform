//! HTTP handlers for notification preference PATCH endpoints (bd-kv15d).
//!
//! Routes:
//!   PATCH /api/customers/:id/notifications
//!   PATCH /api/customers/:id/ship-to/:contact_id/notifications

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use platform_http_contracts::{ApiError, FieldError};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::notifications::{parse_notification_channels, parse_notification_events};
use crate::outbox::enqueue_event_tx;
use crate::AppState;
use platform_sdk::extract_tenant;

// ============================================================================
// Request types
// ============================================================================

/// PATCH body for party-level notification preferences.
/// Omitted field = no change to that column.
#[derive(Debug, Deserialize)]
pub struct PatchPartyNotificationsRequest {
    pub notification_events: Option<Vec<String>>,
    pub notification_channels: Option<Vec<String>>,
}

/// Three-state field for optional nullable contact notification columns.
///
/// - `Missing`: field absent from JSON body → no change to column
/// - `Null`: field present as JSON `null` → clear override (set column to SQL NULL)
/// - `Values(...)`: field present as string array → set column to that array
///
/// Requires `#[serde(default)]` on the struct field so absent fields get `Missing`.
#[derive(Debug, Default)]
pub enum ContactNotifField {
    #[default]
    Missing,
    Null,
    Values(Vec<String>),
}

impl<'de> serde::Deserialize<'de> for ContactNotifField {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        match v {
            Value::Null => Ok(ContactNotifField::Null),
            Value::Array(arr) => {
                let strings: Result<Vec<_>, _> = arr
                    .iter()
                    .map(|x| {
                        x.as_str()
                            .map(|s| s.to_string())
                            .ok_or_else(|| serde::de::Error::custom("expected string in array"))
                    })
                    .collect();
                Ok(ContactNotifField::Values(strings?))
            }
            _ => Err(serde::de::Error::custom("expected null or string array")),
        }
    }
}

/// PATCH body for ship-to (contact) notification override.
/// Field present with array = set override.
/// Field present with JSON null = clear override (restore inheritance).
/// Field omitted = no change to that column.
#[derive(Debug, Deserialize)]
pub struct PatchContactNotificationsRequest {
    #[serde(default)]
    pub notification_events: ContactNotifField,
    #[serde(default)]
    pub notification_channels: ContactNotifField,
}

// ============================================================================
// Outbox event payload
// ============================================================================

#[derive(Debug, Serialize)]
struct NotificationPreferencesChangedPayload {
    tenant_id: String,
    party_id: String,
    notification_events: Vec<String>,
    notification_channels: Vec<String>,
    changed_by: String,
}

// ============================================================================
// Permission helper
// ============================================================================

fn require_notification_permission(claims: &Option<Extension<VerifiedClaims>>) -> Result<Uuid, Response> {
    let c = match claims {
        Some(Extension(c)) => c,
        None => return Err(ApiError::unauthorized("Authentication required").into_response()),
    };
    let allowed = c.roles.iter().any(|r| r == "tenant_admin" || r == "customer_manager" || r == "admin");
    if !allowed {
        return Err(
            ApiError::forbidden("tenant_admin or customer_manager role required").into_response()
        );
    }
    Ok(c.user_id)
}

// ============================================================================
// PATCH /api/customers/:id/notifications
// ============================================================================

pub async fn patch_party_notifications(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
    Json(body): Json<PatchPartyNotificationsRequest>,
) -> Response {
    let caller_id = match require_notification_permission(&claims) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    // Validate enum membership before touching DB
    if let Some(ref evts) = body.notification_events {
        if let Err(fe) = parse_notification_events(evts) {
            return ApiError::new(400, "validation_error", &fe.message).into_response();
        }
    }
    if let Some(ref chans) = body.notification_channels {
        if let Err(fe) = parse_notification_channels(chans) {
            return ApiError::new(400, "validation_error", &fe.message).into_response();
        }
    }

    // Nothing to update if both omitted
    if body.notification_events.is_none() && body.notification_channels.is_none() {
        return StatusCode::NO_CONTENT.into_response();
    }

    // Begin transaction for Guard→Mutation→Outbox atomicity
    let mut tx = match state.pool.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "Failed to begin transaction");
            return ApiError::internal("Database error").into_response();
        }
    };

    // Guard: party must exist and belong to this tenant
    let row: Option<(Value, Value)> = match sqlx::query_as(
        "SELECT notification_events, notification_channels \
         FROM party_parties WHERE id = $1 AND app_id = $2",
    )
    .bind(party_id)
    .bind(&app_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "DB error fetching party");
            return ApiError::internal("Database error").into_response();
        }
    };

    let (current_events_json, current_channels_json) = match row {
        Some(r) => r,
        None => return ApiError::not_found(format!("Party {} not found", party_id)).into_response(),
    };

    let current_events: Vec<String> = serde_json::from_value(current_events_json).unwrap_or_default();
    let current_channels: Vec<String> = serde_json::from_value(current_channels_json).unwrap_or_default();

    let new_events = body.notification_events.as_deref().unwrap_or(&current_events);
    let new_channels = body.notification_channels.as_deref().unwrap_or(&current_channels);

    let events_json = serde_json::to_value(new_events).unwrap_or(Value::Array(vec![]));
    let channels_json = serde_json::to_value(new_channels).unwrap_or(Value::Array(vec![]));

    // Mutation
    if let Err(e) = sqlx::query(
        "UPDATE party_parties SET notification_events = $1, notification_channels = $2 \
         WHERE id = $3 AND app_id = $4",
    )
    .bind(&events_json)
    .bind(&channels_json)
    .bind(party_id)
    .bind(&app_id)
    .execute(&mut *tx)
    .await
    {
        tracing::error!(error = %e, "DB error updating party notification prefs");
        return ApiError::internal("Database error").into_response();
    }

    // Outbox event
    let payload = NotificationPreferencesChangedPayload {
        tenant_id: app_id.clone(),
        party_id: party_id.to_string(),
        notification_events: new_events.to_vec(),
        notification_channels: new_channels.to_vec(),
        changed_by: caller_id.to_string(),
    };
    if let Err(e) = enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        "party.notification_preferences_changed",
        "party",
        &party_id.to_string(),
        &app_id,
        &payload,
    )
    .await
    {
        tracing::error!(error = %e, "Failed to enqueue notification_preferences_changed event");
        return ApiError::internal("Database error").into_response();
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Transaction commit failed");
        return ApiError::internal("Database error").into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}

// ============================================================================
// PATCH /api/customers/:id/ship-to/:contact_id/notifications
// ============================================================================

pub async fn patch_contact_notifications(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((party_id, contact_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<PatchContactNotificationsRequest>,
) -> Response {
    if let Err(e) = require_notification_permission(&claims) {
        return e;
    }

    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    // Validate enum values (only when setting, not clearing)
    if let ContactNotifField::Values(ref evts) = body.notification_events {
        if let Err(fe) = parse_notification_events(evts) {
            return ApiError::new(400, "validation_error", &fe.message).into_response();
        }
    }
    if let ContactNotifField::Values(ref chans) = body.notification_channels {
        if let Err(fe) = parse_notification_channels(chans) {
            return ApiError::new(400, "validation_error", &fe.message).into_response();
        }
    }

    if matches!(body.notification_events, ContactNotifField::Missing)
        && matches!(body.notification_channels, ContactNotifField::Missing)
    {
        return StatusCode::NO_CONTENT.into_response();
    }

    // Guard: contact must exist and belong to this party+tenant
    let exists: Option<(Uuid,)> = match sqlx::query_as(
        "SELECT c.id FROM party_contacts c \
         JOIN party_parties p ON p.id = c.party_id \
         WHERE c.id = $1 AND c.party_id = $2 AND p.app_id = $3",
    )
    .bind(contact_id)
    .bind(party_id)
    .bind(&app_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "DB error fetching contact");
            return ApiError::internal("Database error").into_response();
        }
    };

    if exists.is_none() {
        return ApiError::not_found(format!(
            "Contact {} not found for party {}",
            contact_id, party_id
        ))
        .into_response();
    }

    // Build SET clauses: None = missing (skip), Some(None) = clear, Some(Some(v)) = set
    let events_value: Option<Option<Value>> = match body.notification_events {
        ContactNotifField::Missing => None,
        ContactNotifField::Null => Some(None),
        ContactNotifField::Values(ref arr) => {
            Some(Some(serde_json::to_value(arr).unwrap_or(Value::Null)))
        }
    };
    let channels_value: Option<Option<Value>> = match body.notification_channels {
        ContactNotifField::Missing => None,
        ContactNotifField::Null => Some(None),
        ContactNotifField::Values(ref arr) => {
            Some(Some(serde_json::to_value(arr).unwrap_or(Value::Null)))
        }
    };

    // Each column update uses IS-value-or-NULL semantics. When the Option is None we
    // emit a literal NULL literal via a dedicated query to avoid JSONB type-inference issues.
    if let Some(ev) = events_value {
        let q = match ev {
            Some(v) => sqlx::query(
                "UPDATE party_contacts SET notification_events = $1 WHERE id = $2",
            )
            .bind(v)
            .bind(contact_id),
            None => sqlx::query(
                "UPDATE party_contacts SET notification_events = NULL WHERE id = $1",
            )
            .bind(contact_id),
        };
        if let Err(e) = q.execute(&state.pool).await {
            tracing::error!(error = %e, "DB error updating contact notification_events");
            return ApiError::internal("Database error").into_response();
        }
    }
    if let Some(ch) = channels_value {
        let q = match ch {
            Some(v) => sqlx::query(
                "UPDATE party_contacts SET notification_channels = $1 WHERE id = $2",
            )
            .bind(v)
            .bind(contact_id),
            None => sqlx::query(
                "UPDATE party_contacts SET notification_channels = NULL WHERE id = $1",
            )
            .bind(contact_id),
        };
        if let Err(e) = q.execute(&state.pool).await {
            tracing::error!(error = %e, "DB error updating contact notification_channels");
            return ApiError::internal("Database error").into_response();
        }
    }

    StatusCode::NO_CONTENT.into_response()
}
