# PDF Editor Module (pdf-editor-rs)

Stateless PDF processing engine for annotation rendering, form templating, form filling, and PDF generation.

## Design

The editor **never stores PDF files**. The caller provides PDF bytes, the editor processes them (burn annotations, fill form data), and returns the result. The caller manages file storage.

Two modes of operation:
- **Standalone:** Own React web UI — open PDF from computer, annotate, save back
- **API integration:** Any app sends PDF bytes to the REST API for processing

## API Surface

Full OpenAPI spec: [`contracts/pdf-editor/pdf-editor-v0.1.0.yaml`](../../contracts/pdf-editor/pdf-editor-v0.1.0.yaml)

### PDF Processing (Stateless)

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/pdf/render-annotations` | PDF bytes + annotation JSON -> annotated PDF bytes |
| POST | `/api/pdf/forms/submissions/{id}/generate` | PDF bytes + submission ID -> filled PDF bytes |

Both endpoints accept `multipart/form-data` with a `file` field containing PDF bytes. The render-annotations endpoint has a 50 MB body limit.

### Form Templates

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/pdf/forms/templates` | Create template |
| GET | `/api/pdf/forms/templates` | List templates |
| GET | `/api/pdf/forms/templates/{id}` | Get template |
| PUT | `/api/pdf/forms/templates/{id}` | Update template |

### Form Fields

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/pdf/forms/templates/{id}/fields` | Create field |
| GET | `/api/pdf/forms/templates/{id}/fields` | List fields |
| PUT | `/api/pdf/forms/templates/{tid}/fields/{fid}` | Update field |
| POST | `/api/pdf/forms/templates/{id}/fields/reorder` | Reorder fields |

Field types: `text`, `number`, `date`, `dropdown`, `checkbox`.

### Form Submissions

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/pdf/forms/submissions` | Create draft |
| GET | `/api/pdf/forms/submissions` | List submissions |
| GET | `/api/pdf/forms/submissions/{id}` | Get submission |
| PUT | `/api/pdf/forms/submissions/{id}` | Autosave field_data |
| POST | `/api/pdf/forms/submissions/{id}/submit` | Validate and submit |

Status machine: `draft` -> `submitted` (no regression).

### Ops

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/healthz` | Liveness probe |
| GET | `/api/health` | Health check |
| GET | `/api/ready` | Readiness probe (DB check) |
| GET | `/api/version` | Module version |
| GET | `/metrics` | Prometheus metrics |

## Multi-Tenancy

All data-bearing endpoints require `tenant_id` — either in the request body (create) or as a query parameter (read/update). Every DB query filters by `tenant_id`.

## Events

| Event | When | Key Payload |
|-------|------|-------------|
| `pdf.form.submitted` | Submission transitions to submitted | tenant_id, submission_id, template_id |
| `pdf.form.generated` | Filled PDF returned from generate | tenant_id, submission_id, template_id |

Events use the platform EventEnvelope pattern via the events_outbox table.

## Running

```bash
# Required env vars
DATABASE_URL=postgres://...
BUS_TYPE=nats          # or "in_memory"
NATS_URL=nats://...    # required when BUS_TYPE=nats
CORS_ORIGINS=*         # or comma-separated origins
HOST=0.0.0.0
PORT=8092

cargo run -p pdf-editor-rs
```

## Contract Tests

```bash
cargo test -p contract-tests --test openapi_tests -- test_pdf_editor
```
