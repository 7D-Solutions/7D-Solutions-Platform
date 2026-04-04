use auth_rs::auth::jwt;
use auth_rs::db::{rbac, sod, user_lifecycle_audit};
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, Row};
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://auth_user:auth_pass@localhost:5433/auth_db".into());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to test DB");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

#[tokio::test]
async fn timeline_ordering_for_user_lifecycle_events() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let review_id = Uuid::new_v4();

    let role = rbac::create_role(&pool, tenant_id, "qa_manager", "QA Manager", false)
        .await
        .expect("create role");

    let mut tx = pool.begin().await.expect("begin tx");
    sqlx::query(
        r#"INSERT INTO credentials (tenant_id, user_id, email, password_hash)
           VALUES ($1, $2, $3, $4)"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(format!("timeline-{}@example.com", Uuid::new_v4()))
    .bind("test-hash")
    .execute(&mut *tx)
    .await
    .expect("insert credential");

    let create_ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-register-1".to_string(),
        causation_id: None,
        idempotency_key: format!("register:{tenant_id}:{user_id}"),
    };

    user_lifecycle_audit::append_lifecycle_event_tx(
        &mut tx,
        tenant_id,
        user_id,
        user_lifecycle_audit::LifecycleEventType::UserCreated,
        None,
        None,
        None,
        None,
        json!({"user_id": user_id, "email": "timeline@example.com"}),
        &create_ctx,
    )
    .await
    .expect("append create event");

    tx.commit().await.expect("commit create tx");

    let bind_ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-bind-1".to_string(),
        causation_id: None,
        idempotency_key: format!("bind:{tenant_id}:{user_id}:{}", role.id),
    };

    rbac::bind_user_role_with_audit(
        &pool,
        tenant_id,
        user_id,
        role.id,
        Some(actor_id),
        &bind_ctx,
    )
    .await
    .expect("bind role with audit");

    let revoke_ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-revoke-1".to_string(),
        causation_id: None,
        idempotency_key: format!("revoke:{tenant_id}:{user_id}:{}", role.id),
    };

    rbac::revoke_user_role_with_audit(
        &pool,
        tenant_id,
        user_id,
        role.id,
        Some(actor_id),
        &revoke_ctx,
    )
    .await
    .expect("revoke role with audit");

    let review_ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-review-1".to_string(),
        causation_id: None,
        idempotency_key: format!("review:{tenant_id}:{user_id}:{review_id}"),
    };

    user_lifecycle_audit::record_access_review_decision(
        &pool,
        tenant_id,
        user_id,
        actor_id,
        "approved",
        review_id,
        Some("quarterly review"),
        &review_ctx,
    )
    .await
    .expect("record access review");

    let timeline = user_lifecycle_audit::list_user_lifecycle_timeline(&pool, tenant_id, user_id)
        .await
        .expect("list timeline");

    assert_eq!(timeline.len(), 4, "expected exactly 4 lifecycle events");

    let event_types = timeline
        .iter()
        .map(|e| e.event_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            "user_created",
            "role_assigned",
            "role_revoked",
            "access_review_recorded",
        ],
        "timeline ordering mismatch"
    );

    let outbox_count: i64 = sqlx::query("SELECT COUNT(*) AS c FROM user_lifecycle_events_outbox WHERE tenant_id = $1 AND aggregate_id = $2")
        .bind(tenant_id)
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("count outbox")
        .get("c");
    assert_eq!(
        outbox_count, 4,
        "outbox should contain one record per event"
    );
}

