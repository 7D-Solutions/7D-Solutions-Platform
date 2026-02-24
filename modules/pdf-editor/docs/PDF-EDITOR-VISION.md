# Fireproof PDF Editor — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.x)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | Platform Orchestrator | Initial vision doc — business problem, user stories, design principles, MVP scope, data model, events, structural decisions, decision log, bead plan |

---

## The Business Problem

Every small business that handles documents — purchase orders, inspection reports, compliance forms, permits, contracts — hits the same wall: **PDF tools are either expensive, clunky, or both.**

Adobe Acrobat costs $240/year per seat. Free tools lose your work, lack form support, and require uploading sensitive documents to third-party servers. Field workers on tablets struggle with tools designed for desktop. Form fields misalign. Drafts vanish on refresh. Multi-person signing chains are slow and confusing.

The result: people print PDFs, mark them up by hand, scan them back in, and email them around. In 2026.

The businesses that need this most — contractors approving POs, inspectors filling compliance forms, managers signing off on work orders — are small teams that can't afford enterprise document management but desperately need to mark up, fill, and generate PDFs reliably.

---

## What the Module Does

The Fireproof PDF Editor is a **platform module for PDF annotation, form templating, form filling, and PDF generation.** It handles the complete document lifecycle from upload through final generated output.

It answers four questions:
1. **What document am I working with?** — Upload a PDF, store it durably in S3, download it whenever needed.
2. **What marks need to go on it?** — Annotations (stamps, text, shapes, arrows, signatures, highlights) positioned precisely on the PDF and saved automatically.
3. **What data needs to be collected?** — Form templates define reusable field layouts positioned on a PDF. Submissions fill those fields with validated data.
4. **What's the final output?** — Generated PDFs with annotations permanently rendered or form data filled into the template positions.

**Two modes of operation:**
- **Standalone:** Own web UI (React + Vite) for direct use — no other app required. The owner uses it today to approve POs while Fireproof ERP is still in development.
- **API integration:** Any vertical app calls the module's REST API to upload, annotate, template, fill, and generate PDFs.

---

## Who Uses This

### Document Owner / Office Manager
- Uploads PDFs that need markup or form filling
- Annotates documents with stamps, text, arrows, shapes, signatures
- Generates final marked-up PDFs for distribution or archival
- Needs autosave so work is never lost

### Form Designer
- Creates reusable form templates tied to a PDF document
- Defines field positions, types, validation rules, and display order
- Drag-and-drop field positioning on the PDF canvas
- Reuses templates across many submissions

### Form Filler / Field Worker
- Opens a form template and fills in data
- Saves drafts automatically (never loses work)
- Submits completed forms for processing
- Works from phone/tablet in the field (v2 — mobile optimization)

### Vertical App Developer
- Integrates PDF capabilities via REST API
- Creates templates programmatically
- Submits form data and generates filled PDFs
- Listens for events (document.uploaded, form.submitted, document.generated)

---

## Design Principles

1. **Self-hosted first.** Sensitive documents never leave the user's infrastructure. S3-compatible storage (MinIO for dev, AWS for prod). No third-party document services.

2. **Standalone before integration.** The module must work with its own web UI before any other app exists. A user can run the PDF editor with just infrastructure services and nothing else.

3. **No lost work.** Autosave on annotations and form drafts. Optimistic locking on annotations prevents concurrent overwrites. Drafts persist across sessions.

4. **PDF files in object storage, metadata in Postgres.** Never store PDF blobs in the database. S3 for durability and streaming, Postgres for indexing and querying.

5. **Simple status machine.** draft → submitted for v1. Approval workflows, multi-person signing chains, and interactive PDF form fields are v2.

6. **Platform-compliant.** Multi-tenant via tenant_id, Guard→Mutation→Outbox pattern, EventEnvelope on NATS, standard health/metrics endpoints. Plugs into the platform like every other module.

---

## MVP Scope

### In MVP (v0.1.x)

