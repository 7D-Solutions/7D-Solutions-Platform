//! Backend smoke runbook for the sync stack (bd-r2l8z).
//!
//! Executes all critical sync flows in a single sequential session against real
//! infrastructure — no mocks, no stubs.
//!
//!   Phase 1 — Authority flip (platform → external, then back)
//!   Phase 2 — Push (customer create via QBO sandbox; skipped if QBO_SANDBOX≠1)
//!   Phase 3 — Detector (true-drift scenario → ConflictOpened)
//!   Phase 4 — Conflicts list (paginated read, tenant isolation)
//!   Phase 5 — Bulk resolve (resolve + ignore, deterministic key deduplication)
//!   Phase 6 — DLQ (seed needs_reauth row, list_failed filter)
//!   Phase 7 — Jobs health (upsert success + failure streak, list_jobs)
//!
//! Prerequisites
//! -------------
//! DATABASE_URL  — real Postgres with integrations schema (auto-migrated)
//! NATS_URL      — real NATS server (optional; InMemoryBus used if absent)
//! QBO_SANDBOX=1 — enables Phase 2; also requires:
//!   .env.qbo-sandbox  at the repo root (QBO_CLIENT_ID / QBO_CLIENT_SECRET)
//!   .qbo-tokens.json  at the repo root  (access_token / refresh_token / realm_id)
//!
//! Run full suite (sandbox + NATS):
//!   DATABASE_URL=postgres://... NATS_URL=nats://... QBO_SANDBOX=1 \
//!     ./scripts/cargo-slot.sh test -p integrations-rs --test sync_smoke_runbook -- --nocapture
//!
//! Run DB-only (no sandbox, no NATS):
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test sync_smoke_runbook -- --nocapture

use chrono::Utc;
use event_bus::{InMemoryBus, NatsBus};
use integrations_rs::{
    domain::{
        qbo::{client::QboClient, QboError, TokenProvider},
        sync::{
            authority_service::flip_authority,
            conflicts::CreateConflictRequest,
            conflicts_repo::{create_conflict, list_conflicts_paged},
            dedupe::{compute_comparable_hash, compute_fingerprint},
            detector::{run_detector, DetectorOutcome},
            health::{list_jobs, upsert_job_failure, upsert_job_success},
            resolve_service::{bulk_resolve_conflicts, BulkResolveItem, BulkResolveOutcome},
        },
    },
    outbox::list_failed,
};
use serde_json::Value;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── DB pool ───────────────────────────────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("connect to integrations test DB")
}

async fn run_migrations(pool: &sqlx::PgPool) {
    sqlx::migrate!("db/migrations")
        .run(pool)
        .await
        .expect("run integrations migrations");
}

// ── Event bus ─────────────────────────────────────────────────────────────────

async fn make_bus() -> Arc<dyn event_bus::EventBus> {
    if let Ok(url) = std::env::var("NATS_URL") {
        match event_bus::connect_nats(&url).await {
            Ok(client) => {
                eprintln!("  [BUS]  Connected to NATS at {}", url);
                return Arc::new(NatsBus::new(client));
            }
            Err(e) => {
                eprintln!("  [WARN] NATS_URL set but connection failed: {} — using InMemoryBus", e);
            }
        }
    } else {
        eprintln!("  [BUS]  NATS_URL not set — using InMemoryBus (set NATS_URL for full smoke)");
    }
    Arc::new(InMemoryBus::new())
}

// ── Test-tenant helpers ───────────────────────────────────────────────────────

fn tenant() -> String {
    format!("smoke-{}", Uuid::new_v4().simple())
}

async fn seed_oauth_connection(pool: &sqlx::PgPool, app_id: &str, realm_id: &str) {
    sqlx::query(
        r#"
        INSERT INTO integrations_oauth_connections (
            app_id, provider, realm_id,
            access_token, refresh_token,
            access_token_expires_at, refresh_token_expires_at,
            scopes_granted, connection_status
        )
        VALUES ($1, 'quickbooks', $2,
                '\x74657374'::bytea, '\x74657374'::bytea,
                NOW() + INTERVAL '1 hour', NOW() + INTERVAL '30 days',
                'com.intuit.quickbooks.accounting', 'connected')
        ON CONFLICT (app_id, provider) DO UPDATE
            SET realm_id = EXCLUDED.realm_id,
                connection_status = EXCLUDED.connection_status
        "#,
    )
    .bind(app_id)
    .bind(realm_id)
    .execute(pool)
    .await
    .expect("seed_oauth_connection");
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    for q in &[
        "DELETE FROM integrations_sync_jobs WHERE app_id = $1",
        "DELETE FROM integrations_outbox WHERE app_id = $1",
        "DELETE FROM integrations_sync_conflicts WHERE app_id = $1",
        "DELETE FROM integrations_sync_push_attempts WHERE app_id = $1",
        "DELETE FROM integrations_sync_authority WHERE app_id = $1",
        "DELETE FROM integrations_oauth_connections WHERE app_id = $1",
    ] {
        sqlx::query(q).bind(app_id).execute(pool).await.ok();
    }
}

