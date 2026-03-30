use axum::{extract::DefaultBodyLimit, routing::get, Extension, Json, Router};
use event_bus::{connect_nats, EventBus, InMemoryBus, NatsBus};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::{net::SocketAddr, sync::Arc, time::Instant};
use tokio::time::{sleep, Duration};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;

use inventory_rs::{
    AppState, BusHealth, Config,
    consumers::{component_issue_consumer, fg_receipt_consumer},
    db::resolver::resolve_pool,
    http::{
        adjustments::post_adjustment,
        health::{health, ready, version},
        issues::post_issue,
        items::{create_item, deactivate_item, get_item, list_items, update_item},
        locations::{
            create_location, deactivate_location, get_location, list_locations, update_location,
        },
        make_buy::put_make_buy,
        receipts::post_receipt,
        reservations::{post_fulfill, post_release, post_reserve},
        transfers::post_transfer,
        uom::{create_conversion, create_uom, list_conversions, list_uoms},
    },
    http::{
        cycle_counts::{post_cycle_count_approve, post_cycle_count_submit, post_cycle_count_task},
        expiry::{post_expiry_alert_scan, put_lot_expiry},
        genealogy::{get_lot_children, get_lot_parents, post_lot_merge, post_lot_split},
        history::get_movement_history,
        labels::{get_label_by_id, get_list_labels, post_generate_label},
        lots::get_lots_for_item,
        reorder::{
            get_reorder_policy, list_reorder_policies, post_reorder_policy, put_reorder_policy,
        },
        revisions::{
            get_list_revisions, get_revision_at, post_activate_revision, post_create_revision,
            put_revision_policy,
        },
        serials::get_serials_for_item,
        status::post_status_transfer,
        trace::{trace_lot_handler, trace_serial_handler},
        valuation::{get_valuation_snapshot, list_valuation_snapshots, post_valuation_snapshot},
    },
    config::BusType,
    domain::{
        adjust_service::{AdjustRequest, AdjustResult},
        cycle_count::{
            submit_service::SubmitLineInput,
            task_service::{CreateTaskRequest, CreateTaskResult, TaskLine, TaskScope},
        },
        expiry::{
            LotExpiryRecord, RunExpiryAlertScanRequest, RunExpiryAlertScanResult,
            SetLotExpiryRequest,
        },
        fulfill_service::{FulfillRequest, FulfillResult},
        genealogy::{
            GenealogyResult, LotMergeRequest, LotSplitRequest, MergeParent, SplitChild,
        },
        history::query::MovementEntry,
        issue_service::{IssueRequest, IssueResult},
        items::{CreateItemRequest, Item, TrackingMode, UpdateItemRequest},
        labels::{GenerateLabelRequest, Label},
        locations::{CreateLocationRequest, Location, UpdateLocationRequest},
        lots_serials::models::InventoryLot,
        make_buy::SetMakeBuyRequest,
        receipt_service::{ReceiptRequest, ReceiptResult},
        reorder::models::{
            CreateReorderPolicyRequest, ReorderPolicy, UpdateReorderPolicyRequest,
        },
        reservation_service::{
            ReleaseRequest, ReleaseResult, ReserveRequest, ReserveResult,
        },
        revisions::{
            ActivateRevisionRequest, CreateRevisionRequest, ItemRevision,
            UpdateRevisionPolicyRequest,
        },
        status::transfer_service::{StatusTransferRequest, StatusTransferResult},
        transfer_service::{TransferRequest, TransferResult},
        uom::models::{CreateConversionRequest, CreateUomRequest, ItemUomConversion, Uom},
        valuation::{
            models::{ValuationLine, ValuationSnapshot},
            snapshot_service::CreateSnapshotRequest,
        },
    },
    events::contracts::{ConsumedLayer, SourceRef},
    metrics::{metrics_handler, InventoryMetrics},
};
use inventory_rs::domain::items::ListItemsQuery;
use inventory_rs::domain::genealogy::GenealogyEdge;
use inventory_rs::domain::status::models::InvItemStatus;
use inventory_rs::http::cycle_counts::{ApproveBody, SubmitBody};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Inventory Service",
        version = "2.2.0",
        description = "Inventory management: item master, stock receipts (FIFO), issues, \
                        transfers, adjustments, reservations, cycle counts, UoM, locations, \
                        labels, revisions, reorder policies, valuation snapshots, lot/serial \
                        traceability, lot genealogy, expiry monitoring, status transfers.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims \
                        (not headers). Permissions: `INVENTORY_READ` for queries, \
                        `INVENTORY_MUTATE` for writes.\n\n\
                        **Idempotency:** POST endpoints accept `idempotency_key` in the request \
                        body. 201 = created, 200 = replay (same key + same body), 409 = conflict \
                        (same key + different body).",
    ),
    paths(
        // Items
        inventory_rs::http::items::create_item,
        inventory_rs::http::items::get_item,
        inventory_rs::http::items::update_item,
        inventory_rs::http::items::list_items,
        inventory_rs::http::items::deactivate_item,
        inventory_rs::http::make_buy::put_make_buy,
        // Receipts
        inventory_rs::http::receipts::post_receipt,
        // Issues
        inventory_rs::http::issues::post_issue,
        // Adjustments
        inventory_rs::http::adjustments::post_adjustment,
        // Transfers
        inventory_rs::http::transfers::post_transfer,
        // Reservations
        inventory_rs::http::reservations::post_reserve,
        inventory_rs::http::reservations::post_release,
        inventory_rs::http::reservations::post_fulfill,
        // UoM
        inventory_rs::http::uom::create_uom,
        inventory_rs::http::uom::list_uoms,
        inventory_rs::http::uom::create_conversion,
        inventory_rs::http::uom::list_conversions,
        // Locations
        inventory_rs::http::locations::create_location,
        inventory_rs::http::locations::get_location,
        inventory_rs::http::locations::update_location,
        inventory_rs::http::locations::deactivate_location,
        inventory_rs::http::locations::list_locations,
        // Cycle Counts
        inventory_rs::http::cycle_counts::post_cycle_count_task,
        inventory_rs::http::cycle_counts::post_cycle_count_submit,
        inventory_rs::http::cycle_counts::post_cycle_count_approve,
        // Reorder Policies
        inventory_rs::http::reorder::post_reorder_policy,
        inventory_rs::http::reorder::get_reorder_policy,
        inventory_rs::http::reorder::put_reorder_policy,
        inventory_rs::http::reorder::list_reorder_policies,
        // Valuation
        inventory_rs::http::valuation::post_valuation_snapshot,
        inventory_rs::http::valuation::list_valuation_snapshots,
        inventory_rs::http::valuation::get_valuation_snapshot,
        // Revisions
        inventory_rs::http::revisions::post_create_revision,
        inventory_rs::http::revisions::post_activate_revision,
        inventory_rs::http::revisions::put_revision_policy,
        inventory_rs::http::revisions::get_revision_at,
        inventory_rs::http::revisions::get_list_revisions,
        // Labels
        inventory_rs::http::labels::post_generate_label,
        inventory_rs::http::labels::get_list_labels,
        inventory_rs::http::labels::get_label_by_id,
        // Expiry
        inventory_rs::http::expiry::put_lot_expiry,
        inventory_rs::http::expiry::post_expiry_alert_scan,
        // Lot Genealogy
        inventory_rs::http::genealogy::post_lot_split,
        inventory_rs::http::genealogy::post_lot_merge,
        inventory_rs::http::genealogy::get_lot_children,
        inventory_rs::http::genealogy::get_lot_parents,
        // Lots & Serials
        inventory_rs::http::lots::get_lots_for_item,
        inventory_rs::http::serials::get_serials_for_item,
        // Traceability
        inventory_rs::http::trace::trace_lot_handler,
        inventory_rs::http::trace::trace_serial_handler,
        // History
        inventory_rs::http::history::get_movement_history,
        // Status Transfers
        inventory_rs::http::status::post_status_transfer,
    ),
    components(schemas(
        // Item master
        Item, TrackingMode, CreateItemRequest, UpdateItemRequest, ListItemsQuery,
        SetMakeBuyRequest,
        // Receipts / Issues / Adjustments / Transfers
        ReceiptRequest, ReceiptResult,
        IssueRequest, IssueResult, ConsumedLayer, SourceRef,
        AdjustRequest, AdjustResult,
        TransferRequest, TransferResult,
        // Reservations
        ReserveRequest, ReserveResult, ReleaseRequest, ReleaseResult,
        FulfillRequest, FulfillResult,
        // UoM
        Uom, ItemUomConversion, CreateUomRequest, CreateConversionRequest,
        // Locations
        Location, CreateLocationRequest, UpdateLocationRequest,
        // Cycle counts
        CreateTaskRequest, CreateTaskResult, TaskScope, TaskLine,
        SubmitBody, SubmitLineInput, ApproveBody,
        // Reorder policies
        ReorderPolicy, CreateReorderPolicyRequest, UpdateReorderPolicyRequest,
        // Valuation
        ValuationSnapshot, ValuationLine, CreateSnapshotRequest,
        // Revisions
        ItemRevision, CreateRevisionRequest, ActivateRevisionRequest,
        UpdateRevisionPolicyRequest,
        // Labels
        Label, GenerateLabelRequest,
        // Expiry
        LotExpiryRecord, SetLotExpiryRequest, RunExpiryAlertScanRequest,
        RunExpiryAlertScanResult,
        // Lot genealogy
        LotSplitRequest, LotMergeRequest, SplitChild, MergeParent,
        GenealogyResult, GenealogyEdge,
        // Lots & Serials
        InventoryLot,
        // History
        MovementEntry,
        // Status
        InvItemStatus, StatusTransferRequest, StatusTransferResult,
        // Shared envelopes
        ApiError, PaginatedResponse<Item>, PaginatedResponse<Location>,
        PaginatedResponse<Label>, PaginatedResponse<InventoryLot>,
        PaginatedResponse<ReorderPolicy>, PaginatedResponse<ItemRevision>,
        PaginatedResponse<ValuationSnapshot>, PaginationMeta,
    )),
    security(
        ("bearer" = [])
    ),
    modifiers(&SecurityAddon),
)]
struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::HttpBuilder::new()
                    .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!("Starting Inventory service...");

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Inventory service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: host={}, port={}",
        config.host,
        config.port
    );

    tracing::info!("Connecting to database...");
    let pool = resolve_pool(&config.database_url)
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Inventory: failed to run database migrations");

    let shutdown_pool = pool.clone();

    let metrics = Arc::new(InventoryMetrics::new().expect("Failed to create metrics registry"));

    let admin_pool = pool.clone();
    let bus_health = BusHealth::new();
    let event_bus = Arc::new(InMemoryBus::new());
    let app_state = Arc::new(AppState {
        pool,
        metrics,
        event_bus: event_bus.clone(),
        bus_health: bus_health.clone(),
    });

    let supervisor_config = config.clone();
    tokio::spawn(start_event_bus_supervisor(
        supervisor_config,
        app_state.pool.clone(),
        bus_health,
        event_bus,
    ));

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let inv_mutations = Router::new()
        // Item master — write
        .route("/api/inventory/items", axum::routing::post(create_item))
        .route("/api/inventory/items/{id}", axum::routing::put(update_item))
        .route(
            "/api/inventory/items/{id}/deactivate",
            axum::routing::post(deactivate_item),
        )
        .route(
            "/api/inventory/items/{id}/make-buy",
            axum::routing::put(put_make_buy),
        )
        // Stock receipts — write
        .route("/api/inventory/receipts", axum::routing::post(post_receipt))
        // Stock issues — write
        .route("/api/inventory/issues", axum::routing::post(post_issue))
        // Reservations — write
        .route(
            "/api/inventory/reservations/reserve",
            axum::routing::post(post_reserve),
        )
        .route(
            "/api/inventory/reservations/release",
            axum::routing::post(post_release),
        )
        .route(
            "/api/inventory/reservations/{id}/fulfill",
            axum::routing::post(post_fulfill),
        )
        // UoM — write
        .route("/api/inventory/uoms", axum::routing::post(create_uom))
        .route(
            "/api/inventory/items/{id}/uom-conversions",
            axum::routing::post(create_conversion),
        )
        // Stock adjustments — write
        .route(
            "/api/inventory/adjustments",
            axum::routing::post(post_adjustment),
        )
        // Transfers — write
        .route(
            "/api/inventory/transfers",
            axum::routing::post(post_transfer),
        )
        .route(
            "/api/inventory/status-transfers",
            axum::routing::post(post_status_transfer),
        )
        // Cycle counts — write
        .route(
            "/api/inventory/cycle-count-tasks",
            axum::routing::post(post_cycle_count_task),
        )
        .route(
            "/api/inventory/cycle-count-tasks/{task_id}/submit",
            axum::routing::post(post_cycle_count_submit),
        )
        .route(
            "/api/inventory/cycle-count-tasks/{task_id}/approve",
            axum::routing::post(post_cycle_count_approve),
        )
        // Reorder policies — write
        .route(
            "/api/inventory/reorder-policies",
            axum::routing::post(post_reorder_policy),
        )
        .route(
            "/api/inventory/reorder-policies/{id}",
            axum::routing::put(put_reorder_policy),
        )
        // Valuation snapshots — write
        .route(
            "/api/inventory/valuation-snapshots",
            axum::routing::post(post_valuation_snapshot),
        )
        // Item revisions — write
        .route(
            "/api/inventory/items/{item_id}/revisions",
            axum::routing::post(post_create_revision),
        )
        .route(
            "/api/inventory/items/{item_id}/revisions/{revision_id}/activate",
            axum::routing::post(post_activate_revision),
        )
        .route(
            "/api/inventory/items/{item_id}/revisions/{revision_id}/policy-flags",
            axum::routing::put(put_revision_policy),
        )
        // Labels — write
        .route(
            "/api/inventory/items/{item_id}/labels",
            axum::routing::post(post_generate_label),
        )
        // Expiry — write
        .route(
            "/api/inventory/lots/{lot_id}/expiry",
            axum::routing::put(put_lot_expiry),
        )
        .route(
            "/api/inventory/expiry-alerts/scan",
            axum::routing::post(post_expiry_alert_scan),
        )
        // Lot genealogy — write
        .route(
            "/api/inventory/lots/split",
            axum::routing::post(post_lot_split),
        )
        .route(
            "/api/inventory/lots/merge",
            axum::routing::post(post_lot_merge),
        )
        // Locations — write
        .route(
            "/api/inventory/locations",
            axum::routing::post(create_location),
        )
        .route(
            "/api/inventory/locations/{id}",
            axum::routing::put(update_location),
        )
        .route(
            "/api/inventory/locations/{id}/deactivate",
            axum::routing::post(deactivate_location),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::INVENTORY_MUTATE,
        ]))
        .with_state(app_state.clone());

    let inv_reads = Router::new()
        // Item master — read
        .route("/api/inventory/items", axum::routing::get(list_items))
        .route("/api/inventory/items/{id}", axum::routing::get(get_item))
        // UoM — read
        .route("/api/inventory/uoms", axum::routing::get(list_uoms))
        .route(
            "/api/inventory/items/{id}/uom-conversions",
            axum::routing::get(list_conversions),
        )
        // Movement history — read
        .route(
            "/api/inventory/items/{item_id}/history",
            axum::routing::get(get_movement_history),
        )
        // Lot / serial queries — read
        .route(
            "/api/inventory/items/{item_id}/lots",
            axum::routing::get(get_lots_for_item),
        )
        .route(
            "/api/inventory/items/{item_id}/serials",
            axum::routing::get(get_serials_for_item),
        )
        .route(
            "/api/inventory/items/{item_id}/lots/{lot_code}/trace",
            axum::routing::get(trace_lot_handler),
        )
        .route(
            "/api/inventory/items/{item_id}/serials/{serial_code}/trace",
            axum::routing::get(trace_serial_handler),
        )
        // Item revisions — read
        .route(
            "/api/inventory/items/{item_id}/revisions/at",
            axum::routing::get(get_revision_at),
        )
        .route(
            "/api/inventory/items/{item_id}/revisions",
            axum::routing::get(get_list_revisions),
        )
        // Labels — read
        .route(
            "/api/inventory/items/{item_id}/labels",
            axum::routing::get(get_list_labels),
        )
        .route(
            "/api/inventory/labels/{label_id}",
            axum::routing::get(get_label_by_id),
        )
        // Reorder policies — read
        .route(
            "/api/inventory/reorder-policies/{id}",
            axum::routing::get(get_reorder_policy),
        )
        .route(
            "/api/inventory/items/{item_id}/reorder-policies",
            axum::routing::get(list_reorder_policies),
        )
        // Valuation snapshots — read
        .route(
            "/api/inventory/valuation-snapshots",
            axum::routing::get(list_valuation_snapshots),
        )
        .route(
            "/api/inventory/valuation-snapshots/{id}",
            axum::routing::get(get_valuation_snapshot),
        )
        // Lot genealogy — read
        .route(
            "/api/inventory/lots/{lot_id}/children",
            axum::routing::get(get_lot_children),
        )
        .route(
            "/api/inventory/lots/{lot_id}/parents",
            axum::routing::get(get_lot_parents),
        )
        // Locations — read
        .route(
            "/api/inventory/locations/{id}",
            axum::routing::get(get_location),
        )
        .route(
            "/api/inventory/warehouses/{warehouse_id}/locations",
            axum::routing::get(list_locations),
        )
        .route_layer(RequirePermissionsLayer::new(&[permissions::INVENTORY_READ]))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(health))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/api/openapi.json", get(openapi_json))
        .route("/metrics", get(metrics_handler))
        .with_state(app_state)
        .merge(inv_reads)
        .merge(inv_mutations)
        .merge(inventory_rs::http::admin::admin_router(admin_pool))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(
            security::tracing::tracing_context_middleware,
        ))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(
            maybe_verifier,
            optional_claims_mw,
        ))
        .layer(build_cors_layer(&config))
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("Inventory service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server failed to start");

    tracing::info!("Server stopped — closing resources");
    shutdown_pool.close().await;
    tracing::info!("Shutdown complete");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received — draining in-flight requests");
}