- Upload a PDF, annotate it (stamps, text, shapes, signatures), save permanently
- Form templates: define fields on a PDF, reuse across submissions
- Form filling: submit data, validate, store as draft, submit when ready
- PDF generation: render filled PDF from template + submission data
- PDF generation: render annotations permanently onto a PDF
- Auto-save drafts (annotations and form submissions)
- Optimistic locking on annotation autosave (version + If-Match)
- Self-hosted (S3-compatible storage, no third-party uploads)
- Standalone web UI accessible without any other app running
- CORS configuration for standalone frontend
- File validation on upload (magic bytes, size limits, content-type check)
- Events: pdf.document.uploaded, pdf.form.submitted, pdf.document.generated
- OpenAPI contract for API consumers

### NOT in MVP

- Mobile-optimized experience (responsive layout, touch-friendly annotation)
- Multi-person signing/approval chains
- Interactive PDF form fields (AcroForm / XFA)
- Password protection / PDF encryption
- Integration with other platform modules via events (API works, event consumers come later)
- Virus scanning on uploaded files
- CRDT-based concurrent editing (optimistic locking is sufficient for v1)

---

## Technology Summary

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Backend | Rust + Axum + SQLx | Platform standard |
| Database | PostgreSQL | Platform standard, JSONB for flexible fields |
| File storage | S3-compatible (MinIO dev, AWS prod) | Durable, streaming, tenant-isolated keys |
| PDF rendering | pdfium-render | Read/write/overlay PDFs, active Rust crate |
| PDF fallback | lopdf | If pdfium-render can't handle a specific annotation type |
| Event bus | NATS (via outbox) | Platform standard EventEnvelope pattern |
| Frontend | React + Vite + TypeScript + Zustand | Existing frontend kept as-is, API calls repointed |
| Auth | Platform security crate (JWT) | Standard platform tenant_id + user_id extraction |

---

## Data Model

### Tables

**pdf_documents**
- id, tenant_id, name, storage_key (S3 reference), content_type, size_bytes, uploaded_by, created_at

**annotation_sets**
- id, tenant_id, pdf_document_id (FK), created_by, annotations (JSONB — array of annotation objects with type, position, content), version (INTEGER, for optimistic locking), created_at, updated_at

**form_templates**
- id, tenant_id, name, description, pdf_document_id (FK), created_by, created_at, updated_at

**form_fields**
- id, template_id (FK), field_key, field_label, field_type (text|number|date|dropdown|checkbox), validation_rules (JSONB), pdf_position (JSONB — x, y, width, height, page), display_order

**form_submissions**
- id, tenant_id, template_id (FK), submitted_by, status (draft|submitted), field_data (JSONB), created_at, submitted_at

**generated_documents**
- id, tenant_id, source_type (submission|annotation), source_id, storage_key (S3 reference), created_at

### S3 Key Convention

```
tenant/{tenant_id}/pdf/{document_id}/original.pdf
tenant/{tenant_id}/generated/{generated_id}.pdf
```

---

## Events

| Event | When | Payload |
|-------|------|---------|
| pdf.document.uploaded | PDF upload completes | tenant_id, document_id, uploaded_by, size_bytes, content_type |
| pdf.form.submitted | Form submission transitions to submitted | tenant_id, submission_id, template_id, pdf_document_id, submitted_by |
| pdf.document.generated | Generated PDF created (from annotations or submission) | tenant_id, generated_document_id, source_type, source_id, pdf_document_id |

---

## API Surface (Summary)

