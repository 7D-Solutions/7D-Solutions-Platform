//! Utility binary that prints the Inventory OpenAPI spec as JSON to stdout.
//! No database or NATS connection required — the spec is generated at compile time.
//!
//! Usage:  cargo run --bin openapi_dump > openapi.json

use utoipa::OpenApi;

use inventory_rs::domain::{
    adjust_service::{AdjustRequest, AdjustResult},
    cycle_count::{
        submit_service::SubmitLineInput,
        task_service::{CreateTaskRequest, CreateTaskResult, TaskLine, TaskScope},
    },
    expiry::{
        LotExpiryRecord, RunExpiryAlertScanRequest, RunExpiryAlertScanResult, SetLotExpiryRequest,
    },
    fulfill_service::{FulfillRequest, FulfillResult},
    genealogy::GenealogyEdge,
    genealogy::{GenealogyResult, LotMergeRequest, LotSplitRequest, MergeParent, SplitChild},
    history::query::MovementEntry,
    issue_service::{IssueRequest, IssueResult},
    items::{CreateItemRequest, Item, ListItemsQuery, TrackingMode, UpdateItemRequest},
    labels::{GenerateLabelRequest, Label},
    locations::{CreateLocationRequest, Location, UpdateLocationRequest},
    lots_serials::models::InventoryLot,
    make_buy::SetMakeBuyRequest,
    receipt_service::{ReceiptRequest, ReceiptResult},
    reorder::models::{CreateReorderPolicyRequest, ReorderPolicy, UpdateReorderPolicyRequest},
    reservation_service::{ReleaseRequest, ReleaseResult, ReserveRequest, ReserveResult},
    revisions::{
        ActivateRevisionRequest, CreateRevisionRequest, ItemRevision, UpdateRevisionPolicyRequest,
    },
    status::{
        models::InvItemStatus,
        transfer_service::{StatusTransferRequest, StatusTransferResult},
    },
    transfer_service::{TransferRequest, TransferResult},
    uom::models::{CreateConversionRequest, CreateUomRequest, ItemUomConversion, Uom},
    valuation::{
        models::{ValuationLine, ValuationSnapshot},
        snapshot_service::CreateSnapshotRequest,
    },
};
use inventory_rs::events::contracts::{ConsumedLayer, SourceRef};
use inventory_rs::http::batch_receipts::{BatchReceiptRequest, BatchReceiptResponse, BatchReceiptItemResult};
use inventory_rs::http::cycle_counts::{ApproveBody, SubmitBody};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Inventory Service",
        version = "2.4.1",
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

fn main() {
    let spec = ApiDoc::openapi();
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}
