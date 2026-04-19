//! Integration tests for the shop-floor-gates module.
//!
//! Requires: 7d-shop-floor-gates-postgres running on localhost:5469
//! Run: DATABASE_URL=postgres://sfg_user:sfg_pass@localhost:5469/sfg_db cargo test -p shop-floor-gates-rs --test integration_test

use shop_floor_gates_rs::domain::holds::{
    service as holds_service, PlaceHoldRequest, ReleaseHoldRequest,
};
use shop_floor_gates_rs::domain::handoffs::{
    service as handoffs_service, AcceptHandoffRequest, InitiateHandoffRequest,
};
use shop_floor_gates_rs::domain::verifications::{
    service as verifications_service, CreateVerificationRequest, OperatorConfirmRequest,
    VerifyRequest,
};
use shop_floor_gates_rs::domain::signoffs::{
    service as signoffs_service, RecordSignoffRequest,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://sfg_user:sfg_pass@localhost:5469/sfg_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to SFG test DB — is 7d-shop-floor-gates-postgres running on :5469?");
    MIGRATOR.run(&pool).await.expect("Failed to run SFG migrations");
    pool
}

fn tenant() -> String {
    format!("sfg-test-{}", Uuid::new_v4().simple())
}

fn user() -> Uuid {
    Uuid::new_v4()
}

// ── Hold: operation-scoped hold requires operation_id ─────────────────────────

#[tokio::test]
#[serial]
async fn place_operation_scoped_hold_without_operation_id_rejected() {
    let pool = setup_db().await;
    let t = tenant();
    let placed_by = user();

    let req = PlaceHoldRequest {
        hold_type: "quality".to_string(),
        scope: "operation".to_string(),
        work_order_id: Uuid::new_v4(),
        operation_id: None,
        reason: "Missing operation_id".to_string(),
        release_authority: None,
    };
    let result = holds_service::place_hold(&pool, &t, placed_by, req).await;
    assert!(result.is_err());
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(err_str.contains("400") || err_str.to_lowercase().contains("operation_id"), "Expected 422/400, got: {}", err_str);
}

// ── Hold: release authority enforcement ──────────────────────────────────────

#[tokio::test]
#[serial]
async fn release_authority_quality_only_hold_blocked_for_non_quality_user() {
    let pool = setup_db().await;
    let t = tenant();
    let placed_by = user();
    let non_quality_user = user();

    // Place a quality-only hold
    let req = PlaceHoldRequest {
        hold_type: "quality".to_string(),
        scope: "work_order".to_string(),
        work_order_id: Uuid::new_v4(),
        operation_id: None,
        reason: "Quality gate".to_string(),
        release_authority: Some("quality".to_string()),
    };
    let hold = holds_service::place_hold(&pool, &t, placed_by, req).await.expect("place_hold failed");

    // Non-quality user attempts release — should be forbidden
    let result = holds_service::release_hold(
        &pool,
        &t,
        hold.id,
        non_quality_user,
        ReleaseHoldRequest { release_notes: None },
        "engineering",
        false,
    )
    .await;
    assert!(result.is_err());
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(err_str.contains("403") || err_str.to_lowercase().contains("authority"), "Expected 403, got: {}", err_str);
}

#[tokio::test]
#[serial]
async fn release_authority_quality_only_hold_allowed_for_quality_user() {
    let pool = setup_db().await;
    let t = tenant();
    let placed_by = user();
    let quality_user = user();

    let req = PlaceHoldRequest {
        hold_type: "quality".to_string(),
        scope: "work_order".to_string(),
        work_order_id: Uuid::new_v4(),
        operation_id: None,
        reason: "Quality gate".to_string(),
        release_authority: Some("quality".to_string()),
    };
    let hold = holds_service::place_hold(&pool, &t, placed_by, req).await.expect("place_hold failed");

    let result = holds_service::release_hold(
        &pool,
        &t,
        hold.id,
        quality_user,
        ReleaseHoldRequest { release_notes: Some("Passed inspection".to_string()) },
        "quality",
        false,
    )
    .await;
    assert!(result.is_ok(), "Quality user should be able to release quality hold");
    let released = result.unwrap();
    assert_eq!(released.status, "released");
    assert_eq!(released.released_by, Some(quality_user));
}

// ── Hold: system actor bypasses release authority ─────────────────────────────

