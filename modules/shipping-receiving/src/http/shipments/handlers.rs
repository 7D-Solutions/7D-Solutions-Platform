use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use chrono::Utc;
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use platform_sdk::extract_tenant;
use security::{permissions, VerifiedClaims};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repository::ShipmentRepository;
use crate::domain::outbound_ship::{OutboundShipRequest, OutboundShipService};
use crate::domain::shipments::{
    Direction, InboundStatus, OutboundStatus, ShipmentError, ShipmentService, TransitionRequest,
};
use crate::outbox;
use crate::AppState;

use super::types::{
    idempotency_key, with_request_id, AddLineRequest, CreateShipmentRequest, ListShipmentsQuery,
    ReceiveLineRequest, ShipLineQtyRequest, ShipOutboundRequest, ShipmentLineRow,
    TransitionStatusRequest,
};

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments",
    tag = "Shipments",
    request_body = CreateShipmentRequest,
    responses(
        (status = 201, description = "Shipment created", body = crate::domain::shipments::Shipment),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<CreateShipmentRequest>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let _idem = idempotency_key(&headers);

    let initial_status = match req.direction {
        Direction::Inbound => InboundStatus::Draft.as_str(),
        Direction::Outbound => OutboundStatus::Draft.as_str(),
    };

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
                .into_response()
        }
    };

    let params = crate::db::repository::InsertShipmentParams {
        tenant_id,
        direction: req.direction.as_str().to_string(),
        status: initial_status.to_string(),
        carrier_party_id: req.carrier_party_id,
        tracking_number: req.tracking_number.clone(),
        freight_cost_minor: req.freight_cost_minor,
        currency: req.currency.clone(),
        expected_arrival_date: req.expected_arrival_date,
        created_by: claims.as_ref().map(|Extension(c)| c.user_id),
        source_ref_type: None,
        source_ref_id: None,
    };
    let shipment = match ShipmentRepository::insert_shipment_tx(&mut tx, &params).await {
        Ok(s) => s,
        Err(e) => {
            return with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
                .into_response()
        }
    };

    let event_payload = serde_json::json!({
        "shipment_id": shipment.id,
        "tenant_id": tenant_id,
        "direction": req.direction.as_str(),
        "status": initial_status,
    });
    if let Err(e) = outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        crate::events::EVENT_TYPE_SHIPMENT_CREATED,
        "shipment",
        &shipment.id.to_string(),
        &tenant_id.to_string(),
        &event_payload,
    )
    .await
    {
        return with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
            .into_response();
    }

    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
            .into_response();
    }

    (StatusCode::CREATED, Json(serde_json::json!(shipment))).into_response()
}

