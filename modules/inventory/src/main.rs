use axum::{routing::get, Json, Router};
use std::sync::Arc;
use utoipa::OpenApi;

use inventory_rs::{
    AppState, BusHealth,
    consumers::{component_issue_consumer, fg_receipt_consumer},
    http::{
        adjustments::post_adjustment,
        imports::import_items,
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
    metrics::InventoryMetrics,
};
use inventory_rs::domain::items::ListItemsQuery;
use inventory_rs::http::batch_receipts::{BatchReceiptRequest, BatchReceiptResponse, BatchReceiptItemResult};
use inventory_rs::domain::genealogy::GenealogyEdge;
use inventory_rs::domain::status::models::InvItemStatus;
use inventory_rs::http::cycle_counts::{ApproveBody, SubmitBody};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};
use platform_sdk::ModuleBuilder;
use security::{permissions, RequirePermissionsLayer};

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
        inventory_rs::http::items::create_item,
        inventory_rs::http::items::get_item,
        inventory_rs::http::items::update_item,
        inventory_rs::http::items::list_items,
        inventory_rs::http::items::deactivate_item,
        inventory_rs::http::make_buy::put_make_buy,
        inventory_rs::http::receipts::post_receipt,
        inventory_rs::http::issues::post_issue,
        inventory_rs::http::adjustments::post_adjustment,
        inventory_rs::http::transfers::post_transfer,
        inventory_rs::http::reservations::post_reserve,
        inventory_rs::http::reservations::post_release,
        inventory_rs::http::reservations::post_fulfill,
        inventory_rs::http::uom::create_uom,
        inventory_rs::http::uom::list_uoms,
        inventory_rs::http::uom::create_conversion,
        inventory_rs::http::uom::list_conversions,
        inventory_rs::http::locations::create_location,
        inventory_rs::http::locations::get_location,
        inventory_rs::http::locations::update_location,
        inventory_rs::http::locations::deactivate_location,
        inventory_rs::http::locations::list_locations,
        inventory_rs::http::cycle_counts::post_cycle_count_task,
        inventory_rs::http::cycle_counts::post_cycle_count_submit,
        inventory_rs::http::cycle_counts::post_cycle_count_approve,
        inventory_rs::http::reorder::post_reorder_policy,
        inventory_rs::http::reorder::get_reorder_policy,
        inventory_rs::http::reorder::put_reorder_policy,
        inventory_rs::http::reorder::list_reorder_policies,
        inventory_rs::http::valuation::post_valuation_snapshot,
        inventory_rs::http::valuation::list_valuation_snapshots,
        inventory_rs::http::valuation::get_valuation_snapshot,
        inventory_rs::http::revisions::post_create_revision,
        inventory_rs::http::revisions::post_activate_revision,
        inventory_rs::http::revisions::put_revision_policy,
        inventory_rs::http::revisions::get_revision_at,
        inventory_rs::http::revisions::get_list_revisions,
        inventory_rs::http::labels::post_generate_label,
        inventory_rs::http::labels::get_list_labels,
        inventory_rs::http::labels::get_label_by_id,
        inventory_rs::http::expiry::put_lot_expiry,
        inventory_rs::http::expiry::post_expiry_alert_scan,
        inventory_rs::http::genealogy::post_lot_split,
        inventory_rs::http::genealogy::post_lot_merge,
        inventory_rs::http::genealogy::get_lot_children,
        inventory_rs::http::genealogy::get_lot_parents,
        inventory_rs::http::lots::get_lots_for_item,
        inventory_rs::http::serials::get_serials_for_item,
        inventory_rs::http::trace::trace_lot_handler,
        inventory_rs::http::trace::trace_serial_handler,
        inventory_rs::http::history::get_movement_history,
        inventory_rs::http::status::post_status_transfer,
        inventory_rs::http::admin::projection_status,
        inventory_rs::http::admin::consistency_check,
        inventory_rs::http::admin::list_projections,
        inventory_rs::http::batch_receipts::post_batch_receipts,
    ),
    components(schemas(
        Item, TrackingMode, CreateItemRequest, UpdateItemRequest, ListItemsQuery,
        SetMakeBuyRequest,
        ReceiptRequest, ReceiptResult,
        IssueRequest, IssueResult, ConsumedLayer, SourceRef,
        AdjustRequest, AdjustResult,
        TransferRequest, TransferResult,
        ReserveRequest, ReserveResult, ReleaseRequest, ReleaseResult,
        FulfillRequest, FulfillResult,
        Uom, ItemUomConversion, CreateUomRequest, CreateConversionRequest,
        Location, CreateLocationRequest, UpdateLocationRequest,
        CreateTaskRequest, CreateTaskResult, TaskScope, TaskLine,
        SubmitBody, SubmitLineInput, ApproveBody,
        ReorderPolicy, CreateReorderPolicyRequest, UpdateReorderPolicyRequest,
        ValuationSnapshot, ValuationLine, CreateSnapshotRequest,
        ItemRevision, CreateRevisionRequest, ActivateRevisionRequest,
        UpdateRevisionPolicyRequest,
        Label, GenerateLabelRequest,
        LotExpiryRecord, SetLotExpiryRequest, RunExpiryAlertScanRequest,
        RunExpiryAlertScanResult,
        LotSplitRequest, LotMergeRequest, SplitChild, MergeParent,
        GenealogyResult, GenealogyEdge,
        InventoryLot,
        MovementEntry,
        InvItemStatus, StatusTransferRequest, StatusTransferResult,
        BatchReceiptRequest, BatchReceiptResponse, BatchReceiptItemResult,
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

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes_async(|ctx| async move {
            let pool = ctx.pool().clone();

            let metrics = Arc::new(
                InventoryMetrics::new().expect("Failed to create metrics registry"),
            );
            // Register SLO metrics with global prometheus registry so
            // SDK's /metrics endpoint picks them up.
            let _ = prometheus::register(Box::new(
                metrics.http_request_duration_seconds.clone(),
            ));
            let _ = prometheus::register(Box::new(
                metrics.http_requests_total.clone(),
            ));

            // Get event bus from SDK context — replaces the bus supervisor.
            // SDK ensures bus is connected before routes are built.
            let bus = ctx.bus_arc().expect("Inventory requires event bus");
            let bus_health = BusHealth::new();
            bus_health.mark_connected(0);

            // Start consumers using SDK's bus
            component_issue_consumer::start_component_issue_consumer(
                bus.clone(),
                pool.clone(),
            )
            .await;
            fg_receipt_consumer::start_fg_receipt_consumer(bus.clone(), pool.clone()).await;

            let admin_pool = pool.clone();
            let app_state = Arc::new(AppState {
                pool,
                metrics,
                event_bus: bus,
                bus_health,
            });

            let inv_mutations = Router::new()
                .route("/api/inventory/items", axum::routing::post(create_item))
                .route(
                    "/api/inventory/import/items",
                    axum::routing::post(import_items),
                )
                .route("/api/inventory/items/{id}", axum::routing::put(update_item))
                .route(
                    "/api/inventory/items/{id}/deactivate",
                    axum::routing::post(deactivate_item),
                )
                .route(
                    "/api/inventory/items/{id}/make-buy",
                    axum::routing::put(put_make_buy),
                )
                .route("/api/inventory/receipts", axum::routing::post(post_receipt))
                .route("/api/inventory/issues", axum::routing::post(post_issue))
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
                .route("/api/inventory/uoms", axum::routing::post(create_uom))
                .route(
                    "/api/inventory/items/{id}/uom-conversions",
                    axum::routing::post(create_conversion),
                )
                .route(
                    "/api/inventory/adjustments",
                    axum::routing::post(post_adjustment),
                )
                .route(
                    "/api/inventory/transfers",
                    axum::routing::post(post_transfer),
                )
                .route(
                    "/api/inventory/status-transfers",
                    axum::routing::post(post_status_transfer),
                )
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
                .route(
                    "/api/inventory/reorder-policies",
                    axum::routing::post(post_reorder_policy),
                )
                .route(
                    "/api/inventory/reorder-policies/{id}",
                    axum::routing::put(put_reorder_policy),
                )
                .route(
                    "/api/inventory/valuation-snapshots",
                    axum::routing::post(post_valuation_snapshot),
                )
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
                .route(
                    "/api/inventory/items/{item_id}/labels",
                    axum::routing::post(post_generate_label),
                )
                .route(
                    "/api/inventory/lots/{lot_id}/expiry",
                    axum::routing::put(put_lot_expiry),
                )
                .route(
                    "/api/inventory/expiry-alerts/scan",
                    axum::routing::post(post_expiry_alert_scan),
                )
                .route(
                    "/api/inventory/lots/split",
                    axum::routing::post(post_lot_split),
                )
                .route(
                    "/api/inventory/lots/merge",
                    axum::routing::post(post_lot_merge),
                )
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
                .route("/api/inventory/items", axum::routing::get(list_items))
                .route("/api/inventory/items/{id}", axum::routing::get(get_item))
                .route("/api/inventory/uoms", axum::routing::get(list_uoms))
                .route(
                    "/api/inventory/items/{id}/uom-conversions",
                    axum::routing::get(list_conversions),
                )
                .route(
                    "/api/inventory/items/{item_id}/history",
                    axum::routing::get(get_movement_history),
                )
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
                .route(
                    "/api/inventory/items/{item_id}/revisions/at",
                    axum::routing::get(get_revision_at),
                )
                .route(
                    "/api/inventory/items/{item_id}/revisions",
                    axum::routing::get(get_list_revisions),
                )
                .route(
                    "/api/inventory/items/{item_id}/labels",
                    axum::routing::get(get_list_labels),
                )
                .route(
                    "/api/inventory/labels/{label_id}",
                    axum::routing::get(get_label_by_id),
                )
                .route(
                    "/api/inventory/reorder-policies/{id}",
                    axum::routing::get(get_reorder_policy),
                )
                .route(
                    "/api/inventory/items/{item_id}/reorder-policies",
                    axum::routing::get(list_reorder_policies),
                )
                .route(
                    "/api/inventory/valuation-snapshots",
                    axum::routing::get(list_valuation_snapshots),
                )
                .route(
                    "/api/inventory/valuation-snapshots/{id}",
                    axum::routing::get(get_valuation_snapshot),
                )
                .route(
                    "/api/inventory/lots/{lot_id}/children",
                    axum::routing::get(get_lot_children),
                )
                .route(
                    "/api/inventory/lots/{lot_id}/parents",
                    axum::routing::get(get_lot_parents),
                )
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

            Router::new()
                .route("/api/openapi.json", get(openapi_json))
                .with_state(app_state)
                .merge(inv_reads)
                .merge(inv_mutations)
                .merge(inventory_rs::http::admin::admin_router(admin_pool))
        })
        .run()
        .await
        .expect("inventory module failed");
}