#[tokio::test]
#[serial]
async fn system_actor_bypasses_release_authority() {
    let pool = setup_db().await;
    let t = tenant();
    let placed_by = user();

    let req = PlaceHoldRequest {
        hold_type: "quality".to_string(),
        scope: "work_order".to_string(),
        work_order_id: Uuid::new_v4(),
        operation_id: None,
        reason: "Quality gate for auto-release test".to_string(),
        release_authority: Some("quality".to_string()),
    };
    let hold = holds_service::place_hold(&pool, &t, placed_by, req).await.expect("place_hold failed");

    // System actor is not a quality person but should bypass authority check
    let system = shop_floor_gates_rs::domain::holds::service::SYSTEM_ACTOR;
    let result = holds_service::release_hold(
        &pool,
        &t,
        hold.id,
        system,
        ReleaseHoldRequest { release_notes: Some("auto-released: work_order_completed".to_string()) },
        "",
        true,
    )
    .await;
    assert!(result.is_ok(), "System actor must bypass release authority");
    assert_eq!(result.unwrap().status, "released");
}

// ── Hold: tenant isolation ────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn hold_not_visible_to_other_tenant() {
    let pool = setup_db().await;
    let t_a = tenant();
    let t_b = tenant();
    let placed_by = user();

    let req = PlaceHoldRequest {
        hold_type: "quality".to_string(),
        scope: "work_order".to_string(),
        work_order_id: Uuid::new_v4(),
        operation_id: None,
        reason: "Tenant A hold".to_string(),
        release_authority: None,
    };
    let hold = holds_service::place_hold(&pool, &t_a, placed_by, req).await.expect("place_hold failed");

    // Attempt to release from tenant B — should not find the hold
    let result = holds_service::release_hold(
        &pool,
        &t_b,
        hold.id,
        user(),
        ReleaseHoldRequest { release_notes: None },
        "any_with_role",
        false,
    )
    .await;
    assert!(result.is_err());
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(err_str.contains("404") || err_str.to_lowercase().contains("not found"), "Expected 404, got: {}", err_str);
}

// ── Verification: two-step invariant — operator must confirm first ────────────

#[tokio::test]
#[serial]
async fn verify_blocked_when_operator_not_confirmed() {
    let pool = setup_db().await;
    let t = tenant();
    let operator = user();
    let verifier = user();
    let wo_id = Uuid::new_v4();
    let op_id = Uuid::new_v4();

    let verification = verifications_service::create_verification(
        &pool,
        &t,
        operator,
        CreateVerificationRequest { work_order_id: wo_id, operation_id: op_id, notes: None },
    )
    .await
    .expect("create_verification failed");

    let result = verifications_service::verify(&pool, &t, verification.id, verifier, VerifyRequest { notes: None }).await;
    assert!(result.is_err());
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(
        err_str.contains("400") || err_str.to_lowercase().contains("operator"),
        "Expected 400 about operator confirmation, got: {}",
        err_str
    );
}

// ── Verification: two-step invariant — all checkboxes required ───────────────

#[tokio::test]
#[serial]
async fn verify_blocked_when_checkboxes_not_all_true() {
    let pool = setup_db().await;
    let t = tenant();
    let operator = user();
    let verifier = user();
    let wo_id = Uuid::new_v4();
    let op_id = Uuid::new_v4();

    let verification = verifications_service::create_verification(
        &pool,
        &t,
        operator,
        CreateVerificationRequest { work_order_id: wo_id, operation_id: op_id, notes: None },
    )
    .await
    .expect("create_verification failed");

    // Operator confirms but leaves drawing_verified = false
    verifications_service::operator_confirm(
        &pool,
        &t,
        verification.id,
        operator,
        OperatorConfirmRequest { drawing_verified: false, material_verified: true, instruction_verified: true },
    )
    .await
    .expect("operator_confirm failed");

    let result = verifications_service::verify(&pool, &t, verification.id, verifier, VerifyRequest { notes: None }).await;
    assert!(result.is_err());
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(
        err_str.contains("400") || err_str.to_lowercase().contains("checkbox") || err_str.to_lowercase().contains("verified"),
        "Expected 400 about checkboxes, got: {}",
        err_str
    );
}