#[utoipa::path(
    get,
    path = "/api/shipping-receiving/shipments/{id}",
    tag = "Shipments",
    params(("id" = Uuid, Path, description = "Shipment ID")),
    responses(
        (status = 200, description = "Shipment details", body = crate::domain::shipments::Shipment),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match ShipmentService::find_by_id(&state.pool, id, tenant_id).await {
        Ok(Some(s)) => (StatusCode::OK, Json(serde_json::json!(s))).into_response(),
        Ok(None) => {
            with_request_id(ApiError::from(ShipmentError::NotFound), &tracing_ctx).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/shipping-receiving/shipments",
    tag = "Shipments",
    params(ListShipmentsQuery),
    responses(
        (status = 200, description = "Paginated shipments", body = PaginatedResponse<crate::domain::shipments::Shipment>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_shipments(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(q): Query<ListShipmentsQuery>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * page_size;

    let total = match ShipmentRepository::count_shipments(
        &state.pool,
        tenant_id,
        q.direction.as_deref(),
        q.status.as_deref(),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            return with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
                .into_response()
        }
    };

    match ShipmentRepository::list_shipments(
        &state.pool,
        tenant_id,
        q.direction.as_deref(),
        q.status.as_deref(),
        page_size,
        offset,
    )
    .await
    {
        Ok(shipments) => {
            let resp = PaginatedResponse::new(shipments, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
            .into_response(),
    }
}

#[utoipa::path(
    patch,
    path = "/api/shipping-receiving/shipments/{id}/status",
    tag = "Shipments",
    params(("id" = Uuid, Path, description = "Shipment ID")),
    request_body = TransitionStatusRequest,
    responses(
        (status = 200, description = "Shipment after transition", body = crate::domain::shipments::Shipment),
        (status = 400, description = "Invalid transition or guard failure", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn transition_status(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<TransitionStatusRequest>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let _idem = idempotency_key(&headers);

    let domain_req = TransitionRequest {
        status: req.status,
        arrived_at: req.arrived_at,
        shipped_at: req.shipped_at,
        delivered_at: req.delivered_at,
        closed_at: req.closed_at,
    };

    match ShipmentService::transition(&state.pool, id, tenant_id, &domain_req, &state.inventory)
        .await
    {
        Ok(s) => (StatusCode::OK, Json(serde_json::json!(s))).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments/{id}/lines",
    tag = "Shipment Lines",
    params(("id" = Uuid, Path, description = "Shipment ID")),
    request_body = AddLineRequest,
    responses(
        (status = 201, description = "Line added", body = ShipmentLineRow),
        (status = 400, description = "Validation error", body = ApiError),
        (status = 404, description = "Shipment not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn add_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(shipment_id): Path<Uuid>,
    Json(req): Json<AddLineRequest>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let _idem = idempotency_key(&headers);

    if req.qty_expected < 0 {
        return with_request_id(
            ApiError::bad_request("qty_expected must be >= 0"),
            &tracing_ctx,
        )
        .into_response();
    }

    let shipment = match ShipmentService::find_by_id(&state.pool, shipment_id, tenant_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return with_request_id(ApiError::from(ShipmentError::NotFound), &tracing_ctx)
                .into_response()
        }
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
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
        return with_request_id(
            ApiError::bad_request("Cannot add lines to a shipment in terminal status"),
            &tracing_ctx,
        )
        .into_response();
    }

    let line_params = crate::db::repository::InsertLineParams {
        tenant_id,
        shipment_id,
        sku: req.sku.clone(),
        uom: req.uom.clone(),
        warehouse_id: req.warehouse_id,
        qty_expected: req.qty_expected,
        source_ref_type: req.source_ref_type.clone(),
        source_ref_id: req.source_ref_id,
        po_id: req.po_id,
        po_line_id: req.po_line_id,
    };
    let line = ShipmentRepository::insert_line(&state.pool, &line_params).await;

    match line {
        Ok(l) => (StatusCode::CREATED, Json(serde_json::json!(l))).into_response(),
        Err(e) => with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments/{shipment_id}/lines/{line_id}/receive",
    tag = "Shipment Lines",
    params(
        ("shipment_id" = Uuid, Path, description = "Shipment ID"),
        ("line_id" = Uuid, Path, description = "Shipment line ID"),
    ),
    request_body = ReceiveLineRequest,
    responses(
        (status = 200, description = "Line received", body = ShipmentLineRow),
        (status = 400, description = "Validation error", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn receive_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path((shipment_id, line_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<ReceiveLineRequest>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let _idem = idempotency_key(&headers);

    let shipment = match ShipmentService::find_by_id(&state.pool, shipment_id, tenant_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return with_request_id(ApiError::from(ShipmentError::NotFound), &tracing_ctx)
                .into_response()
        }
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    if shipment.direction != Direction::Inbound {
        return with_request_id(
            ApiError::bad_request("receive is only valid for inbound shipments"),
            &tracing_ctx,
        )
        .into_response();
    }

    let line = ShipmentRepository::receive_line(
        &state.pool,
        line_id,
        shipment_id,
        tenant_id,
        req.qty_received,
        req.qty_accepted,
        req.qty_rejected,
    )
    .await;

    match line {
        Ok(Some(l)) => (StatusCode::OK, Json(serde_json::json!(l))).into_response(),
        Ok(None) => with_request_id(ApiError::not_found("Shipment line not found"), &tracing_ctx)
            .into_response(),
        Err(e) => with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments/{shipment_id}/lines/{line_id}/ship-qty",
    tag = "Shipment Lines",
    params(
        ("shipment_id" = Uuid, Path, description = "Shipment ID"),
        ("line_id" = Uuid, Path, description = "Shipment line ID"),
    ),
    request_body = ShipLineQtyRequest,
    responses(
        (status = 200, description = "Shipped quantity set", body = ShipmentLineRow),
        (status = 400, description = "Validation error", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn ship_line_qty(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path((shipment_id, line_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<ShipLineQtyRequest>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let _idem = idempotency_key(&headers);

    let shipment = match ShipmentService::find_by_id(&state.pool, shipment_id, tenant_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return with_request_id(ApiError::from(ShipmentError::NotFound), &tracing_ctx)
                .into_response()
        }
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    if shipment.direction != Direction::Outbound {
        return with_request_id(
            ApiError::bad_request("ship-qty is only valid for outbound shipments"),
            &tracing_ctx,
        )
        .into_response();
    }

    let line = ShipmentRepository::ship_line_qty(
        &state.pool,
        line_id,
        shipment_id,
        tenant_id,
        req.qty_shipped,
    )
    .await;

    match line {
        Ok(Some(l)) => (StatusCode::OK, Json(serde_json::json!(l))).into_response(),
        Ok(None) => with_request_id(ApiError::not_found("Shipment line not found"), &tracing_ctx)
            .into_response(),
        Err(e) => with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments/{id}/close",
    tag = "Shipments",
    params(("id" = Uuid, Path, description = "Shipment ID")),
    responses(
        (status = 200, description = "Shipment closed", body = crate::domain::shipments::Shipment),
        (status = 400, description = "Invalid transition", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn close_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let _idem = idempotency_key(&headers);

    let req = TransitionRequest {
        status: "closed".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: None,
        closed_at: Some(Utc::now()),
    };

    match ShipmentService::transition(&state.pool, id, tenant_id, &req, &state.inventory).await {
        Ok(s) => (StatusCode::OK, Json(serde_json::json!(s))).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments/{id}/ship",
    tag = "Shipments",
    params(("id" = Uuid, Path, description = "Shipment ID")),
    responses(
        (status = 200, description = "Shipment shipped", body = crate::domain::shipments::Shipment),
        (status = 400, description = "Invalid transition", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn ship_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let _idem = idempotency_key(&headers);

    let req = TransitionRequest {
        status: "shipped".to_string(),
        arrived_at: None,
        shipped_at: Some(Utc::now()),
        delivered_at: None,
        closed_at: None,
    };

    match ShipmentService::transition(&state.pool, id, tenant_id, &req, &state.inventory).await {
        Ok(s) => (StatusCode::OK, Json(serde_json::json!(s))).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments/{id}/deliver",
    tag = "Shipments",
    params(("id" = Uuid, Path, description = "Shipment ID")),
    responses(
        (status = 200, description = "Shipment delivered", body = crate::domain::shipments::Shipment),
        (status = 400, description = "Invalid transition", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn deliver_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let _idem = idempotency_key(&headers);

    let req = TransitionRequest {
        status: "delivered".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: Some(Utc::now()),
        closed_at: None,
    };

    match ShipmentService::transition(&state.pool, id, tenant_id, &req, &state.inventory).await {
        Ok(s) => (StatusCode::OK, Json(serde_json::json!(s))).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments/{shipment_id}/lines/{line_id}/accept",
    tag = "Shipment Lines",
    params(
        ("shipment_id" = Uuid, Path, description = "Shipment ID"),
        ("line_id" = Uuid, Path, description = "Shipment line ID"),
    ),
    responses(
        (status = 200, description = "Line accepted", body = ShipmentLineRow),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn accept_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path((shipment_id, line_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    let line = ShipmentRepository::accept_line(&state.pool, line_id, shipment_id, tenant_id).await;

    match line {
        Ok(Some(l)) => (StatusCode::OK, Json(serde_json::json!(l))).into_response(),
        Ok(None) => with_request_id(ApiError::not_found("Shipment line not found"), &tracing_ctx)
            .into_response(),
        Err(e) => with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments/{id}/outbound",
    tag = "Shipments",
    params(("id" = Uuid, Path, description = "Shipment ID")),
    request_body = ShipOutboundRequest,
    responses(
        (status = 200, description = "Shipment shipped", body = crate::domain::shipments::Shipment),
        (status = 400, description = "Invalid state or direction", body = ApiError),
        (status = 403, description = "Quality gate hold or insufficient permissions", body = ApiError),
        (status = 404, description = "Shipment not found", body = ApiError),
        (status = 502, description = "Quality inspection service error", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn ship_outbound(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<ShipOutboundRequest>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|id| {
        id.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let _idem = idempotency_key(&headers);

    // Determine whether the caller holds the quality_inspection.mutate permission.
    let caller_can_override_qi = claims
        .as_ref()
        .map(|Extension(c)| {
            c.perms
                .iter()
                .any(|p| p == permissions::QUALITY_INSPECTION_MUTATE)
        })
        .unwrap_or(false);

    let domain_req = OutboundShipRequest {
        shipment_id: id,
        tenant_id,
        shipped_at: req.shipped_at,
        override_reason: req.override_reason,
        caller_can_override_qi,
    };

    match OutboundShipService::execute(
        &state.pool,
        domain_req,
        &state.inventory,
        &state.quality_gate,
    )
    .await
    {
        Ok(shipment) => (StatusCode::OK, Json(serde_json::json!(shipment))).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