// ── QBO sandbox token provider (Phase 2 only) ────────────────────────────────

struct SandboxTokenProvider {
    access_token: RwLock<String>,
    refresh_tok: RwLock<String>,
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
    tokens_path: PathBuf,
}

impl SandboxTokenProvider {
    fn load() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        dotenvy::from_path(root.join(".env.qbo-sandbox")).expect(".env.qbo-sandbox missing");

        let client_id = std::env::var("QBO_CLIENT_ID").expect("QBO_CLIENT_ID");
        let client_secret = std::env::var("QBO_CLIENT_SECRET").expect("QBO_CLIENT_SECRET");

        let tokens_path = root.join(".qbo-tokens.json");
        let content = std::fs::read_to_string(&tokens_path).expect(".qbo-tokens.json missing");
        let tokens: Value = serde_json::from_str(&content).expect("parse .qbo-tokens.json");

        Self {
            access_token: RwLock::new(
                tokens["access_token"].as_str().expect("access_token").into(),
            ),
            refresh_tok: RwLock::new(
                tokens["refresh_token"].as_str().expect("refresh_token").into(),
            ),
            client_id,
            client_secret,
            http: reqwest::Client::new(),
            tokens_path,
        }
    }

    fn realm_id(&self) -> String {
        let content = std::fs::read_to_string(&self.tokens_path).expect(".qbo-tokens.json");
        let t: Value = serde_json::from_str(&content).expect("parse");
        t["realm_id"].as_str().expect("realm_id").to_string()
    }
}

#[async_trait::async_trait]
impl TokenProvider for SandboxTokenProvider {
    async fn get_token(&self) -> Result<String, QboError> {
        Ok(self.access_token.read().await.clone())
    }

