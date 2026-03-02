//! Accrual template & instance engine (Phase 24b, bd-3qa)
//!
//! Templates define recurring accrual patterns (accounts, amount, reversal policy).
//! Instances are created from templates per accounting period — each instance
//! posts a balanced journal entry and emits `gl.accrual_created` atomically.
//!
//! ## Guarantees
//! - **Atomic**: instance row + journal entry + outbox event in one transaction
//! - **Idempotent**: deterministic idempotency_key = "accrual:{template_id}:{period}"
//! - **Append-only**: instances are never modified after creation
//! - **Deterministic**: accrual_id = Uuid::v5(template_id, period)

use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::events::contracts::{
    AccrualCreatedPayload, AccrualReversedPayload, CashFlowClass, ReversalPolicy,
    EVENT_TYPE_ACCRUAL_CREATED, EVENT_TYPE_ACCRUAL_REVERSED, MUTATION_CLASS_DATA_MUTATION,
    MUTATION_CLASS_REVERSAL,
};
use crate::repos::outbox_repo;

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTemplateRequest {
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub debit_account: String,
    pub credit_account: String,
    pub amount_minor: i64,
    pub currency: String,
    pub reversal_policy: Option<ReversalPolicy>,
    pub cashflow_class: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TemplateResult {
    pub template_id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAccrualRequest {
    pub template_id: Uuid,
    pub tenant_id: String,
    pub period: String,
    pub posting_date: String,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccrualResult {
    pub instance_id: Uuid,
    pub accrual_id: Uuid,
    pub template_id: Uuid,
    pub period: String,
    pub journal_entry_id: Uuid,
    pub amount_minor: i64,
    pub currency: String,
    pub status: String,
    pub idempotent_hit: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecuteReversalsRequest {
    pub tenant_id: String,
    pub target_period: String,
    pub reversal_date: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReversalResult {
    pub reversal_id: Uuid,
    pub original_accrual_id: Uuid,
    pub journal_entry_id: Uuid,
    pub amount_minor: i64,
    pub currency: String,
    pub idempotent_hit: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecuteReversalsResult {
    pub target_period: String,
    pub reversals_executed: usize,
    pub reversals_skipped: usize,
    pub results: Vec<ReversalResult>,
}

// ============================================================================
// Template operations
// ============================================================================

/// Create an accrual template.
pub async fn create_template(
    db: &PgPool,
    req: &CreateTemplateRequest,
) -> Result<TemplateResult, AccrualError> {
    if req.amount_minor <= 0 {
        return Err(AccrualError::Validation(
            "amount_minor must be positive".to_string(),
        ));
    }
    if req.debit_account == req.credit_account {
        return Err(AccrualError::Validation(
            "debit_account and credit_account must differ".to_string(),
        ));
    }

    let template_id = Uuid::new_v4();
    let reversal_policy = req.reversal_policy.clone().unwrap_or(ReversalPolicy {
        auto_reverse_next_period: true,
        reverse_on_date: None,
    });
    let cashflow_class = req.cashflow_class.as_deref().unwrap_or("operating");
    let reversal_json =
        serde_json::to_value(&reversal_policy).map_err(AccrualError::Serialization)?;

    sqlx::query(
        r#"
        INSERT INTO gl_accrual_templates (
            template_id, tenant_id, name, description,
            debit_account, credit_account, amount_minor, currency,
            reversal_policy, cashflow_class
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(template_id)
    .bind(&req.tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(&req.debit_account)
    .bind(&req.credit_account)
    .bind(req.amount_minor)
    .bind(&req.currency)
    .bind(&reversal_json)
    .bind(cashflow_class)
    .execute(db)
    .await
    .map_err(AccrualError::Database)?;

    Ok(TemplateResult {
        template_id,
        tenant_id: req.tenant_id.clone(),
        name: req.name.clone(),
        active: true,
    })
}

// ============================================================================
// Instance creation (the core accrual posting)
// ============================================================================

/// Create an accrual instance from a template for a specific period.
///
/// Atomically:
/// 1. Inserts the instance row
/// 2. Posts a balanced journal entry (debit + credit)
/// 3. Emits gl.accrual_created to the outbox
///
/// Idempotent: deterministic key = "accrual:{template_id}:{period}"
pub async fn create_accrual_instance(
    db: &PgPool,
    req: &CreateAccrualRequest,
) -> Result<AccrualResult, AccrualError> {
    let idem_key = format!("accrual:{}:{}", req.template_id, req.period);

    // Idempotency guard: check for existing instance
    let existing = sqlx::query(
        r#"
        SELECT instance_id, accrual_id, journal_entry_id, amount_minor, currency, status
        FROM gl_accrual_instances
        WHERE idempotency_key = $1
        "#,
    )
    .bind(&idem_key)
    .fetch_optional(db)
    .await
    .map_err(AccrualError::Database)?;

    if let Some(row) = existing {
        return Ok(AccrualResult {
            instance_id: row.get("instance_id"),
            accrual_id: row.get("accrual_id"),
            template_id: req.template_id,
            period: req.period.clone(),
            journal_entry_id: row.get("journal_entry_id"),
            amount_minor: row.get("amount_minor"),
            currency: row.get("currency"),
            status: row.get("status"),
            idempotent_hit: true,
        });
    }

    // Fetch template
    let template = sqlx::query(
        r#"
        SELECT template_id, tenant_id, name, debit_account, credit_account,
               amount_minor, currency, reversal_policy, cashflow_class, active
        FROM gl_accrual_templates
        WHERE template_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(req.template_id)
    .bind(&req.tenant_id)
    .fetch_optional(db)
    .await
    .map_err(AccrualError::Database)?
    .ok_or_else(|| AccrualError::Validation("Template not found".to_string()))?;

    let active: bool = template.get("active");
    if !active {
        return Err(AccrualError::Validation("Template is inactive".to_string()));
    }

    let debit_account: String = template.get("debit_account");
    let credit_account: String = template.get("credit_account");
    let amount_minor: i64 = template.get("amount_minor");
    let currency: String = template.get("currency");
    let reversal_json: serde_json::Value = template.get("reversal_policy");
    let cashflow_str: String = template.get("cashflow_class");
    let template_name: String = template.get("name");

    let reversal_policy: ReversalPolicy =
        serde_json::from_value(reversal_json.clone()).map_err(AccrualError::Serialization)?;
    let cashflow_class = parse_cashflow_class(&cashflow_str);

    // Deterministic IDs
    let accrual_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("accrual:{}:{}", req.template_id, req.period).as_bytes(),
    );
    let instance_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("instance:{}:{}", req.template_id, req.period).as_bytes(),
    );
    let event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("accrual_event:{}:{}", req.template_id, req.period).as_bytes(),
    );

    let posting_date = NaiveDate::parse_from_str(&req.posting_date, "%Y-%m-%d")
        .map_err(|e| AccrualError::Validation(format!("Invalid posting_date: {}", e)))?;

    let instance_name = format!("{} — {}", template_name, req.period);
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| accrual_id.to_string());

    // Begin atomic transaction
    let mut tx = db.begin().await.map_err(AccrualError::Database)?;

    // 1. Post balanced journal entry (debit + credit)
    let journal_entry_id = Uuid::new_v4();
    let posted_at = posting_date.and_hms_opt(0, 0, 0).unwrap().and_utc();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (
            id, tenant_id, source_module, source_event_id, source_subject,
            posted_at, currency, description, reference_type, reference_id, correlation_id
        )
        VALUES ($1, $2, 'gl', $3, 'accrual', $4, $5, $6, 'GL_ACCRUAL', $7, $8)
        "#,
    )
    .bind(journal_entry_id)
    .bind(&req.tenant_id)
    .bind(event_id)
    .bind(posted_at)
    .bind(&currency)
    .bind(&instance_name)
    .bind(accrual_id.to_string())
    .bind(Uuid::parse_str(&correlation_id).ok())
    .execute(&mut *tx)
    .await
    .map_err(AccrualError::Database)?;

    // Debit line
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES ($1, $2, 1, $3, $4, 0, $5)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(journal_entry_id)
    .bind(&debit_account)
    .bind(amount_minor)
    .bind(format!("Accrual DR: {}", instance_name))
    .execute(&mut *tx)
    .await
    .map_err(AccrualError::Database)?;

    // Credit line
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES ($1, $2, 2, $3, 0, $4, $5)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(journal_entry_id)
    .bind(&credit_account)
    .bind(amount_minor)
    .bind(format!("Accrual CR: {}", instance_name))
    .execute(&mut *tx)
    .await
    .map_err(AccrualError::Database)?;

    // Mark event as processed (idempotency for journal posting)
    sqlx::query(
        r#"
        INSERT INTO processed_events (event_id, event_type, processor)
        VALUES ($1, $2, 'gl-accrual')
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_ACCRUAL_CREATED)
    .execute(&mut *tx)
    .await
    .map_err(AccrualError::Database)?;

    // 2. Insert accrual instance
    sqlx::query(
        r#"
        INSERT INTO gl_accrual_instances (
            instance_id, template_id, tenant_id, accrual_id, period,
            posting_date, name, debit_account, credit_account,
            amount_minor, currency, reversal_policy, cashflow_class,
            journal_entry_id, status, idempotency_key, outbox_event_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, 'posted', $15, $16)
        "#,
    )
    .bind(instance_id)
    .bind(req.template_id)
    .bind(&req.tenant_id)
    .bind(accrual_id)
    .bind(&req.period)
    .bind(posting_date)
    .bind(&instance_name)
    .bind(&debit_account)
    .bind(&credit_account)
    .bind(amount_minor)
    .bind(&currency)
    .bind(&reversal_json)
    .bind(&cashflow_str)
    .bind(journal_entry_id)
    .bind(&idem_key)
    .bind(event_id)
    .execute(&mut *tx)
    .await
    .map_err(AccrualError::Database)?;

    // 3. Emit gl.accrual_created outbox event
    let payload = AccrualCreatedPayload {
        accrual_id,
        template_id: Some(req.template_id),
        tenant_id: req.tenant_id.clone(),
        name: instance_name.clone(),
        period: req.period.clone(),
        posting_date: req.posting_date.clone(),
        debit_account: debit_account.clone(),
        credit_account: credit_account.clone(),
        amount_minor,
        currency: currency.clone(),
        cashflow_class,
        reversal_policy,
        journal_entry_id: Some(journal_entry_id),
        description: instance_name.clone(),
        created_at: Utc::now(),
    };

    let event_payload = serde_json::to_value(&payload).map_err(AccrualError::Serialization)?;

    outbox_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_ACCRUAL_CREATED,
        "accrual",
        &accrual_id.to_string(),
        event_payload,
        MUTATION_CLASS_DATA_MUTATION,
    )
    .await
    .map_err(AccrualError::Database)?;

    tx.commit().await.map_err(AccrualError::Database)?;

    Ok(AccrualResult {
        instance_id,
        accrual_id,
        template_id: req.template_id,
        period: req.period.clone(),
        journal_entry_id,
        amount_minor,
        currency,
        status: "posted".to_string(),
        idempotent_hit: false,
    })
}

// ============================================================================
// Auto-reversal execution (bd-2ob)
// ============================================================================

/// Execute auto-reversals for all accruals scheduled to reverse in `target_period`.
///
/// Finds accrual instances from the prior period that have
/// `reversal_policy.auto_reverse_next_period = true` and have not yet been reversed.
/// For each, posts a compensating (reversed debit/credit) journal entry atomically
/// with the reversal record and outbox event.
///
/// ## Exactly-once guarantee
/// - Deterministic reversal_id = Uuid::v5(original_accrual_id, "reversal")
/// - Deterministic event_id = Uuid::v5(original_accrual_id, "reversal_event")
/// - processed_events dedupe prevents duplicate journal postings on replay
/// - gl_accrual_reversals.original_accrual_id UNIQUE constraint prevents double reversal
///
/// ## Idempotency
/// If a reversal already exists (idempotency_key match), it is counted as
/// `reversals_skipped` and the existing record is returned.
pub async fn execute_auto_reversals(
    db: &PgPool,
    req: &ExecuteReversalsRequest,
) -> Result<ExecuteReversalsResult, AccrualError> {
    let reversal_date = NaiveDate::parse_from_str(&req.reversal_date, "%Y-%m-%d")
        .map_err(|e| AccrualError::Validation(format!("Invalid reversal_date: {}", e)))?;

    // Find accrual instances eligible for auto-reversal into target_period:
    // - belongs to tenant
    // - has reversal_policy.auto_reverse_next_period = true
    // - has status = 'posted'
    // - has NOT already been reversed (no row in gl_accrual_reversals)
    // - the accrual period is the period immediately before target_period
    let prior_period = compute_prior_period(&req.target_period).ok_or_else(|| {
        AccrualError::Validation(format!("Invalid target_period: {}", req.target_period))
    })?;

    let candidates = sqlx::query(
        r#"
        SELECT i.instance_id, i.accrual_id, i.template_id, i.tenant_id,
               i.period, i.debit_account, i.credit_account,
               i.amount_minor, i.currency, i.reversal_policy, i.cashflow_class,
               i.journal_entry_id AS original_journal_entry_id, i.outbox_event_id
        FROM gl_accrual_instances i
        WHERE i.tenant_id = $1
          AND i.period = $2
          AND i.status = 'posted'
          AND (i.reversal_policy->>'auto_reverse_next_period')::boolean = true
          AND NOT EXISTS (
              SELECT 1 FROM gl_accrual_reversals r
              WHERE r.original_accrual_id = i.accrual_id
          )
        ORDER BY i.created_at
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&prior_period)
    .fetch_all(db)
    .await
    .map_err(AccrualError::Database)?;

    let mut results = Vec::new();
    let mut reversals_executed = 0usize;
    let mut reversals_skipped = 0usize;

    for row in &candidates {
        let instance_id: Uuid = row.get("instance_id");
        let accrual_id: Uuid = row.get("accrual_id");
        let template_id: Uuid = row.get("template_id");
        let original_debit: String = row.get("debit_account");
        let original_credit: String = row.get("credit_account");
        let amount_minor: i64 = row.get("amount_minor");
        let currency: String = row.get("currency");
        let cashflow_str: String = row.get("cashflow_class");
        let original_event_id: Option<Uuid> = row.get("outbox_event_id");

        // Deterministic IDs for exactly-once
        let reversal_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("reversal:{}", accrual_id).as_bytes(),
        );
        let event_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("reversal_event:{}", accrual_id).as_bytes(),
        );
        let idem_key = format!("reversal:{}:{}", accrual_id, req.target_period);

        // Idempotency check: already reversed?
        let existing = sqlx::query(
            r#"
            SELECT reversal_id, journal_entry_id, amount_minor, currency
            FROM gl_accrual_reversals
            WHERE idempotency_key = $1
            "#,
        )
        .bind(&idem_key)
        .fetch_optional(db)
        .await
        .map_err(AccrualError::Database)?;

        if let Some(existing_row) = existing {
            reversals_skipped += 1;
            results.push(ReversalResult {
                reversal_id: existing_row.get("reversal_id"),
                original_accrual_id: accrual_id,
                journal_entry_id: existing_row.get("journal_entry_id"),
                amount_minor: existing_row.get("amount_minor"),
                currency: existing_row.get("currency"),
                idempotent_hit: true,
            });
            continue;
        }

        // Check processed_events for this event_id (replay dedupe)
        let already_processed: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM processed_events WHERE event_id = $1)")
                .bind(event_id)
                .fetch_one(db)
                .await
                .map_err(AccrualError::Database)?;

        if already_processed {
            reversals_skipped += 1;
            continue;
        }

        // Reversal swaps debit/credit accounts
        let rev_debit = original_credit.clone();
        let rev_credit = original_debit.clone();

        let posted_at = reversal_date.and_hms_opt(0, 0, 0).unwrap().and_utc();

        let reversal_name = format!("Reversal of accrual {}", accrual_id);
        let cashflow_class = parse_cashflow_class(&cashflow_str);

        // Begin atomic transaction
        let mut tx = db.begin().await.map_err(AccrualError::Database)?;

        // 1. Post reversing journal entry
        let journal_entry_id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO journal_entries (
                id, tenant_id, source_module, source_event_id, source_subject,
                posted_at, currency, description, reference_type, reference_id, correlation_id
            )
            VALUES ($1, $2, 'gl', $3, 'accrual_reversal', $4, $5, $6, 'GL_ACCRUAL_REVERSAL', $7, $8)
            "#,
        )
        .bind(journal_entry_id)
        .bind(&req.tenant_id)
        .bind(event_id)
        .bind(posted_at)
        .bind(&currency)
        .bind(&reversal_name)
        .bind(reversal_id.to_string())
        .bind(Some(accrual_id))
        .execute(&mut *tx)
        .await
        .map_err(AccrualError::Database)?;

        // Debit line (swapped: was credit in original)
        sqlx::query(
            r#"
            INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
            VALUES ($1, $2, 1, $3, $4, 0, $5)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(journal_entry_id)
        .bind(&rev_debit)
        .bind(amount_minor)
        .bind(format!("Reversal DR: {}", reversal_name))
        .execute(&mut *tx)
        .await
        .map_err(AccrualError::Database)?;

        // Credit line (swapped: was debit in original)
        sqlx::query(
            r#"
            INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
            VALUES ($1, $2, 2, $3, 0, $4, $5)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(journal_entry_id)
        .bind(&rev_credit)
        .bind(amount_minor)
        .bind(format!("Reversal CR: {}", reversal_name))
        .execute(&mut *tx)
        .await
        .map_err(AccrualError::Database)?;

        // 2. Mark event as processed (dedupe on replay)
        sqlx::query(
            r#"
            INSERT INTO processed_events (event_id, event_type, processor)
            VALUES ($1, $2, 'gl-accrual-reversal')
            ON CONFLICT (event_id) DO NOTHING
            "#,
        )
        .bind(event_id)
        .bind(EVENT_TYPE_ACCRUAL_REVERSED)
        .execute(&mut *tx)
        .await
        .map_err(AccrualError::Database)?;

        // 3. Insert reversal record with linkage
        sqlx::query(
            r#"
            INSERT INTO gl_accrual_reversals (
                reversal_id, original_accrual_id, original_instance_id,
                tenant_id, reversal_period, reversal_date,
                debit_account, credit_account, amount_minor, currency,
                journal_entry_id, outbox_event_id, idempotency_key, reason
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            "#,
        )
        .bind(reversal_id)
        .bind(accrual_id)
        .bind(instance_id)
        .bind(&req.tenant_id)
        .bind(&req.target_period)
        .bind(reversal_date)
        .bind(&rev_debit)
        .bind(&rev_credit)
        .bind(amount_minor)
        .bind(&currency)
        .bind(journal_entry_id)
        .bind(event_id)
        .bind(&idem_key)
        .bind("auto_reverse_next_period")
        .execute(&mut *tx)
        .await
        .map_err(AccrualError::Database)?;

        // 4. Update accrual instance status to 'reversed'
        sqlx::query("UPDATE gl_accrual_instances SET status = 'reversed' WHERE instance_id = $1")
            .bind(instance_id)
            .execute(&mut *tx)
            .await
            .map_err(AccrualError::Database)?;

        // 5. Emit gl.accrual_reversed outbox event
        let payload = AccrualReversedPayload {
            reversal_id,
            original_accrual_id: accrual_id,
            template_id: Some(template_id),
            tenant_id: req.tenant_id.clone(),
            reversal_period: req.target_period.clone(),
            reversal_date: req.reversal_date.clone(),
            debit_account: rev_debit.clone(),
            credit_account: rev_credit.clone(),
            amount_minor,
            currency: currency.clone(),
            cashflow_class,
            journal_entry_id: Some(journal_entry_id),
            reason: "auto_reverse_next_period".to_string(),
            reversed_at: Utc::now(),
        };

        let event_payload = serde_json::to_value(&payload).map_err(AccrualError::Serialization)?;

        outbox_repo::insert_outbox_event_with_linkage(
            &mut tx,
            event_id,
            EVENT_TYPE_ACCRUAL_REVERSED,
            "accrual",
            &accrual_id.to_string(),
            event_payload,
            original_event_id, // reverses_event_id — links to original accrual's outbox event
            None,              // supersedes_event_id — not applicable
            MUTATION_CLASS_REVERSAL,
        )
        .await
        .map_err(AccrualError::Database)?;

        tx.commit().await.map_err(AccrualError::Database)?;

        reversals_executed += 1;
        results.push(ReversalResult {
            reversal_id,
            original_accrual_id: accrual_id,
            journal_entry_id,
            amount_minor,
            currency,
            idempotent_hit: false,
        });
    }

    Ok(ExecuteReversalsResult {
        target_period: req.target_period.clone(),
        reversals_executed,
        reversals_skipped,
        results,
    })
}

// ============================================================================
// Helpers
// ============================================================================

/// Compute the period (YYYY-MM) immediately before the given period.
fn compute_prior_period(period: &str) -> Option<String> {
    let parts: Vec<&str> = period.split('-').collect();
    if parts.len() != 2 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    if month == 1 {
        Some(format!("{}-12", year - 1))
    } else {
        Some(format!("{}-{:02}", year, month - 1))
    }
}

fn parse_cashflow_class(s: &str) -> CashFlowClass {
    match s {
        "investing" => CashFlowClass::Investing,
        "financing" => CashFlowClass::Financing,
        "non_cash" => CashFlowClass::NonCash,
        _ => CashFlowClass::Operating,
    }
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum AccrualError {
    Database(sqlx::Error),
    Validation(String),
    Serialization(serde_json::Error),
}

impl std::fmt::Display for AccrualError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccrualError::Database(e) => write!(f, "database error: {}", e),
            AccrualError::Validation(msg) => write!(f, "validation error: {}", msg),
            AccrualError::Serialization(e) => write!(f, "serialization error: {}", e),
        }
    }
}