fn build_cors_layer(config: &Config) -> CorsLayer {
    let is_wildcard = config.cors_origins.len() == 1 && config.cors_origins[0] == "*";

    if is_wildcard && config.env != "development" {
        tracing::warn!(
            "CORS_ORIGINS is set to wildcard — restrict to specific origins in production"
        );
    }

    let layer = if is_wildcard {
        CorsLayer::new().allow_origin(AllowOrigin::any())
    } else {
        let origins: Vec<_> = config
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new().allow_origin(origins)
    };

    layer
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}

async fn start_event_bus_supervisor(
    config: Config,
    pool: sqlx::PgPool,
    bus_health: Arc<BusHealth>,
    fallback_bus: Arc<dyn EventBus>,
) {
    match config.bus_type {
        BusType::InMemory => {
            bus_health.mark_connected(0);
            component_issue_consumer::start_component_issue_consumer(fallback_bus.clone(), pool.clone())
                .await;
            fg_receipt_consumer::start_fg_receipt_consumer(fallback_bus.clone(), pool.clone()).await;
            // TODO(bd-rbhj1): outbox publisher not yet implemented in event_bus crate
            let _ = (pool, fallback_bus);
        }
        BusType::Nats => {
            let nats_url = config
                .nats_url
                .clone()
                .unwrap_or_else(|| "nats://localhost:4222".to_string());
            let mut attempt = 0;
            loop {
                attempt += 1;
                let start = Instant::now();
                match connect_nats(&nats_url).await {
                    Ok(client) => {
                        let bus = Arc::new(NatsBus::new(client));
                        let latency = start.elapsed().as_millis() as u64;
                        bus_health.mark_connected(latency);
                        component_issue_consumer::start_component_issue_consumer(
                            bus.clone(),
                            pool.clone(),
                        )
                        .await;
                        fg_receipt_consumer::start_fg_receipt_consumer(bus.clone(), pool.clone())
                            .await;
                        // TODO(bd-rbhj1): outbox publisher not yet implemented in event_bus crate
                        let _ = (pool, bus);
                        break;
                    }
                    Err(err) => {
                        let latency = start.elapsed().as_millis() as u64;
                        bus_health.mark_disconnected(Some(err.to_string()), latency);
                        tracing::warn!(
                            error = %err,
                            url = %nats_url,
                            attempt,
                            "Unable to connect to NATS, retrying in 5s"
                        );
                        sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cors_wildcard_parses() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8092,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            bus_type: BusType::InMemory,
            nats_url: None,
        };
        let _layer = build_cors_layer(&config);
    }

    #[test]
    fn cors_specific_origins_parse() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8092,
            env: "development".to_string(),
            cors_origins: vec![
                "http://localhost:3000".to_string(),
                "https://app.example.com".to_string(),
            ],
            bus_type: BusType::InMemory,
            nats_url: None,
        };
        let _layer = build_cors_layer(&config);
    }
}
