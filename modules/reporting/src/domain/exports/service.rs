use super::models::{ExportCompletedPayload, ExportFormat, ExportRow, ExportRun};
use event_bus::outbox::validate_and_serialize_envelope;
use event_bus::EventEnvelope;
use sqlx::PgPool;
use uuid::Uuid;

/// Execute a report export using Guard → Mutation → Outbox.
///
/// 1. Guard: check idempotency_key for duplicates
/// 2. Mutation: insert export run, generate output, update status
/// 3. Outbox: enqueue completion event atomically with status update
pub async fn run_export(
    pool: &PgPool,
    tenant_id: &str,
    report_id: &str,
    format: ExportFormat,
    idempotency_key: Option<&str>,
) -> Result<ExportRun, anyhow::Error> {
    // ── Guard: idempotency ──────────────────────────────────────────
    if let Some(key) = idempotency_key {
        let existing: Option<ExportRun> = sqlx::query_as(
            "SELECT * FROM rpt_export_runs WHERE tenant_id = $1 AND idempotency_key = $2",
        )
        .bind(tenant_id)
        .bind(key)
        .fetch_optional(pool)
        .await?;

        if let Some(run) = existing {
            return Ok(run);
        }
    }

    // ── Mutation: create export run ──────────────────────────────────
    let run_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"INSERT INTO rpt_export_runs (id, tenant_id, report_id, format, status, idempotency_key)
           VALUES ($1, $2, $3, $4, 'running', $5)"#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .bind(report_id)
    .bind(format.as_str())
    .bind(idempotency_key)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // ── Generate export data ────────────────────────────────────────
    let rows = fetch_report_data(pool, tenant_id, report_id).await?;
    let row_count = rows.len() as i32;

    let output_bytes = match format {
        ExportFormat::Csv => generate_csv(&rows)?,
        ExportFormat::Xlsx => generate_xlsx(&rows)?,
        ExportFormat::Pdf => generate_pdf(&rows)?,
    };

    // output_ref is a deterministic reference (in production this would be an
    // object-store key; here we use a hash-based reference for testability)
    let output_ref = format!(
        "exports/{}/{}/{}.{}",
        tenant_id,
        report_id,
        run_id,
        format.as_str()
    );

    // ── Mutation + Outbox: complete export atomically ───────────────
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"UPDATE rpt_export_runs
           SET status = 'completed', row_count = $1, output_ref = $2, completed_at = NOW()
           WHERE id = $3 AND tenant_id = $4"#,
    )
    .bind(row_count)
    .bind(&output_ref)
    .bind(run_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    // Outbox event
    let event_payload = ExportCompletedPayload {
        export_run_id: run_id,
        report_id: report_id.to_string(),
        format: format.as_str().to_string(),
        row_count,
        output_ref: output_ref.clone(),
    };

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "reporting".to_string(),
        "reporting.export.completed".to_string(),
        event_payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_mutation_class(Some("SIDE_EFFECT".to_string()));

    let payload_json = validate_and_serialize_envelope(&envelope).map_err(|e| {
        anyhow::anyhow!("Envelope validation failed: {}", e)
    })?;

    sqlx::query(
        r#"INSERT INTO events_outbox (
               event_id, event_type, aggregate_type, aggregate_id, payload,
               tenant_id, source_module, source_version, schema_version,
               occurred_at, replay_safe, trace_id, correlation_id, causation_id,
               reverses_event_id, supersedes_event_id, side_effect_id, mutation_class
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)"#,
    )
    .bind(envelope.event_id)
    .bind(&envelope.event_type)
    .bind("export_run")
    .bind(run_id.to_string())
    .bind(payload_json)
    .bind(&envelope.tenant_id)
    .bind(&envelope.source_module)
    .bind(&envelope.source_version)
    .bind(&envelope.schema_version)
    .bind(envelope.occurred_at)
    .bind(envelope.replay_safe)
    .bind(&envelope.trace_id)
    .bind(&envelope.correlation_id)
    .bind(&envelope.causation_id)
    .bind(envelope.reverses_event_id)
    .bind(envelope.supersedes_event_id)
    .bind(&envelope.side_effect_id)
    .bind(&envelope.mutation_class)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // ── Return completed run ────────────────────────────────────────
    let run: ExportRun = sqlx::query_as(
        "SELECT * FROM rpt_export_runs WHERE id = $1 AND tenant_id = $2",
    )
    .bind(run_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    // Store the bytes alongside (in production: object store write).
    // For now we keep the bytes in-memory; the output_ref is the durable pointer.
    let _ = output_bytes; // consumed by object-store layer in production

    Ok(run)
}