| Method | Path | Purpose |
|--------|------|---------|
| POST | /api/pdf/documents | Upload PDF (multipart) |
| GET | /api/pdf/documents/:id | Document metadata |
| GET | /api/pdf/documents/:id/download | Stream PDF bytes |
| GET | /api/pdf/documents/:id/annotations | Get annotations + version |
| PUT | /api/pdf/documents/:id/annotations | Upsert annotations (If-Match required) |
| POST | /api/pdf/documents/:id/render-annotations | Render annotations into PDF |
| POST | /api/pdf/forms/templates | Create form template |
| GET | /api/pdf/forms/templates | List templates |
| GET | /api/pdf/forms/templates/:id | Get template |
| PUT | /api/pdf/forms/templates/:id | Update template |
| POST | /api/pdf/forms/templates/:id/fields | Create field |
| GET | /api/pdf/forms/templates/:id/fields | List fields |
| PUT | /api/pdf/forms/fields/:id | Update field |
| POST | /api/pdf/forms/submissions | Create submission (draft) |
| PUT | /api/pdf/forms/submissions/:id | Autosave field_data |
| POST | /api/pdf/forms/submissions/:id/submit | Validate and submit |
| GET | /api/pdf/forms/submissions/:id | Get submission |
| POST | /api/pdf/forms/submissions/:id/generate | Generate filled PDF |

---

## Structural Decisions

### SD-1: One module, not two
**Decision:** Annotations and forms live in the same module.
**Rationale:** They share the same PDF documents, same S3 storage, same rendering pipeline. Splitting would create cross-module dependencies for basic operations. Single deployment, single database, shared code.
**Source:** Grok stress test (2026-02-24)

### SD-2: S3 object storage for PDF files
**Decision:** PDFs stored in S3-compatible object storage (MinIO dev, AWS prod). Database stores references only.
**Rationale:** PDF files can be large (multi-MB). Postgres is not designed for blob storage. S3 provides streaming, durability, and tenant isolation via key prefixes.
**Source:** Grok stress test (2026-02-24)

### SD-3: pdfium-render for PDF manipulation
**Decision:** Use pdfium-render Rust crate for reading, overlaying, and writing PDFs.
**Rationale:** Active crate (v0.8.37, Feb 2026), supports high-level edit/annotate/form-fill on existing PDFs. The Node.js version used pdf-lib and pdfkit — pdfium-render is the closest Rust equivalent.
**Gotchas:** PDFium binary must be bundled (dynamic load), thread_safe feature required for concurrent use, watch for lifetime quirks on long-lived document handles. Keep handles short-lived per request.
**Source:** Grok stress test (2026-02-24)

### SD-4: Simplified status machine for v1
**Decision:** Form submissions use draft → submitted only. No approved/rejected states.
**Rationale:** Approval workflows require multi-person chains, permissions, and notification integration. That's v2 complexity. For v1, the user submits and the consuming app decides what to do with it.
**Source:** Grok stress test (2026-02-24)

### SD-5: Optimistic locking on annotations
**Decision:** annotation_sets table has a version column. PUT requires If-Match header. Conflicting writes return 409.
**Rationale:** Last-write-wins autosave will lose data when two users edit the same PDF's annotations simultaneously. Optimistic locking prevents silent data loss while keeping the API simple.
**Source:** Grok adversarial review (2026-02-24)

### SD-6: File validation on upload
**Decision:** Upload validates magic bytes (%PDF-), content-type, and configurable size limit (50MB default).
**Rationale:** Prevents non-PDF uploads, PDF bomb attacks, and memory exhaustion from oversized files.
**Source:** Grok adversarial review (2026-02-24)

### SD-7: Standalone + integration dual mode
**Decision:** Module runs with its own React web UI for standalone use AND exposes REST API for app integration.
**Rationale:** Owner needs to use the PDF editor today (approving POs) before Fireproof ERP is complete. Other apps need API access. CORS is configured for standalone frontend.
**Source:** User requirement (2026-02-24)

### SD-8: Frontend kept as-is, API repointed
**Decision:** Existing React + Vite + TypeScript + Zustand frontend is preserved. Only API calls are changed to point at the new Rust backend.
**Rationale:** Frontend works today for annotations. Rewriting it would double the effort without adding value. Create a typed API client layer, then update stores.
**Source:** Planning decision (2026-02-24)

---

## Decision Log

