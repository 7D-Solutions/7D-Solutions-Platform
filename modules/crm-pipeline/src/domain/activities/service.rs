//! Activity service.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    repo, Activity, ActivityError, CreateActivityRequest, ListActivitiesQuery,
    UpdateActivityRequest,
};
use crate::events::{
    build_activity_completed_envelope, build_activity_logged_envelope, ActivityCompletedPayload,
    ActivityLoggedPayload, EVENT_TYPE_ACTIVITY_COMPLETED, EVENT_TYPE_ACTIVITY_LOGGED,
};
use crate::outbox;

pub async fn log_activity(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateActivityRequest,
    actor: String,
) -> Result<Activity, ActivityError> {
    if req.subject.trim().is_empty() {
        return Err(ActivityError::Validation("subject is required".into()));
    }
    if req.activity_type_code.trim().is_empty() {
        return Err(ActivityError::Validation(
            "activity_type_code is required".into(),
        ));
    }

    // INVARIANT: must reference at least one entity
    if req.lead_id.is_none()
        && req.opportunity_id.is_none()
        && req.party_id.is_none()
        && req.party_contact_id.is_none()
    {
        return Err(ActivityError::NoEntityReference);
    }

    let activity = Activity {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        activity_type_code: req.activity_type_code.clone(),
        subject: req.subject.clone(),
        description: req.description.clone(),
        activity_date: req.activity_date,
        duration_minutes: req.duration_minutes,
        lead_id: req.lead_id,
        opportunity_id: req.opportunity_id,
        party_id: req.party_id,
        party_contact_id: req.party_contact_id,
        due_date: req.due_date,
        is_completed: false,
        completed_at: None,
        assigned_to: req.assigned_to.clone(),
        created_by: actor,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let mut tx = pool.begin().await?;
    let created = repo::insert_activity(&mut *tx, &activity).await?;

    let entity_id = created
        .lead_id
        .or(created.opportunity_id)
        .or(created.party_id)
        .or(created.party_contact_id)
        .unwrap_or(created.id);
    let entity_type = if created.lead_id.is_some() {
        "lead"
    } else if created.opportunity_id.is_some() {
        "opportunity"
    } else if created.party_id.is_some() {
        "party"
    } else {
        "party_contact"
    };

    let payload = ActivityLoggedPayload {
        activity_id: created.id,
        tenant_id: tenant_id.to_string(),
        activity_type_code: created.activity_type_code.clone(),
        entity_type: entity_type.to_string(),
        entity_id,
        assigned_to: created.assigned_to.clone(),
    };
    let envelope = build_activity_logged_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_ACTIVITY_LOGGED,
        "activity",
        &created.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(created)
}

pub async fn get_activity(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<Activity, ActivityError> {
    repo::fetch_activity(pool, tenant_id, id)
        .await?
        .ok_or(ActivityError::NotFound(id))
}

pub async fn list_activities(
    pool: &PgPool,
    tenant_id: &str,
    query: &ListActivitiesQuery,
) -> Result<Vec<Activity>, ActivityError> {
    repo::list_activities(pool, tenant_id, query).await
}

pub async fn complete_activity(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    actor: String,
) -> Result<Activity, ActivityError> {
    let act = get_activity(pool, tenant_id, id).await?;
    if act.is_completed {
        return Err(ActivityError::AlreadyCompleted);
    }

    let mut tx = pool.begin().await?;
    let updated = repo::complete_activity(&mut *tx, tenant_id, id).await?;

    let completed_at = updated.completed_at.unwrap_or_else(Utc::now);
    let payload = ActivityCompletedPayload {
        activity_id: updated.id,
        tenant_id: tenant_id.to_string(),
        completed_at,
        completed_by: actor,
    };
    let envelope = build_activity_completed_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_ACTIVITY_COMPLETED,
        "activity",
        &updated.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

pub async fn update_activity(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &UpdateActivityRequest,
) -> Result<Activity, ActivityError> {
    let act = get_activity(pool, tenant_id, id).await?;
    if act.is_completed {
        return Err(ActivityError::AlreadyCompleted);
    }
    repo::update_activity(pool, tenant_id, id, req).await
}