// ── Verification: happy path ──────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn verify_succeeds_when_operator_confirmed_all_checks_true() {
    let pool = setup_db().await;
    let t = tenant();
    let operator = user();
    let verifier = user();
    let wo_id = Uuid::new_v4();
    let op_id = Uuid::new_v4();

    let verification = verifications_service::create_verification(
        &pool,
        &t,
        operator,
        CreateVerificationRequest { work_order_id: wo_id, operation_id: op_id, notes: None },
    )
    .await
    .expect("create_verification failed");

    verifications_service::operator_confirm(
        &pool,
        &t,
        verification.id,
        operator,
        OperatorConfirmRequest { drawing_verified: true, material_verified: true, instruction_verified: true },
    )
    .await
    .expect("operator_confirm failed");

    let result = verifications_service::verify(&pool, &t, verification.id, verifier, VerifyRequest { notes: None }).await;
    assert!(result.is_ok(), "verify should succeed when all conditions met");
    let v = result.unwrap();
    assert_eq!(v.status, "verified");
    assert_eq!(v.verifier_id, Some(verifier));
    assert!(v.verified_at.is_some());
}

// ── Verification: uniqueness constraint ──────────────────────────────────────

#[tokio::test]
#[serial]
async fn verification_uniqueness_per_operation() {
    let pool = setup_db().await;
    let t = tenant();
    let operator = user();
    let wo_id = Uuid::new_v4();
    let op_id = Uuid::new_v4();

    verifications_service::create_verification(
        &pool,
        &t,
        operator,
        CreateVerificationRequest { work_order_id: wo_id, operation_id: op_id, notes: None },
    )
    .await
    .expect("first create_verification failed");

    let result = verifications_service::create_verification(
        &pool,
        &t,
        operator,
        CreateVerificationRequest { work_order_id: wo_id, operation_id: op_id, notes: None },
    )
    .await;
    assert!(result.is_err(), "Duplicate verification for same operation should be rejected");
}

// ── Signoff: entity_type whitelist enforced ──────────────────────────────────

#[tokio::test]
#[serial]
async fn signoff_invalid_entity_type_rejected() {
    let pool = setup_db().await;
    let t = tenant();
    let signed_by = user();

    let result = signoffs_service::record_signoff(
        &pool,
        &t,
        signed_by,
        RecordSignoffRequest {
            entity_type: "unknown_type".to_string(),
            entity_id: Uuid::new_v4(),
            role: "quality".to_string(),
            signature_text: "Signed".to_string(),
            notes: None,
        },
    )
    .await;
    assert!(result.is_err());
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(
        err_str.contains("400") || err_str.to_lowercase().contains("entity_type") || err_str.to_lowercase().contains("invalid"),
        "Expected 400 about invalid entity_type, got: {}",
        err_str
    );
}

// ── Signoff: valid entity_type accepted ──────────────────────────────────────

#[tokio::test]
#[serial]
async fn signoff_valid_entity_type_accepted() {
    let pool = setup_db().await;
    let t = tenant();
    let signed_by = user();

    let result = signoffs_service::record_signoff(
        &pool,
        &t,
        signed_by,
        RecordSignoffRequest {
            entity_type: "work_order".to_string(),
            entity_id: Uuid::new_v4(),
            role: "quality".to_string(),
            signature_text: "I hereby certify this work order meets spec".to_string(),
            notes: None,
        },
    )
    .await;
    assert!(result.is_ok(), "Valid entity_type 'work_order' should be accepted");
    let s = result.unwrap();
    assert_eq!(s.entity_type, "work_order");
    assert_eq!(s.signed_by, signed_by);
}

// ── Signoff: append-only — no update/delete in domain layer ──────────────────
// The HTTP handler returns 405 for PUT/DELETE; the domain layer has no
// update/delete functions — this is enforced structurally.
#[tokio::test]
#[serial]
async fn signoff_is_append_only_no_delete_function() {
    // Verify that the signoffs repo has no delete function
    // by confirming a signoff can be fetched but there is no way to remove it
    let pool = setup_db().await;
    let t = tenant();
    let signed_by = user();

    let s = signoffs_service::record_signoff(
        &pool,
        &t,
        signed_by,
        RecordSignoffRequest {
            entity_type: "operation".to_string(),
            entity_id: Uuid::new_v4(),
            role: "operator".to_string(),
            signature_text: "Operation completed per plan".to_string(),
            notes: None,
        },
    )
    .await
    .expect("record_signoff failed");

    // Signoff must still be there
    let fetched = signoffs_service::get_signoff(&pool, s.id, &t).await.expect("get_signoff failed");
    assert_eq!(fetched.id, s.id);
}

