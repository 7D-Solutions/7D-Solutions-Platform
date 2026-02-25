use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use chrono::Utc;
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::shipments::{
    Direction, InboundStatus, OutboundStatus, Shipment, ShipmentError, ShipmentService,
    TransitionRequest,
};
use crate::outbox;
use crate::AppState;

use super::types::{
    error_response, extract_tenant, idempotency_key, AddLineRequest, CreateShipmentRequest,
    ListShipmentsQuery, ReceiveLineRequest, ShipLineQtyRequest, ShipmentLineRow,
    TransitionStatusRequest,
};

/// POST /api/shipping-receiving/shipments
pub async fn create_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<CreateShipmentRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let _idem = idempotency_key(&headers);

    let initial_status = match req.direction {
        Direction::Inbound => InboundStatus::Draft.as_str(),
        Direction::Outbound => OutboundStatus::Draft.as_str(),
    };

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => return error_response(ShipmentError::Database(e)).into_response(),
    };

    let shipment = match sqlx::query_as::<_, Shipment>(
        r#"
        INSERT INTO shipments (tenant_id, direction, status, carrier_party_id,
            tracking_number, freight_cost_minor, currency, expected_arrival_date, created_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(req.direction.as_str())
    .bind(initial_status)
    .bind(req.carrier_party_id)
    .bind(&req.tracking_number)
    .bind(req.freight_cost_minor)
    .bind(&req.currency)
    .bind(req.expected_arrival_date)
    .bind(claims.as_ref().map(|Extension(c)| c.user_id))
    .fetch_one(&mut *tx)
    .await
    {
        Ok(s) => s,
        Err(e) => return error_response(ShipmentError::Database(e)).into_response(),
    };

    let event_payload = json!({
        "shipment_id": shipment.id,
        "tenant_id": tenant_id,
        "direction": req.direction.as_str(),
        "status": initial_status,
    });
    if let Err(e) = outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        "shipping.shipment.created",
        "shipment",
        &shipment.id.to_string(),
        &tenant_id.to_string(),
        &event_payload,
    )
    .await
    {
        return error_response(ShipmentError::Database(e)).into_response();
    }

    if let Err(e) = tx.commit().await {
        return error_response(ShipmentError::Database(e)).into_response();
    }

    (StatusCode::CREATED, Json(json!(shipment))).into_response()
}

/// GET /api/shipping-receiving/shipments/:id
pub async fn get_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match ShipmentService::find_by_id(&state.pool, id, tenant_id).await {
        Ok(Some(s)) => (StatusCode::OK, Json(json!(s))).into_response(),
        Ok(None) => error_response(ShipmentError::NotFound).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// GET /api/shipping-receiving/shipments
pub async fn list_shipments(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListShipmentsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0).max(0);

    let rows = sqlx::query_as::<_, Shipment>(
        r#"
        SELECT * FROM shipments
        WHERE tenant_id = $1
          AND ($2::text IS NULL OR direction = $2)
          AND ($3::text IS NULL OR status = $3)
        ORDER BY created_at DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(tenant_id)
    .bind(&q.direction)
    .bind(&q.status)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await;

    match rows {
        Ok(shipments) => (StatusCode::OK, Json(json!(shipments))).into_response(),
        Err(e) => error_response(ShipmentError::Database(e)).into_response(),
    }
}

/// PATCH /api/shipping-receiving/shipments/:id/status
pub async fn transition_status(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<TransitionStatusRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let _idem = idempotency_key(&headers);

    let domain_req = TransitionRequest {
        status: req.status,
        arrived_at: req.arrived_at,
        shipped_at: req.shipped_at,
        delivered_at: req.delivered_at,
        closed_at: req.closed_at,
    };

    match ShipmentService::transition(&state.pool, id, tenant_id, &domain_req).await {
        Ok(s) => (StatusCode::OK, Json(json!(s))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// POST /api/shipping-receiving/shipments/:id/lines
pub async fn add_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(shipment_id): Path<Uuid>,
    Json(req): Json<AddLineRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let _idem = idempotency_key(&headers);

    if req.qty_expected < 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": "qty_expected must be >= 0" })),
        )
            .into_response();
    }

    let shipment = match ShipmentService::find_by_id(&state.pool, shipment_id, tenant_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return error_response(ShipmentError::NotFound).into_response(),
        Err(e) => return error_response(e).into_response(),
    };

    let is_terminal = match shipment.direction {
        Direction::Inbound => InboundStatus::from_str_value(&shipment.status)
            .map(|s| s.is_terminal())
            .unwrap_or(false),
        Direction::Outbound => OutboundStatus::from_str_value(&shipment.status)
            .map(|s| s.is_terminal())
            .unwrap_or(false),
    };
    if is_terminal {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "validation_error",
                "message": "Cannot add lines to a shipment in terminal status"
            })),
        )
            .into_response();
    }

    let line = sqlx::query_as::<_, ShipmentLineRow>(
        r#"
        INSERT INTO shipment_lines (tenant_id, shipment_id, sku, uom, warehouse_id,
            qty_expected, source_ref_type, source_ref_id, po_id, po_line_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(&req.sku)
    .bind(&req.uom)
    .bind(req.warehouse_id)
    .bind(req.qty_expected)
    .bind(&req.source_ref_type)
    .bind(req.source_ref_id)
    .bind(req.po_id)
    .bind(req.po_line_id)
    .fetch_one(&state.pool)
    .await;

    match line {
        Ok(l) => (StatusCode::CREATED, Json(json!(l))).into_response(),
        Err(e) => error_response(ShipmentError::Database(e)).into_response(),
    }
}

/// POST /api/shipping-receiving/shipments/:shipment_id/lines/:line_id/receive
pub async fn receive_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path((shipment_id, line_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<ReceiveLineRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let _idem = idempotency_key(&headers);

    let shipment = match ShipmentService::find_by_id(&state.pool, shipment_id, tenant_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return error_response(ShipmentError::NotFound).into_response(),
        Err(e) => return error_response(e).into_response(),
    };

    if shipment.direction != Direction::Inbound {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "validation_error",
                "message": "receive is only valid for inbound shipments"
            })),
        )
            .into_response();
    }

    let line = sqlx::query_as::<_, ShipmentLineRow>(
        r#"
        UPDATE shipment_lines SET
            qty_received = $4,
            qty_accepted = $5,
            qty_rejected = $6,
            updated_at = NOW()
        WHERE id = $1 AND shipment_id = $2 AND tenant_id = $3
        RETURNING *
        "#,
    )
    .bind(line_id)
    .bind(shipment_id)
    .bind(tenant_id)
    .bind(req.qty_received)
    .bind(req.qty_accepted)
    .bind(req.qty_rejected)
    .fetch_optional(&state.pool)
    .await;

    match line {
        Ok(Some(l)) => (StatusCode::OK, Json(json!(l))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Shipment line not found" })),
        )
            .into_response(),
        Err(e) => error_response(ShipmentError::Database(e)).into_response(),
    }
}

