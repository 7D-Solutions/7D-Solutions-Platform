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

## Running the Backend

```bash
# Required env vars
DATABASE_URL=postgres://...
BUS_TYPE=nats          # or "in_memory"
NATS_URL=nats://...    # required when BUS_TYPE=nats
CORS_ORIGINS=http://localhost:5173   # match the frontend dev server origin; use * for dev
HOST=0.0.0.0
PORT=8102

cargo run -p pdf-editor-rs
```

## Standalone Frontend

The frontend is a React + Vite + TypeScript + Zustand app in `frontend/`. It connects to the Rust backend for PDF processing and form CRUD. Annotations live in the browser (Zustand stores with localStorage persistence).

### Setup

```bash
cd modules/pdf-editor/frontend
npm install
```

### Environment

Set `VITE_PDF_API_BASE_URL` to point at the running backend. Default: `http://localhost:3100`.

Create `.env.local`:
```
VITE_PDF_API_BASE_URL=http://localhost:8102
```

The backend must have `CORS_ORIGINS` set to allow the frontend's origin (e.g. `http://localhost:5173` for the Vite dev server).

### Stores

| Store | Purpose |
|-------|---------|
| `annotationStore` | Annotation tool selection, drag/edit state, render-to-PDF action |
| `formStore` | Template/field/submission CRUD via API client, local field data |
| `uiStore` | PDF file state (browser-local), mode, processing status |
| `pdfTabStore` | Multi-tab PDF management with per-tab annotations and undo/redo |
| `toolbarStore` | Customizable toolbar buttons (persisted to localStorage) |
| `sidebarStore` | Sidebar panel mode |
| `viewportStore` | Zoom, rotation, text search |
| `notificationStore` | Toast notifications and confirm dialogs |

### Running Tests

```bash
npm test          # run once
npm run test:watch  # watch mode
```

### Smoke Test Checklist (Manual, Against Running Backend)

1. Open a PDF from the local computer (File picker)
2. Add annotations (stamp, text, arrow, highlight) in the browser
3. Call render-annotations: PDF bytes + annotations sent to backend, receive burned PDF
4. Create a form template with fields
5. Create a draft submission, autosave field data
6. Submit the submission
7. Generate a filled PDF from the submission

## Contract Tests

```bash
cargo test -p contract-tests --test openapi_tests -- test_pdf_editor
```