    async fn refresh_token(&self) -> Result<String, QboError> {
        let rt = self.refresh_tok.read().await.clone();
        let resp = self
            .http
            .post("https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer")
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[("grant_type", "refresh_token"), ("refresh_token", &rt)])
            .send()
            .await
            .map_err(|e| QboError::TokenError(e.to_string()))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(QboError::TokenError(format!("refresh failed: {body}")));
        }

        let tr: Value = resp.json().await.map_err(|e| QboError::TokenError(e.to_string()))?;
        let new_at = tr["access_token"]
            .as_str()
            .ok_or_else(|| QboError::TokenError("no access_token in refresh".into()))?
            .to_string();
        let new_rt = tr["refresh_token"]
            .as_str()
            .ok_or_else(|| QboError::TokenError("no refresh_token in refresh".into()))?
            .to_string();

        *self.access_token.write().await = new_at.clone();
        *self.refresh_tok.write().await = new_rt.clone();

        if let Ok(raw) = std::fs::read_to_string(&self.tokens_path) {
            if let Ok(mut v) = serde_json::from_str::<Value>(&raw) {
                v["access_token"] = Value::String(new_at.clone());
                v["refresh_token"] = Value::String(new_rt);
                let _ = std::fs::write(
                    &self.tokens_path,
                    serde_json::to_string_pretty(&v).expect("json"),
                );
            }
        }
        Ok(new_at)
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Smoke runbook
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn sync_smoke_runbook() {
    let pool = setup_db().await;
    run_migrations(&pool).await;
    let _bus = make_bus().await;

    let app_id = tenant();
    let realm_id = format!("realm-smoke-{}", Uuid::new_v4().simple());
    seed_oauth_connection(&pool, &app_id, &realm_id).await;

    let mut failures: Vec<String> = Vec::new();
    let sandbox_enabled = std::env::var("QBO_SANDBOX").unwrap_or_default() == "1";

    eprintln!("\n╔══════════════════════════════════════════════════════════╗");
    eprintln!("║  SYNC STACK BACKEND SMOKE RUNBOOK                        ║");
    eprintln!("╠══════════════════════════════════════════════════════════╣");
    eprintln!("║  tenant:  {}  ║", &app_id);
    eprintln!("║  QBO sandbox: {}   NATS: {}               ║",
        if sandbox_enabled { "ON " } else { "OFF" },
        if std::env::var("NATS_URL").is_ok() { "ON " } else { "OFF" });
    eprintln!("╚══════════════════════════════════════════════════════════╝\n");

    // ── Phase 1: Authority flip ───────────────────────────────────────────────

    eprintln!("▸ Phase 1 — Authority flip");

    let flip1 = flip_authority(
        &pool,
        &app_id,
        "quickbooks",
        "customer",
        "external",
        "smoke-runbook",
        Uuid::new_v4().to_string(),
    )
    .await;

    match &flip1 {
        Ok(r) => {
            assert_eq!(r.row.authoritative_side, "external", "expected external after flip");
            assert!(r.row.authority_version >= 1, "version must be >= 1");
            eprintln!(
                "  PASS  flip platform→external: version={}",
                r.row.authority_version
            );
        }
        Err(e) => {
            failures.push(format!("Phase 1 flip platform→external: {e}"));
            eprintln!("  FAIL  flip platform→external: {e}");
        }
    }

    // Flip back to platform — bidirectional capability.
    let flip2 = flip_authority(
        &pool,
        &app_id,
        "quickbooks",
        "customer",
        "platform",
        "smoke-runbook",
        Uuid::new_v4().to_string(),
    )
    .await;

    let current_authority_version = match &flip2 {
        Ok(r) => {
            assert_eq!(r.row.authoritative_side, "platform");
            eprintln!(
                "  PASS  flip external→platform: version={}",
                r.row.authority_version
            );
            r.row.authority_version
        }
        Err(e) => {
            failures.push(format!("Phase 1 flip external→platform: {e}"));
            eprintln!("  FAIL  flip external→platform: {e}");
            1
        }
    };

    // ── Phase 2: Push (QBO sandbox only) ─────────────────────────────────────

    eprintln!("\n▸ Phase 2 — Push (QBO sandbox)");

    if !sandbox_enabled {
        eprintln!("  SKIP  QBO_SANDBOX≠1 — set QBO_SANDBOX=1 for live push");
    } else {
        use integrations_rs::domain::{
            qbo::client::QboCustomerPayload,
            sync::resolve_service::{PushOutcome, ResolveService},
        };

        let provider = Arc::new(SandboxTokenProvider::load());
        let realm = provider.realm_id();
        let base_url = std::env::var("QBO_SANDBOX_BASE")
            .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com".into());

        let qbo = Arc::new(QboClient::new(&base_url, &realm, provider));
        let svc = ResolveService::new(qbo);

        let unique_name = format!("SmokeRunbook-{}", Uuid::new_v4().simple());
        let payload = serde_json::to_value(QboCustomerPayload {
            display_name: unique_name.clone(),
            email: Some("smoke@runbook.example.com".into()),
            company_name: Some("Smoke Runbook Corp".into()),
            currency_ref: None,
        })
        .expect("serialize payload");

        let entity_id = format!("smoke-cust-{}", Uuid::new_v4().simple());
        let fp = format!("smoke-fp-{}", Uuid::new_v4().simple());

        match svc
            .push_customer(&pool, &app_id, &entity_id, "create", current_authority_version, &fp, &payload)
            .await
        {
            Ok(PushOutcome::Succeeded { provider_entity_id, .. }) => {
                eprintln!(
                    "  PASS  push customer create: QBO entity_id={:?}",
                    provider_entity_id
                );
            }
            Ok(other) => {
                failures.push(format!("Phase 2 push unexpected outcome: {:?}", other));
                eprintln!("  FAIL  unexpected push outcome: {:?}", other);
            }
            Err(e) => {
                failures.push(format!("Phase 2 push error: {e}"));
                eprintln!("  FAIL  push error: {e}");
            }
        }
    }

    // ── Phase 3: Detector (true drift → ConflictOpened) ──────────────────────

    eprintln!("\n▸ Phase 3 — Detector (true drift)");

    let drift_entity_id = format!("drift-ent-{}", Uuid::new_v4().simple());
    let drift_ts = Utc::now();
    let drift_fingerprint = compute_fingerprint(
        Some("st:smoke-drift-sync-token"),
        Some(drift_ts),
        &serde_json::json!({}),
    );
    let drift_comparable_hash = compute_comparable_hash(
        &serde_json::json!({"amount": 999, "name": "Drifted Customer"}),
        drift_ts,
    );

    let detector_outcome = run_detector(
        &pool,
        &app_id,
        "quickbooks",
        "customer",
        &drift_entity_id,
        &drift_fingerprint,
        &drift_comparable_hash,
        Some(serde_json::json!({"amount": 100, "name": "Platform Customer"})),
        Some(serde_json::json!({"amount": 999, "name": "Drifted Customer"})),
    )
    .await;

    let conflict_id = match detector_outcome {
        Ok(DetectorOutcome::ConflictOpened(row)) => {
            eprintln!(
                "  PASS  detector opened conflict: id={}, class={}",
                row.id, row.conflict_class
            );
            Some(row.id)
        }
        Ok(DetectorOutcome::SelfEchoSuppressed { attempt_id }) => {
            failures.push(format!("Phase 3 detector: unexpected SelfEcho on attempt {attempt_id}"));
            eprintln!("  FAIL  unexpected SelfEchoSuppressed");
            None
        }
        Ok(DetectorOutcome::OrphanedWriteRecovered { attempt_id }) => {
            failures.push(format!(
                "Phase 3 detector: unexpected OrphanedWriteRecovered on attempt {attempt_id}"
            ));
            eprintln!("  FAIL  unexpected OrphanedWriteRecovered");
            None
        }
        Err(e) => {
            failures.push(format!("Phase 3 detector error: {e}"));
            eprintln!("  FAIL  detector error: {e}");
            None
        }
    };

    // ── Phase 4: Conflicts list ───────────────────────────────────────────────

    eprintln!("\n▸ Phase 4 — Conflicts list");

    match list_conflicts_paged(
        &pool,
        &app_id,
        Some("quickbooks"),
        Some("customer"),
        Some("pending"),
        10,
        0,
    )
    .await
    {
        Ok((rows, total)) => {
            let found_phase3 = conflict_id.map_or(false, |cid| rows.iter().any(|r| r.id == cid));
            if conflict_id.is_some() && !found_phase3 {
                failures.push("Phase 4: conflict from Phase 3 not found in listing".into());
                eprintln!("  FAIL  Phase 3 conflict missing from list");
            } else {
                eprintln!("  PASS  list conflicts: {} rows, total={}", rows.len(), total);
            }

            // Isolation: another tenant must see 0 rows.
            let other_app = format!("other-{}", Uuid::new_v4().simple());
            match list_conflicts_paged(&pool, &other_app, None, None, None, 10, 0).await {
                Ok((other_rows, _)) => {
                    if !other_rows.is_empty() {
                        failures.push(format!(
                            "Phase 4 isolation: other tenant saw {} rows (expected 0)",
                            other_rows.len()
                        ));
                        eprintln!("  FAIL  tenant isolation violated");
                    } else {
                        eprintln!("  PASS  tenant isolation: other tenant sees 0 rows");
                    }
                }
                Err(e) => {
                    failures.push(format!("Phase 4 isolation query error: {e}"));
                    eprintln!("  FAIL  isolation query error: {e}");
                }
            }
        }
        Err(e) => {
            failures.push(format!("Phase 4 list_conflicts_paged: {e}"));
            eprintln!("  FAIL  list_conflicts_paged: {e}");
        }
    }

    // ── Phase 5: Bulk resolve ─────────────────────────────────────────────────

    eprintln!("\n▸ Phase 5 — Bulk resolve");

    // Seed two conflicts: one to resolve, one to ignore.
    let resolve_conflict_row = create_conflict(
        &pool,
        &CreateConflictRequest {
            app_id: app_id.clone(),
            provider: "quickbooks".into(),
            entity_type: "invoice".into(),
            entity_id: format!("inv-resolve-{}", Uuid::new_v4().simple()),
            conflict_class: integrations_rs::domain::sync::conflicts::ConflictClass::Edit,
            detected_by: "smoke-runbook".into(),
            internal_value: Some(serde_json::json!({"total": 100})),
            external_value: Some(serde_json::json!({"total": 200})),
        },
    )
    .await
    .expect("create resolve-target conflict");

    let ignore_conflict_row = create_conflict(
        &pool,
        &CreateConflictRequest {
            app_id: app_id.clone(),
            provider: "quickbooks".into(),
            entity_type: "invoice".into(),
            entity_id: format!("inv-ignore-{}", Uuid::new_v4().simple()),
            conflict_class: integrations_rs::domain::sync::conflicts::ConflictClass::Edit,
            detected_by: "smoke-runbook".into(),
            internal_value: Some(serde_json::json!({"total": 300})),
            external_value: Some(serde_json::json!({"total": 400})),
        },
    )
    .await
    .expect("create ignore-target conflict");

    let make_items = || {
        vec![
            BulkResolveItem {
                conflict_id: resolve_conflict_row.id,
                action: "resolve".into(),
                authority_version: 1,
                internal_id: Some("int-smoke-001".into()),
                resolution_note: Some("smoke runbook resolve".into()),
                caller_idempotency_key: None,
            },
            BulkResolveItem {
                conflict_id: ignore_conflict_row.id,
                action: "ignore".into(),
                authority_version: 1,
                internal_id: None,
                resolution_note: None,
                caller_idempotency_key: None,
            },
        ]
    };

    match bulk_resolve_conflicts(&pool, &app_id, "smoke-runbook", make_items()).await {
        Ok(outcomes) => {
            let items_check = make_items();
            let mut pass = true;
            for (item, outcome) in items_check.iter().zip(outcomes.iter()) {
                match outcome {
                    BulkResolveOutcome::Resolved { .. } if item.action == "resolve" => {
                        eprintln!("  PASS  conflict {} → Resolved", item.conflict_id);
                    }
                    BulkResolveOutcome::Ignored { .. } if item.action == "ignore" => {
                        eprintln!("  PASS  conflict {} → Ignored", item.conflict_id);
                    }
                    other => {
                        failures.push(format!(
                            "Phase 5 unexpected outcome for {}: {:?}",
                            item.conflict_id, other
                        ));
                        eprintln!("  FAIL  conflict {} unexpected: {:?}", item.conflict_id, other);
                        pass = false;
                    }
                }
            }

            // Idempotency: re-submitting the same items returns AlreadyResolved/AlreadyIgnored.
            if pass {
                match bulk_resolve_conflicts(&pool, &app_id, "smoke-runbook", make_items()).await {
                    Ok(retry_outcomes) => {
                        let all_terminal = retry_outcomes.iter().all(|o| {
                            matches!(
                                o,
                                BulkResolveOutcome::AlreadyResolved { .. }
                                    | BulkResolveOutcome::AlreadyIgnored { .. }
                                    | BulkResolveOutcome::AlreadyUnresolvable { .. }
                            )
                        });
                        if all_terminal {
                            eprintln!("  PASS  bulk resolve idempotency: retry returns terminal outcomes");
                        } else {
                            failures.push("Phase 5 idempotency: retry did not return terminal outcomes".into());
                            eprintln!("  FAIL  bulk resolve idempotency violated");
                        }
                    }
                    Err(e) => {
                        failures.push(format!("Phase 5 idempotency retry error: {e:?}"));
                        eprintln!("  FAIL  idempotency retry error: {e:?}");
                    }
                }
            }
        }
        Err(e) => {
            failures.push(format!("Phase 5 bulk_resolve_conflicts: {e:?}"));
            eprintln!("  FAIL  bulk_resolve_conflicts: {e:?}");
        }
    }

    // ── Phase 6: DLQ ─────────────────────────────────────────────────────────

    eprintln!("\n▸ Phase 6 — DLQ");

    let dlq_event_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO integrations_outbox
            (event_id, event_type, aggregate_type, aggregate_id, app_id, payload,
             failed_at, error_message, failure_reason, schema_version)
        VALUES ($1, 'integrations.sync.push.failed', 'customer', 'cust-dlq-1', $2,
                '{}', NOW(), 'token expired', 'needs_reauth', '1.0.0')
        "#,
    )
    .bind(dlq_event_id)
    .bind(&app_id)
    .execute(&pool)
    .await
    .expect("seed DLQ row");

    match list_failed(&pool, &app_id, Some("needs_reauth"), 1, 10).await {
        Ok((rows, total)) => {
            let found = rows.iter().any(|r| r.event_id == dlq_event_id);
            if found {
                eprintln!("  PASS  DLQ needs_reauth filter: {} row(s), total={}", rows.len(), total);
            } else {
                failures.push("Phase 6: seeded DLQ row not returned by list_failed".into());
                eprintln!("  FAIL  seeded DLQ row not found");
            }

            // Filter by a different reason should return 0 for this tenant.
            match list_failed(&pool, &app_id, Some("authority_superseded"), 1, 10).await {
                Ok((other_rows, _)) => {
                    if other_rows.is_empty() {
                        eprintln!("  PASS  DLQ filter isolation: authority_superseded returns 0");
                    } else {
                        failures.push(format!(
                            "Phase 6 DLQ filter: expected 0 authority_superseded, got {}",
                            other_rows.len()
                        ));
                        eprintln!("  FAIL  unexpected DLQ rows for authority_superseded");
                    }
                }
                Err(e) => {
                    failures.push(format!("Phase 6 DLQ filter query: {e}"));
                    eprintln!("  FAIL  DLQ filter query: {e}");
                }
            }
        }
        Err(e) => {
            failures.push(format!("Phase 6 list_failed: {e}"));
            eprintln!("  FAIL  list_failed: {e}");
        }
    }

    // ── Phase 7: Jobs health ──────────────────────────────────────────────────

    eprintln!("\n▸ Phase 7 — Jobs health");

    // Upsert a successful run.
    match upsert_job_success(&pool, &app_id, "quickbooks", "cdc_poll").await {
        Ok(row) => {
            assert_eq!(row.failure_streak, 0, "success must reset streak");
            eprintln!(
                "  PASS  upsert_job_success: failure_streak={}, last_success_at={}",
                row.failure_streak,
                row.last_success_at.map_or("none".into(), |t| t.to_rfc3339())
            );
        }
        Err(e) => {
            failures.push(format!("Phase 7 upsert_job_success: {e}"));
            eprintln!("  FAIL  upsert_job_success: {e}");
        }
    }

    // Two consecutive failures should produce streak=2.
    for i in 1u32..=2 {
        match upsert_job_failure(&pool, &app_id, "quickbooks", "cdc_poll", "smoke_error").await {
            Ok(row) => {
                if row.failure_streak == i as i32 {
                    eprintln!("  PASS  failure #{i}: streak={}", row.failure_streak);
                } else {
                    failures.push(format!(
                        "Phase 7 failure #{i}: expected streak={i}, got {}",
                        row.failure_streak
                    ));
                    eprintln!("  FAIL  failure #{i}: wrong streak={}", row.failure_streak);
                }
            }
            Err(e) => {
                failures.push(format!("Phase 7 upsert_job_failure #{i}: {e}"));
                eprintln!("  FAIL  upsert_job_failure #{i}: {e}");
            }
        }
    }

    // list_jobs must return the job row and respect tenant isolation.
    match list_jobs(&pool, &app_id, 1, 10).await {
        Ok((rows, total)) => {
            let found = rows.iter().any(|r| r.job_name == "cdc_poll");
            if found {
                let job = rows.iter().find(|r| r.job_name == "cdc_poll").unwrap();
                eprintln!(
                    "  PASS  list_jobs: {} row(s), cdc_poll streak={}, total={}",
                    rows.len(),
                    job.failure_streak,
                    total
                );
            } else {
                failures.push("Phase 7: cdc_poll job not returned by list_jobs".into());
                eprintln!("  FAIL  cdc_poll not found in list_jobs");
            }

            // Tenant isolation.
            let other = format!("other-jobs-{}", Uuid::new_v4().simple());
            match list_jobs(&pool, &other, 1, 10).await {
                Ok((other_rows, _)) => {
                    if other_rows.is_empty() {
                        eprintln!("  PASS  list_jobs tenant isolation: other tenant sees 0 rows");
                    } else {
                        failures.push(format!(
                            "Phase 7 isolation: other tenant saw {} job rows",
                            other_rows.len()
                        ));
                        eprintln!("  FAIL  list_jobs tenant isolation violated");
                    }
                }
                Err(e) => {
                    failures.push(format!("Phase 7 isolation query: {e}"));
                    eprintln!("  FAIL  list_jobs isolation query: {e}");
                }
            }
        }
        Err(e) => {
            failures.push(format!("Phase 7 list_jobs: {e}"));
            eprintln!("  FAIL  list_jobs: {e}");
        }
    }

    // ── Cleanup ───────────────────────────────────────────────────────────────

    cleanup(&pool, &app_id).await;

    // ── Summary ───────────────────────────────────────────────────────────────

    eprintln!("\n╔══════════════════════════════════════════════════════════╗");
    eprintln!("║  SMOKE RUNBOOK SUMMARY                                    ║");
    eprintln!("╚══════════════════════════════════════════════════════════╝");

    if failures.is_empty() {
        eprintln!("  ALL PHASES PASSED\n");
    } else {
        eprintln!("  {} FAILURE(S):", failures.len());
        for f in &failures {
            eprintln!("    ✗ {f}");
        }
        eprintln!();
        panic!("{} smoke runbook failure(s)", failures.len());
    }
}