/// POST /api/shipping-receiving/shipments/:shipment_id/lines/:line_id/ship-qty
pub async fn ship_line_qty(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path((shipment_id, line_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<ShipLineQtyRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let _idem = idempotency_key(&headers);

    let shipment = match ShipmentService::find_by_id(&state.pool, shipment_id, tenant_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return error_response(ShipmentError::NotFound).into_response(),
        Err(e) => return error_response(e).into_response(),
    };

    if shipment.direction != Direction::Outbound {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "validation_error",
                "message": "ship-qty is only valid for outbound shipments"
            })),
        )
            .into_response();
    }

    let line = sqlx::query_as::<_, ShipmentLineRow>(
        r#"
        UPDATE shipment_lines SET
            qty_shipped = $4,
            updated_at = NOW()
        WHERE id = $1 AND shipment_id = $2 AND tenant_id = $3
        RETURNING *
        "#,
    )
    .bind(line_id)
    .bind(shipment_id)
    .bind(tenant_id)
    .bind(req.qty_shipped)
    .fetch_optional(&state.pool)
    .await;

    match line {
        Ok(Some(l)) => (StatusCode::OK, Json(json!(l))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Shipment line not found" })),
        )
            .into_response(),
        Err(e) => error_response(ShipmentError::Database(e)).into_response(),
    }
}

/// POST /api/shipping-receiving/shipments/:id/close
pub async fn close_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let _idem = idempotency_key(&headers);

    let req = TransitionRequest {
        status: "closed".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: None,
        closed_at: Some(Utc::now()),
    };

    match ShipmentService::transition(&state.pool, id, tenant_id, &req).await {
        Ok(s) => (StatusCode::OK, Json(json!(s))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// POST /api/shipping-receiving/shipments/:id/ship
pub async fn ship_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let _idem = idempotency_key(&headers);

    let req = TransitionRequest {
        status: "shipped".to_string(),
        arrived_at: None,
        shipped_at: Some(Utc::now()),
        delivered_at: None,
        closed_at: None,
    };

    match ShipmentService::transition(&state.pool, id, tenant_id, &req).await {
        Ok(s) => (StatusCode::OK, Json(json!(s))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// POST /api/shipping-receiving/shipments/:id/deliver
pub async fn deliver_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let _idem = idempotency_key(&headers);

    let req = TransitionRequest {
        status: "delivered".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: Some(Utc::now()),
        closed_at: None,
    };

    match ShipmentService::transition(&state.pool, id, tenant_id, &req).await {
        Ok(s) => (StatusCode::OK, Json(json!(s))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// POST /api/shipping-receiving/shipments/:id/lines/:line_id/accept
pub async fn accept_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((shipment_id, line_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let line = sqlx::query_as::<_, ShipmentLineRow>(
        r#"
        UPDATE shipment_lines SET
            qty_accepted = qty_received,
            qty_rejected = 0,
            updated_at = NOW()
        WHERE id = $1 AND shipment_id = $2 AND tenant_id = $3
        RETURNING *
        "#,
    )
    .bind(line_id)
    .bind(shipment_id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await;

    match line {
        Ok(Some(l)) => (StatusCode::OK, Json(json!(l))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Shipment line not found" })),
        )
            .into_response(),
        Err(e) => error_response(ShipmentError::Database(e)).into_response(),
    }
}