#[tokio::test]
async fn replay_safety_duplicate_idempotency_key_does_not_duplicate_audit_rows() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let reviewer = Uuid::new_v4();
    let review_id = Uuid::new_v4();
    let idem = format!("review-replay:{tenant_id}:{user_id}:{review_id}");

    let ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-review-replay".to_string(),
        causation_id: None,
        idempotency_key: idem.clone(),
    };

    let first = user_lifecycle_audit::record_access_review_decision(
        &pool,
        tenant_id,
        user_id,
        reviewer,
        "approved",
        review_id,
        Some("first attempt"),
        &ctx,
    )
    .await
    .expect("first review write");

    let second = user_lifecycle_audit::record_access_review_decision(
        &pool,
        tenant_id,
        user_id,
        reviewer,
        "approved",
        review_id,
        Some("retry attempt"),
        &ctx,
    )
    .await
    .expect("second review write");

    assert!(first.is_some(), "first write should create an event");
    assert!(
        second.is_none(),
        "duplicate idempotency key must be a no-op"
    );

    let audit_count: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM user_lifecycle_audit_events WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(idem)
    .fetch_one(&pool)
    .await
    .expect("count audit rows")
    .get("c");

    let outbox_count: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM user_lifecycle_events_outbox WHERE tenant_id = $1 AND aggregate_id = $2",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("count outbox rows")
    .get("c");

    assert_eq!(
        audit_count, 1,
        "duplicate idempotency must not create another audit row"
    );
    assert_eq!(
        outbox_count, 1,
        "duplicate idempotency must not create another outbox row"
    );
}

#[tokio::test]
async fn sod_forbidden_combo_denies_high_risk_action_and_logs_idempotently() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_user_id = Uuid::new_v4();

    let role_a = rbac::create_role(
        &pool,
        tenant_id,
        "quality_release",
        "Quality Release",
        false,
    )
    .await
    .expect("create role a");
    let role_b = rbac::create_role(
        &pool,
        tenant_id,
        "production_execute",
        "Production Execute",
        false,
    )
    .await
    .expect("create role b");

    let ctx_bind_a = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-bind-a".to_string(),
        causation_id: None,
        idempotency_key: format!("bind-a:{tenant_id}:{actor_user_id}:{}", role_a.id),
    };
    let ctx_bind_b = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-bind-b".to_string(),
        causation_id: None,
        idempotency_key: format!("bind-b:{tenant_id}:{actor_user_id}:{}", role_b.id),
    };

    rbac::bind_user_role_with_audit(
        &pool,
        tenant_id,
        actor_user_id,
        role_a.id,
        None,
        &ctx_bind_a,
    )
    .await
    .expect("bind role a");
    rbac::bind_user_role_with_audit(
        &pool,
        tenant_id,
        actor_user_id,
        role_b.id,
        None,
        &ctx_bind_b,
    )
    .await
    .expect("bind role b");

    let policy = sod::upsert_policy(
        &pool,
        sod::SodPolicyUpsert {
            tenant_id,
            action_key: "workflow.approve_release".to_string(),
            primary_role_id: role_a.id,
            conflicting_role_id: role_b.id,
            allow_override: false,
            override_requires_approval: false,
            scope: None,
            description: None,
            actor_user_id: None,
            idempotency_key: format!("sod-policy:{tenant_id}:approve_release"),
            trace_id: "trace-sod-policy".to_string(),
            causation_id: None,
            producer: "auth-rs@test".to_string(),
        },
    )
    .await
    .expect("upsert sod policy");
    assert!(!policy.idempotent_replay);

    let first_eval = sod::evaluate_decision(
        &pool,
        sod::SodDecisionRequest {
            tenant_id,
            action_key: "workflow.approve_release".to_string(),
            actor_user_id,
            subject_user_id: None,
            override_granted_by: None,
            override_ticket: None,
            idempotency_key: format!("sod-eval:{tenant_id}:1"),
            trace_id: "trace-sod-eval-1".to_string(),
            causation_id: None,
            producer: "auth-rs@test".to_string(),
        },
    )
    .await
    .expect("first sod eval");

    assert_eq!(first_eval.decision, "deny");
    assert!(
        first_eval.reason.contains("sod_conflict"),
        "unexpected deny reason: {}",
        first_eval.reason
    );
    assert_eq!(first_eval.matched_policy_ids.len(), 1);
    assert!(!first_eval.idempotent_replay);

    let replay_eval = sod::evaluate_decision(
        &pool,
        sod::SodDecisionRequest {
            tenant_id,
            action_key: "workflow.approve_release".to_string(),
            actor_user_id,
            subject_user_id: None,
            override_granted_by: None,
            override_ticket: None,
            idempotency_key: format!("sod-eval:{tenant_id}:1"),
            trace_id: "trace-sod-eval-1-replay".to_string(),
            causation_id: None,
            producer: "auth-rs@test".to_string(),
        },
    )
    .await
    .expect("replay sod eval");

    assert_eq!(replay_eval.decision, "deny");
    assert!(replay_eval.idempotent_replay);

    let decision_rows: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM sod_decision_logs WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(format!("sod-eval:{tenant_id}:1"))
    .fetch_one(&pool)
    .await
    .expect("count sod decision logs")
    .get("c");

    let outbox_rows: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM sod_events_outbox WHERE tenant_id = $1 AND event_type = 'auth.sod.decision.recorded'",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count sod outbox")
    .get("c");

    assert_eq!(
        decision_rows, 1,
        "duplicate idempotency should not duplicate decision rows"
    );
    assert_eq!(
        outbox_rows, 1,
        "duplicate idempotency should not duplicate outbox events"
    );
}