| # | Date | Decision | Rationale |
|---|------|----------|-----------|
| 1 | 2026-02-24 | Convert from Node.js/Express/MySQL to Rust/Axum/SQLx/PostgreSQL | Platform standard. Module must integrate with platform event bus, auth, and multi-tenant patterns. |
| 2 | 2026-02-24 | Keep React frontend, repoint API calls | Working annotation UI exists. No reason to rewrite. |
| 3 | 2026-02-24 | One module for annotations + forms (not split) | Shared PDF documents, storage, rendering pipeline. |
| 4 | 2026-02-24 | S3 for file storage, DB for metadata only | PDFs are too large for Postgres. S3 provides streaming and tenant isolation. |
| 5 | 2026-02-24 | pdfium-render for PDF rendering | Best Rust crate for read/write/overlay on existing PDFs. |
| 6 | 2026-02-24 | draft → submitted for v1 (no approval states) | Approval workflows are v2 complexity. |
| 7 | 2026-02-24 | Optimistic locking for annotation autosave | Prevents silent data loss from concurrent edits. |
| 8 | 2026-02-24 | Magic bytes + size limit on upload | Security: prevents non-PDF uploads and PDF bombs. |
| 9 | 2026-02-24 | Split annotation store from annotation render (separate beads) | Grok review: rendering with pdfium-render is complex enough to warrant its own bead. |
| 10 | 2026-02-24 | Split frontend migration into client layer + store migration | Grok review: touching all Zustand stores in one bead is too risky. |
| 11 | 2026-02-24 | CORS middleware in scaffold | Required for standalone mode where frontend runs on different port/origin. |

---

## Open Questions

1. **pdfium-render annotation type coverage:** Which annotation types (stamps, arrows, shapes, signatures, highlights) can pdfium-render overlay reliably? Need to test each type early in bd-1647. Fallback to lopdf for unsupported types.

2. **Font embedding for form generation:** When rendering text onto PDFs from form data, which fonts are available? Need to bundle a base font set or rely on PDF's embedded fonts.

3. **Frontend test strategy:** The existing frontend has minimal tests. Should bd-37tz (smoke tests) use Playwright, or is a simpler approach sufficient?

4. **MinIO in infrastructure compose:** MinIO doesn't exist in docker-compose.infrastructure.yml today. bd-1fwv will add it — need to ensure it doesn't conflict with any other module's S3 usage.

5. **Signature handling:** Signatures in the annotation system are likely Base64 PNG images. How do we overlay PNG onto PDF via pdfium-render? This is the riskiest annotation type.

---

## Bead Plan

| Bead ID | Title | Depends On | Status |
|---------|-------|-----------|--------|
| bd-2o6u | Scaffold + CORS | — | In progress (SageDesert) |
| bd-1fwv | S3 storage abstraction | bd-2o6u | Open |
| bd-7r4l | DB schema v1 | bd-2o6u | Open |
| bd-2218 | Documents API + validation | bd-1fwv, bd-7r4l | Open |
| bd-3i7a | Annotations store + optimistic locking | bd-2218 | Open |
| bd-1647 | Render annotations (pdfium-render) | bd-3i7a, bd-1fwv | Open |
| bd-2hrz | Forms template/field CRUD | bd-7r4l, bd-2218 | Open |
| bd-3q6j | Form submissions + validation | bd-2hrz | Open |
| bd-1j5x | PDF generation from submissions | bd-3q6j, bd-1fwv | Open |
| bd-34k2 | Frontend API client + types | bd-1j5x, bd-3i7a, bd-3q6j | Open |
| bd-37tz | Store migration + standalone wiring | bd-34k2 | Open |
| bd-1pdk | OpenAPI contract | bd-1j5x, bd-3i7a, bd-1647, bd-3q6j | Open |

---

## ChatGPT Planning Conversation

https://chatgpt.com/g/g-p-698c7e2090308191ba6e6eac93e3cc59-rust-postgres-modules/c/6999f1f7-4014-8325-9176-82016f9594d3

## Existing Frontend

Location: `/Users/james/Projects/PDF-Creation/`
Project name: Fireproof PDF Editor
Stack: React + Vite + TypeScript + Zustand
Stores: uiStore, annotationStore, formStore, toolbarStore
Backend being replaced: Node.js + Express + Prisma + MySQL 8.0