/// List export runs for a tenant (tenant-scoped query).
pub async fn list_export_runs(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<ExportRun>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM rpt_export_runs WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

// ── Data fetching ─────────────────────────────────────────────────────────

async fn fetch_report_data(
    pool: &PgPool,
    tenant_id: &str,
    _report_id: &str,
) -> Result<Vec<ExportRow>, sqlx::Error> {
    // For now exports pull from the trial balance cache.
    // report_id can route to different source tables in future.
    let rows = sqlx::query_as::<_, (String, String, String, i64, i64, i64)>(
        r#"SELECT account_code, account_name, currency, debit_minor, credit_minor, net_minor
           FROM rpt_trial_balance_cache
           WHERE tenant_id = $1
           ORDER BY account_code"#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(account_code, account_name, currency, debit_minor, credit_minor, net_minor)| {
            ExportRow {
                account_code,
                account_name,
                currency,
                debit_minor,
                credit_minor,
                net_minor,
            }
        })
        .collect())
}

// ── Format generators ─────────────────────────────────────────────────────

fn generate_csv(rows: &[ExportRow]) -> Result<Vec<u8>, anyhow::Error> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["account_code", "account_name", "currency", "debit", "credit", "net"])?;
    for row in rows {
        wtr.write_record([
            &row.account_code,
            &row.account_name,
            &row.currency,
            &row.debit_minor.to_string(),
            &row.credit_minor.to_string(),
            &row.net_minor.to_string(),
        ])?;
    }
    Ok(wtr.into_inner()?)
}

fn generate_xlsx(rows: &[ExportRow]) -> Result<Vec<u8>, anyhow::Error> {
    use rust_xlsxwriter::Workbook;

    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();

    // Header
    let headers = ["Account Code", "Account Name", "Currency", "Debit", "Credit", "Net"];
    for (col, header) in headers.iter().enumerate() {
        sheet.write_string(0, col as u16, *header)?;
    }

    // Data rows
    for (i, row) in rows.iter().enumerate() {
        let r = (i + 1) as u32;
        sheet.write_string(r, 0, &row.account_code)?;
        sheet.write_string(r, 1, &row.account_name)?;
        sheet.write_string(r, 2, &row.currency)?;
        sheet.write_number(r, 3, row.debit_minor as f64)?;
        sheet.write_number(r, 4, row.credit_minor as f64)?;
        sheet.write_number(r, 5, row.net_minor as f64)?;
    }

    let buf = workbook.save_to_buffer()?;
    Ok(buf)
}

fn generate_pdf(rows: &[ExportRow]) -> Result<Vec<u8>, anyhow::Error> {
    use printpdf::*;

    let (doc, page1, layer1) =
        PdfDocument::new("Report Export", Mm(210.0), Mm(297.0), "Layer 1");
    let layer = doc.get_page(page1).get_layer(layer1);

    let font = doc.add_builtin_font(BuiltinFont::Helvetica)?;

    // Title
    layer.use_text("Report Export", 16.0, Mm(20.0), Mm(280.0), &font);

    // Header row
    let y_start = 265.0;
    let headers = ["Code", "Name", "Currency", "Debit", "Credit", "Net"];
    let x_positions = [20.0, 45.0, 100.0, 130.0, 155.0, 180.0];
    for (i, header) in headers.iter().enumerate() {
        layer.use_text(*header, 10.0, Mm(x_positions[i]), Mm(y_start), &font);
    }

    // Data rows (fit what we can on one page)
    let max_rows = rows.len().min(40);
    for (i, row) in rows.iter().take(max_rows).enumerate() {
        let y = y_start - 7.0 - (i as f32 * 6.0);
        if y < 20.0 {
            break;
        }
        layer.use_text(&row.account_code, 8.0, Mm(x_positions[0]), Mm(y), &font);
        layer.use_text(&row.account_name, 8.0, Mm(x_positions[1]), Mm(y), &font);
        layer.use_text(&row.currency, 8.0, Mm(x_positions[2]), Mm(y), &font);
        layer.use_text(
            &row.debit_minor.to_string(),
            8.0,
            Mm(x_positions[3]),
            Mm(y),
            &font,
        );
        layer.use_text(
            &row.credit_minor.to_string(),
            8.0,
            Mm(x_positions[4]),
            Mm(y),
            &font,
        );
        layer.use_text(
            &row.net_minor.to_string(),
            8.0,
            Mm(x_positions[5]),
            Mm(y),
            &font,
        );
    }

    Ok(doc.save_to_bytes()?)
}