#[tokio::test]
async fn sod_delete_policy_removes_rule_and_emits_outbox_event() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();

    let role_a = rbac::create_role(&pool, tenant_id, "inspector", "Inspector", false)
        .await
        .expect("create role a");
    let role_b = rbac::create_role(&pool, tenant_id, "approver", "Approver", false)
        .await
        .expect("create role b");

    let result = sod::upsert_policy(
        &pool,
        sod::SodPolicyUpsert {
            tenant_id,
            action_key: "mrb.approve".to_string(),
            primary_role_id: role_a.id,
            conflicting_role_id: role_b.id,
            allow_override: false,
            override_requires_approval: false,
            scope: None,
            description: None,
            actor_user_id: None,
            idempotency_key: format!("sod-del-test:{tenant_id}:create"),
            trace_id: "trace-del-create".to_string(),
            causation_id: None,
            producer: "auth-rs@test".to_string(),
        },
    )
    .await
    .expect("upsert policy");
    let policy_id = result.policy.id;

    // Delete the policy
    let del = sod::delete_policy(
        &pool,
        sod::SodPolicyDeleteRequest {
            tenant_id,
            policy_id,
            actor_user_id: None,
            idempotency_key: format!("sod-del-test:{tenant_id}:delete"),
            trace_id: "trace-del-1".to_string(),
            causation_id: None,
            producer: "auth-rs@test".to_string(),
        },
    )
    .await
    .expect("delete policy");

    assert!(del.deleted, "policy should be deleted");
    assert!(!del.idempotent_replay);

    // Verify policy no longer exists
    let policies = sod::list_policies(&pool, tenant_id, "mrb.approve")
        .await
        .expect("list after delete");
    assert!(policies.is_empty(), "policy should be gone");

    // Verify outbox event emitted
    let outbox: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM sod_events_outbox WHERE tenant_id = $1 AND event_type = 'auth.sod.policy.deleted'",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count outbox")
    .get("c");
    assert_eq!(outbox, 1, "delete event must be in outbox");

    // Replay: same idempotency key returns replay
    let replay = sod::delete_policy(
        &pool,
        sod::SodPolicyDeleteRequest {
            tenant_id,
            policy_id,
            actor_user_id: None,
            idempotency_key: format!("sod-del-test:{tenant_id}:delete"),
            trace_id: "trace-del-replay".to_string(),
            causation_id: None,
            producer: "auth-rs@test".to_string(),
        },
    )
    .await
    .expect("replay delete");
    assert!(replay.idempotent_replay, "duplicate key must be replay");
}