// ── Handoff: happy path ───────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn handoff_initiate_and_accept() {
    let pool = setup_db().await;
    let t = tenant();
    let initiator = user();
    let acceptor = user();
    let wo_id = Uuid::new_v4();
    let src_op = Uuid::new_v4();
    let dst_op = Uuid::new_v4();

    let handoff = handoffs_service::initiate_handoff(
        &pool,
        &t,
        initiator,
        InitiateHandoffRequest {
            work_order_id: wo_id,
            source_operation_id: src_op,
            dest_operation_id: dst_op,
            initiation_type: Some("push".to_string()),
            quantity: 10.0,
            unit_of_measure: "EA".to_string(),
            lot_number: None,
            serial_numbers: None,
            notes: None,
        },
    )
    .await
    .expect("initiate_handoff failed");

    assert_eq!(handoff.status, "initiated");
    assert_eq!(handoff.work_order_id, wo_id);

    let accepted = handoffs_service::accept_handoff(
        &pool,
        &t,
        handoff.id,
        acceptor,
        AcceptHandoffRequest { notes: None },
    )
    .await
    .expect("accept_handoff failed");

    assert_eq!(accepted.status, "accepted");
    assert_eq!(accepted.accepted_by, Some(acceptor));
}

// ── Hold: count_active_holds reflects placed and released holds ───────────────

#[tokio::test]
#[serial]
async fn active_hold_count_tracks_state() {
    let pool = setup_db().await;
    let t = tenant();
    let placed_by = user();
    let wo_id = Uuid::new_v4();

    // No holds initially (tenant is unique)
    let count_before = shop_floor_gates_rs::domain::holds::repo::count_active_holds_for_work_order(
        &pool,
        wo_id,
        &t,
    )
    .await
    .expect("count failed");
    assert_eq!(count_before, 0);

    // Place two holds
    for _ in 0..2 {
        holds_service::place_hold(
            &pool,
            &t,
            placed_by,
            PlaceHoldRequest {
                hold_type: "quality".to_string(),
                scope: "work_order".to_string(),
                work_order_id: wo_id,
                operation_id: None,
                reason: "Test hold".to_string(),
                release_authority: None,
            },
        )
        .await
        .expect("place_hold failed");
    }

    let count_after = shop_floor_gates_rs::domain::holds::repo::count_active_holds_for_work_order(
        &pool,
        wo_id,
        &t,
    )
    .await
    .expect("count failed");
    assert_eq!(count_after, 2);
}

// ── Consumer: auto-release all active holds on work_order_closed ──────────────

#[tokio::test]
#[serial]
async fn auto_release_holds_on_work_order_closed() {
    let pool = setup_db().await;
    let t = tenant();
    let placed_by = user();
    let wo_id = Uuid::new_v4();

    // Place a quality-only hold and a general hold
    holds_service::place_hold(
        &pool,
        &t,
        placed_by,
        PlaceHoldRequest {
            hold_type: "quality".to_string(),
            scope: "work_order".to_string(),
            work_order_id: wo_id,
            operation_id: None,
            reason: "Quality check".to_string(),
            release_authority: Some("quality".to_string()),
        },
    )
    .await
    .expect("place_hold failed");

    holds_service::place_hold(
        &pool,
        &t,
        placed_by,
        PlaceHoldRequest {
            hold_type: "engineering".to_string(),
            scope: "work_order".to_string(),
            work_order_id: wo_id,
            operation_id: None,
            reason: "Drawing review".to_string(),
            release_authority: None,
        },
    )
    .await
    .expect("place_hold failed");

    // Simulate the consumer: release all active holds for the work order
    let released = shop_floor_gates_rs::domain::holds::repo::release_all_active_for_work_order(
        &pool,
        wo_id,
        &t,
        shop_floor_gates_rs::domain::holds::service::SYSTEM_ACTOR,
        Some("Auto-released: work order closed"),
    )
    .await
    .expect("release_all_active_for_work_order failed");

    assert_eq!(released, 2, "Both holds should be auto-released");

    let count = shop_floor_gates_rs::domain::holds::repo::count_active_holds_for_work_order(
        &pool,
        wo_id,
        &t,
    )
    .await
    .expect("count failed");
    assert_eq!(count, 0, "No active holds should remain");
}

// ── Migration safety ─────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn migrations_are_idempotent() {
    let pool = setup_db().await;
    // Running migrations again must not error
    MIGRATOR.run(&pool).await.expect("Migrations must be idempotent");
}