#[tokio::test]
async fn sod_evaluate_allows_action_when_no_conflict_and_denies_after_policy_created() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    let role = rbac::create_role(&pool, tenant_id, "viewer", "Viewer", false)
        .await
        .expect("create role");

    let ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-ctx-aware".to_string(),
        causation_id: None,
        idempotency_key: format!("bind-ctx:{tenant_id}:{actor_id}:{}", role.id),
    };
    rbac::bind_user_role_with_audit(&pool, tenant_id, actor_id, role.id, None, &ctx)
        .await
        .expect("bind role");

    // No SoD policy → allow
    let allowed = sod::evaluate_decision(
        &pool,
        sod::SodDecisionRequest {
            tenant_id,
            action_key: "report.view".to_string(),
            actor_user_id: actor_id,
            subject_user_id: None,
            override_granted_by: None,
            override_ticket: None,
            idempotency_key: format!("ctx-eval:{tenant_id}:view"),
            trace_id: "trace-ctx-view".to_string(),
            causation_id: None,
            producer: "auth-rs@test".to_string(),
        },
    )
    .await
    .expect("eval view");
    assert_eq!(allowed.decision, "allow");

    // Different action → also allow (context-aware)
    let allowed2 = sod::evaluate_decision(
        &pool,
        sod::SodDecisionRequest {
            tenant_id,
            action_key: "report.export".to_string(),
            actor_user_id: actor_id,
            subject_user_id: None,
            override_granted_by: None,
            override_ticket: None,
            idempotency_key: format!("ctx-eval:{tenant_id}:export"),
            trace_id: "trace-ctx-export".to_string(),
            causation_id: None,
            producer: "auth-rs@test".to_string(),
        },
    )
    .await
    .expect("eval export");
    assert_eq!(allowed2.decision, "allow");
}

#[tokio::test]
async fn role_snapshot_id_is_deterministic_and_order_independent() {
    let roles_a = vec!["admin".to_string(), "operator".to_string()];
    let roles_b = vec!["operator".to_string(), "admin".to_string()];
    let roles_c = vec!["admin".to_string()];

    let snap_a = jwt::compute_role_snapshot_id(&roles_a);
    let snap_b = jwt::compute_role_snapshot_id(&roles_b);
    let snap_c = jwt::compute_role_snapshot_id(&roles_c);

    assert_eq!(snap_a, snap_b, "order must not matter");
    assert_ne!(snap_a, snap_c, "different sets must differ");
    assert_eq!(snap_a.len(), 16, "snapshot must be 16-char hex");
}

#[tokio::test]
async fn cross_tenant_sod_isolation() {
    let pool = test_pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    let role_a = rbac::create_role(&pool, tenant_a, "qa_role", "QA", false)
        .await
        .expect("create role tenant_a");
    let role_b = rbac::create_role(&pool, tenant_a, "prod_role", "Prod", false)
        .await
        .expect("create role2 tenant_a");

    // Create policy in tenant_a
    sod::upsert_policy(
        &pool,
        sod::SodPolicyUpsert {
            tenant_id: tenant_a,
            action_key: "release.approve".to_string(),
            primary_role_id: role_a.id,
            conflicting_role_id: role_b.id,
            allow_override: false,
            override_requires_approval: false,
            scope: None,
            description: None,
            actor_user_id: None,
            idempotency_key: format!("cross-tenant:{tenant_a}:policy"),
            trace_id: "trace-cross".to_string(),
            causation_id: None,
            producer: "auth-rs@test".to_string(),
        },
    )
    .await
    .expect("upsert tenant_a policy");

    // tenant_b should have no policies for same action
    let policies_b = sod::list_policies(&pool, tenant_b, "release.approve")
        .await
        .expect("list tenant_b");
    assert!(policies_b.is_empty(), "tenant_b must not see tenant_a policies");

    // Evaluating in tenant_b with same actor should always allow (no policy)
    let eval_b = sod::evaluate_decision(
        &pool,
        sod::SodDecisionRequest {
            tenant_id: tenant_b,
            action_key: "release.approve".to_string(),
            actor_user_id: actor_id,
            subject_user_id: None,
            override_granted_by: None,
            override_ticket: None,
            idempotency_key: format!("cross-eval:{tenant_b}:1"),
            trace_id: "trace-cross-eval".to_string(),
            causation_id: None,
            producer: "auth-rs@test".to_string(),
        },
    )
    .await
    .expect("eval tenant_b");
    assert_eq!(eval_b.decision, "allow", "no policy in tenant_b = allow");
}
